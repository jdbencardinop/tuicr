//! Percent-encoding helpers for the dynamic path/query pieces this
//! transport interpolates into request URLs (owner/repo names, file
//! paths, refs/SHAs).
//!
//! `GfHttpClient::url` (`src/forge/giteafj/client.rs`) builds the final
//! request URL with a plain string `format!`, and `ureq` in turn treats the
//! whole thing as a URL to parse: an unescaped `#` truncates everything
//! after it as a fragment, an unescaped `?` starts a bogus query string, a
//! literal space breaks the request line, and a literal `%` is ambiguous
//! with an existing percent-escape. Every caller that splices repository
//! names, file paths, or ref/SHA strings into a path or query value must
//! therefore encode that dynamic piece first — this module is the single
//! place that logic lives so every call site in `backend.rs` uses the same
//! rules.
//!
//! Built on the `percent-encoding` crate (already resolved transitively at
//! this exact version via `ureq`'s own dependency tree; see `Cargo.lock`),
//! per the [WHATWG URL percent-encode sets](https://url.spec.whatwg.org/#percent-encoded-bytes),
//! which this module's `AsciiSet`s mirror (path/path-segment and query
//! sets, minimally extended for query values that are hand-assembled with
//! `format!` rather than through a real query encoder).

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

/// Characters that must be escaped within a single path *segment* — i.e.
/// one slash-delimited component of a path, never the `/` separators
/// between segments themselves (those are preserved by the caller, which
/// splits on `/` before encoding each piece and rejoins with a literal
/// `/`). Mirrors the WHATWG "path percent-encode set" plus `/` and `%`
/// (a literal `/` or `%` *inside* one segment's data, as opposed to a
/// structural separator, must be escaped so it can never be mistaken for
/// one).
///
/// `percent_encoding::utf8_percent_encode` always additionally escapes
/// every non-ASCII (>= 0x80) byte regardless of this set's contents, so
/// unicode filenames are handled without needing to enumerate them here.
const PATH_SEGMENT_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'%')
    .add(b'/');

/// Characters that must be escaped within a hand-assembled query-string
/// *value* (this transport builds query strings with plain `format!`
/// rather than a real query encoder, so `&`/`=`/`+` need escaping here too
/// — a real query encoder would handle those structurally instead).
const QUERY_VALUE_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'%')
    .add(b'&')
    .add(b'=')
    .add(b'+');

/// Percent-encode one path segment (an owner/repo name, or one component
/// of a file path) so it can never be misread as a path separator, query
/// start, or fragment start by the URL parser `ureq` uses internally.
pub fn encode_path_segment(segment: &str) -> String {
    utf8_percent_encode(segment, PATH_SEGMENT_ENCODE_SET).to_string()
}

/// Percent-encode a full slash-delimited path (e.g. a file path within a
/// repository) segment-by-segment, preserving the `/` separators between
/// segments while escaping any other reserved character within each
/// segment's own data. Empty segments (leading/trailing/doubled `/`) are
/// preserved as empty strings so the overall structure round-trips.
pub fn encode_path_segments(path: &str) -> String {
    path.split('/')
        .map(encode_path_segment)
        .collect::<Vec<_>>()
        .join("/")
}

/// Percent-encode a value for use as a `key=value` query-string value in a
/// hand-assembled (non-form-urlencoded) query string, such as `?ref=...`.
pub fn encode_query_value(value: &str) -> String {
    utf8_percent_encode(value, QUERY_VALUE_ENCODE_SET).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_leave_unreserved_characters_untouched() {
        assert_eq!(encode_path_segment("my-repo_1.0"), "my-repo_1.0");
    }

    #[test]
    fn should_encode_space_in_path_segment() {
        assert_eq!(encode_path_segment("my file.txt"), "my%20file.txt");
    }

    #[test]
    fn should_encode_hash_in_path_segment() {
        assert_eq!(encode_path_segment("notes#1.md"), "notes%231.md");
    }

    #[test]
    fn should_encode_question_mark_in_path_segment() {
        assert_eq!(encode_path_segment("what?.rs"), "what%3F.rs");
    }

    #[test]
    fn should_encode_literal_percent_in_path_segment() {
        assert_eq!(encode_path_segment("100%.txt"), "100%25.txt");
    }

    #[test]
    fn should_encode_unicode_in_path_segment() {
        assert_eq!(encode_path_segment("café.rs"), "caf%C3%A9.rs");
    }

    #[test]
    fn should_encode_literal_slash_within_one_segment() {
        // A literal `/` handed to `encode_path_segment` directly (as
        // opposed to `encode_path_segments`, which treats `/` as a
        // structural separator) must still be escaped: this is what
        // protects a data value containing a slash from being
        // reinterpreted as a path separator.
        assert_eq!(encode_path_segment("a/b"), "a%2Fb");
    }

    #[test]
    fn should_preserve_slashes_as_separators_across_segments() {
        assert_eq!(
            encode_path_segments("dir with space/notes#1.md"),
            "dir%20with%20space/notes%231.md"
        );
    }

    #[test]
    fn should_preserve_structure_for_plain_ascii_path() {
        assert_eq!(
            encode_path_segments("src/forge/giteafj/backend.rs"),
            "src/forge/giteafj/backend.rs"
        );
    }

    #[test]
    fn should_encode_ampersand_and_equals_in_query_value() {
        assert_eq!(encode_query_value("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn should_encode_space_and_hash_in_query_value() {
        assert_eq!(
            encode_query_value("feature branch#1"),
            "feature%20branch%231"
        );
    }

    #[test]
    fn should_encode_unicode_in_query_value() {
        assert_eq!(encode_query_value("função"), "fun%C3%A7%C3%A3o");
    }
}
