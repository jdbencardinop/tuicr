//! A minimal, dependency-free HTTP/1.1 mock server for unit-testing
//! `GiteaForgejoBackend`'s wiring (version gating, pagination) without a
//! live Docker instance. Complements — does not replace — the
//! `live_tests.rs` evidence gathered against real Gitea 1.24/Forgejo 16
//! containers: this only proves *this crate's* request/response handling
//! is wired correctly (right path, right query params, right gating
//! order), not that a real server actually behaves this way.
//!
//! Deliberately hand-rolled over `std::net::TcpListener` rather than
//! adding a new mocking dependency (`mockito`/`wiremock`) for a handful of
//! GET-only canned responses.

#![cfg(test)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// Starts a mock server that serves exactly `responses.len()` requests (one
/// per registered path, each served at most once, in the order the client
/// happens to request them) then shuts down. Returns the `http://host:port`
/// base URL, plus a shared, thread-safe log of every request-target (the
/// path and query string, verbatim off the request line, in arrival order)
/// the server actually received — asserting on this log (rather than only
/// on whether the overall call succeeded) is what proves a client sent the
/// exact URL expected, catching a percent-encoding regression even in
/// cases where a lenient/matching mock would otherwise let the call
/// through. The server thread is detached; tests are expected to make
/// exactly as many requests as there are registered responses before the
/// test function returns (any excess connection attempt after the server
/// has served its full registered set will fail to connect, which is fine
/// — the test that provoked it is asserting on an error path anyway).
pub(crate) fn start_mock_server(
    responses: HashMap<String, (u16, String)>,
) -> (String, Arc<Mutex<Vec<String>>>) {
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

/// Read (and discard) one HTTP request's headers off `stream`, returning
/// its request-target (the path+query portion of the request line)
/// verbatim, exactly as the client sent it.
fn read_request_target(stream: &std::net::TcpStream) -> Option<String> {
    let mut reader = BufReader::new(stream.try_clone().expect("clone mock stream"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return None;
    }
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) if line == "\r\n" || line == "\n" => break,
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    request_line
        .split_whitespace()
        .nth(1)
        .map(|s| s.to_string())
}

fn handle_connection(
    stream: std::net::TcpStream,
    responses: &HashMap<String, (u16, String)>,
    captured: &Mutex<Vec<String>>,
) {
    let Some(path) = read_request_target(&stream) else {
        return;
    };
    captured
        .lock()
        .expect("lock captured requests")
        .push(path.clone());

    let mut stream = stream;
    match responses.get(&path) {
        Some((status, body)) => {
            let reason = if *status == 200 { "OK" } else { "Error" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
        None => {
            let body = format!("no mock response registered for path {path}");
            let response = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    }
}
