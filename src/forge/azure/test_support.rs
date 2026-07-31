//! A minimal, dependency-free HTTP/1.1 mock server for unit-testing
//! `AzureDevOpsBackend`'s wiring (path/query construction, continuation
//! pagination, HTTP status handling) without a live Azure DevOps
//! organization — which this task's constraints forbid entirely (no live
//! sandbox is approved; see `docs/follow-on-map/tickets/
//! 04-provision-provider-sandboxes.md`).
//!
//! Extends `crate::forge::giteafj::test_support`'s design (same
//! `TcpListener`-based approach, chosen for the same reason: a handful of
//! canned JSON responses do not justify a new mocking dependency) with two
//! capabilities Gitea's GET-only harness does not need:
//! - **method + request body capture**, so tests can assert a `POST`/
//!   `PATCH`/`PUT` call sent the exact JSON shape expected (thread
//!   creation, thread status update, vote casting);
//! - **per-response headers**, so tests can exercise the
//!   `x-ms-continuationtoken` pagination header.
//!
//! Each registered response is keyed by `"{METHOD} {path}"` (e.g.
//! `"POST /contoso/.../threads?api-version=7.1"`) rather than by path
//! alone, since Azure DevOps' thread-creation and reply flows both `POST`
//! to path shapes that can otherwise collide with a `GET` on the same
//! path.

#![cfg(test)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// One canned response: status, headers, and body.
#[derive(Clone)]
pub(crate) struct MockResponse {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: String,
}

impl MockResponse {
    pub fn json(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    pub fn with_header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

/// One captured request: method, request-target (path+query verbatim),
/// headers (lowercased names, e.g. `"authorization"`, so lookups don't
/// depend on the client's exact casing), and body (empty for `GET`/
/// `DELETE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapturedRequest {
    pub method: String,
    pub target: String,
    pub headers: HashMap<String, String>,
    pub body: String,
}

/// Starts a mock server that serves exactly `responses.len()` requests
/// (each registered `"{METHOD} {path}"` key served at most once, in
/// whatever order the client happens to request them) then shuts down.
/// Returns the `http://host:port` base URL plus a shared, thread-safe log
/// of every request the server actually received, in arrival order.
pub(crate) fn start_mock_server(
    responses: HashMap<String, MockResponse>,
) -> (String, Arc<Mutex<Vec<CapturedRequest>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock listener");
    let addr = listener.local_addr().expect("mock listener local addr");
    let total = responses.len();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let captured_for_thread = Arc::clone(&captured);
    std::thread::spawn(move || {
        for _ in 0..total {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            handle_connection(stream, &responses, &captured_for_thread);
        }
    });
    (format!("http://{addr}"), captured)
}

struct RawRequest {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    content_length: usize,
}

fn read_request_head(reader: &mut BufReader<std::net::TcpStream>) -> Option<RawRequest> {
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return None;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();

    let mut headers = HashMap::new();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) if line == "\r\n" || line == "\n" => break,
            Ok(_) => {
                let trimmed = line.trim_end_matches(['\r', '\n']);
                if let Some((name, value)) = trimmed.split_once(':') {
                    let name = name.trim().to_ascii_lowercase();
                    let value = value.trim().to_string();
                    if name == "content-length" {
                        content_length = value.parse().unwrap_or(0);
                    }
                    headers.insert(name, value);
                }
            }
            Err(_) => break,
        }
    }
    Some(RawRequest {
        method,
        target,
        headers,
        content_length,
    })
}

fn handle_connection(
    stream: std::net::TcpStream,
    responses: &HashMap<String, MockResponse>,
    captured: &Mutex<Vec<CapturedRequest>>,
) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone mock stream"));
    let Some(head) = read_request_head(&mut reader) else {
        return;
    };
    let mut body = vec![0u8; head.content_length];
    if head.content_length > 0 {
        let _ = reader.read_exact(&mut body);
    }
    let body_string = String::from_utf8_lossy(&body).to_string();

    let key = format!("{} {}", head.method, head.target);
    captured
        .lock()
        .expect("lock captured requests")
        .push(CapturedRequest {
            method: head.method,
            target: head.target,
            headers: head.headers,
            body: body_string,
        });

    let mut stream = stream;
    match responses.get(&key) {
        Some(mock) => {
            let reason = if (200..300).contains(&mock.status) {
                "OK"
            } else {
                "Error"
            };
            let mut header_lines = String::new();
            for (name, value) in &mock.headers {
                header_lines.push_str(&format!("{name}: {value}\r\n"));
            }
            let response = format!(
                "HTTP/1.1 {} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{header_lines}Connection: close\r\n\r\n{}",
                mock.status,
                mock.body.len(),
                mock.body
            );
            let _ = stream.write_all(response.as_bytes());
        }
        None => {
            let body = format!("no mock response registered for {key}");
            let response = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    }
}
