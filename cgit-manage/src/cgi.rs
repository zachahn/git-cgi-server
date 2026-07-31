//! A small CGI request reader and HTML response writer.
//!
//! The tool reads the request from the environment and standard input, the
//! way a CGI program does. It writes the response to standard output. No
//! external crate parses anything here, so the deploy stays one binary.

use std::collections::HashMap;
use std::io::Read;

/// One parsed CGI request.
pub struct Request {
    pub method: String,
    /// The path after the script, from `PATH_INFO`. Empty at the root.
    pub path_info: String,
    /// The authenticated user the web server passed in `REMOTE_USER`.
    pub remote_user: Option<String>,
    /// Decoded form fields, from the query string on GET or the body on POST.
    pub params: HashMap<String, String>,
}

impl Request {
    /// Read the request from the environment and standard input.
    pub fn read() -> Request {
        let method = std::env::var("REQUEST_METHOD").unwrap_or_else(|_| "GET".into());
        let path_info = std::env::var("PATH_INFO").unwrap_or_default();
        let remote_user = std::env::var("REMOTE_USER").ok().filter(|u| !u.is_empty());

        let raw = if method == "POST" {
            read_body()
        } else {
            std::env::var("QUERY_STRING").unwrap_or_default()
        };

        Request {
            method,
            path_info,
            remote_user,
            params: parse_form(&raw),
        }
    }

    /// Fetch a trimmed form field, or the empty string when absent.
    pub fn field(&self, key: &str) -> &str {
        self.params.get(key).map(|s| s.trim()).unwrap_or("")
    }
}

/// Read the request body, bounded by `CONTENT_LENGTH`.
///
/// The bound stops an oversized body from filling memory. A repo-admin form
/// carries a name and a short description, so a small cap is plenty.
fn read_body() -> String {
    const MAX_BODY: usize = 64 * 1024;

    let declared = std::env::var("CONTENT_LENGTH")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);

    let limit = declared.min(MAX_BODY);
    let mut buf = Vec::with_capacity(limit);
    std::io::stdin()
        .take(limit as u64)
        .read_to_end(&mut buf)
        .ok();
    String::from_utf8_lossy(&buf).into_owned()
}

/// Parse `application/x-www-form-urlencoded` text into fields.
fn parse_form(raw: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        out.insert(url_decode(k), url_decode(v));
    }
    out
}

/// Decode one url-encoded field. `+` becomes a space, `%XX` becomes a byte.
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_val(bytes[i + 1]);
                let lo = hex_val(bytes[i + 2]);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push(h << 4 | l);
                        i += 3;
                    }
                    _ => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Escape text for safe inclusion in HTML.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Print a full CGI response: status line, headers, blank line, HTML body.
pub fn respond(status: u16, reason: &str, body: &str) {
    print!("Status: {status} {reason}\r\n");
    print!("Content-Type: text/html; charset=utf-8\r\n");
    print!("\r\n");
    print!("{body}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_plus_and_percent() {
        assert_eq!(url_decode("a+b"), "a b");
        assert_eq!(url_decode("a%2Fb"), "a/b");
        assert_eq!(url_decode("hi%21"), "hi!");
    }

    #[test]
    fn keeps_stray_percent() {
        assert_eq!(url_decode("100%"), "100%");
        assert_eq!(url_decode("a%zzb"), "a%zzb");
    }

    #[test]
    fn parses_form_pairs() {
        let f = parse_form("name=my-repo&desc=hello+world");
        assert_eq!(f.get("name").unwrap(), "my-repo");
        assert_eq!(f.get("desc").unwrap(), "hello world");
    }

    #[test]
    fn escapes_html_specials() {
        assert_eq!(escape_html("<a href=\"x\">&'"), "&lt;a href=&quot;x&quot;&gt;&amp;&#39;");
    }
}
