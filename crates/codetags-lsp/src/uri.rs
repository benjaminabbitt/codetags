//! The root's two spellings, and rewriting URIs between them (PLAN.md D26).
//!
//! The server is given the canonical project root. Each shim rewrites its
//! own client's spelling of that root to the canonical one in every message
//! to the server, and back in every message to the client, so documents'
//! URIs and the server's own file watcher agree (V132: on macOS, a root
//! reached through a symlink otherwise never sees a change on disk).
//!
//! A URI is rewritten when it is a `file:` URI under the root: any JSON
//! string, or object key (a `WorkspaceEdit`'s `changes` is keyed by URI),
//! anywhere under a message's `params` or `result`. Under the root means its
//! first path segments equal the root's, compared percent-decoded and with a
//! Windows drive letter in either case (`c%3A`, `C:`). The rest of the URI
//! keeps its own spelling. URIs outside the root, such as the standard
//! library's or `~/.cargo`'s sources, are left alone.

use std::path::Path;

use serde_json::Value;

/// A root URI, parsed for matching.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Root {
    /// The URI without a trailing `/`, as written: what replaces a match.
    prefix: String,
    /// The authority, lower case (empty for a local path).
    authority: String,
    /// Path segments, decoded and normalized ([`segment_key`]).
    segments: Vec<String>,
}

/// A parsed `file:` URI: its authority and its raw path segments.
struct FileUri<'a> {
    authority: &'a str,
    /// The path's segments as written (no leading empty segment).
    segments: Vec<&'a str>,
    /// A `?query` or `#fragment`, kept as written.
    tail: &'a str,
}

fn parse(uri: &str) -> Option<FileUri<'_>> {
    let scheme = uri.get(..7)?;
    if !scheme.eq_ignore_ascii_case("file://") {
        return None;
    }
    let rest = &uri[7..];
    let slash = rest.find('/')?;
    let authority = &rest[..slash];
    let path_and_tail = &rest[slash + 1..];
    let tail_at = path_and_tail
        .find(['?', '#'])
        .unwrap_or(path_and_tail.len());
    let (path, tail) = path_and_tail.split_at(tail_at);
    let segments = if path.is_empty() {
        Vec::new()
    } else {
        path.split('/').collect()
    };
    Some(FileUri {
        authority,
        segments,
        tail,
    })
}

/// A segment for comparison: percent-decoded, and a drive letter (`c:`)
/// in lower case.
fn segment_key(segment: &str, first: bool) -> String {
    let decoded = percent_decode(segment).unwrap_or_else(|| segment.to_string());
    let bytes = decoded.as_bytes();
    if first && bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        decoded.to_ascii_lowercase()
    } else {
        decoded
    }
}

fn authority_key(authority: &str) -> String {
    let lower = authority.to_ascii_lowercase();
    if lower == "localhost" {
        String::new()
    } else {
        lower
    }
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

impl Root {
    fn new(uri: &str) -> Option<Self> {
        let prefix = uri.trim_end_matches('/');
        let parsed = parse(prefix)?;
        if !parsed.tail.is_empty() {
            return None;
        }
        Some(Self {
            prefix: prefix.to_string(),
            authority: authority_key(parsed.authority),
            segments: parsed
                .segments
                .iter()
                .enumerate()
                .map(|(index, segment)| segment_key(segment, index == 0))
                .collect(),
        })
    }

    /// `uri` respelled under `to`, if it is under this root.
    fn respell(&self, uri: &str, to: &Self) -> Option<String> {
        let parsed = parse(uri)?;
        if authority_key(parsed.authority) != self.authority
            || parsed.segments.len() < self.segments.len()
        {
            return None;
        }
        let matches = self
            .segments
            .iter()
            .zip(&parsed.segments)
            .enumerate()
            .all(|(index, (key, segment))| *key == segment_key(segment, index == 0));
        if !matches {
            return None;
        }
        let mut out = to.prefix.clone();
        for segment in &parsed.segments[self.segments.len()..] {
            out.push('/');
            out.push_str(segment);
        }
        out.push_str(parsed.tail);
        Some(out)
    }
}

/// The two spellings of one project root, when they differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    client: Root,
    canonical: Root,
}

impl Rewrite {
    /// The rewrite between `client` (the root URI as the client spells it)
    /// and `canonical`; `None` when they name the root alike, or either is
    /// not a `file:` URI.
    pub fn new(client: &str, canonical: &str) -> Option<Self> {
        let client = Root::new(client)?;
        let canonical = Root::new(canonical)?;
        if client.authority == canonical.authority && client.segments == canonical.segments {
            return None;
        }
        Some(Self { client, canonical })
    }

    /// Respells the client's URIs in `message` canonically, for the server.
    /// Whether anything changed.
    pub fn to_server(&self, message: &mut Value) -> bool {
        rewrite_message(message, &self.client, &self.canonical)
    }

    /// Respells the canonical URIs in `message` as the client spells them.
    /// Whether anything changed.
    pub fn to_client(&self, message: &mut Value) -> bool {
        rewrite_message(message, &self.canonical, &self.client)
    }
}

fn rewrite_message(message: &mut Value, from: &Root, to: &Root) -> bool {
    let mut changed = false;
    for key in ["params", "result"] {
        if let Some(value) = message.get_mut(key) {
            changed |= rewrite_value(value, from, to);
        }
    }
    changed
}

fn rewrite_value(value: &mut Value, from: &Root, to: &Root) -> bool {
    match value {
        Value::String(text) => match from.respell(text, to) {
            Some(respelled) => {
                *text = respelled;
                true
            }
            None => false,
        },
        Value::Array(items) => items.iter_mut().fold(false, |changed, item| {
            rewrite_value(item, from, to) | changed
        }),
        Value::Object(map) => {
            let mut changed = false;
            let renamed: Vec<(String, String)> = map
                .keys()
                .filter_map(|key| from.respell(key, to).map(|new| (key.clone(), new)))
                .collect();
            for (old, new) in renamed {
                if let Some(item) = map.remove(&old) {
                    map.insert(new, item);
                    changed = true;
                }
            }
            for item in map.values_mut() {
                changed |= rewrite_value(item, from, to);
            }
            changed
        }
        _ => false,
    }
}

/// Whether a message body may hold a `file:` URI, so a message without one
/// is passed on as it came.
pub fn may_hold_file_uri(body: &[u8]) -> bool {
    body.windows(5)
        .any(|window| window.eq_ignore_ascii_case(b"file:"))
}

/// The `file:` URI of an absolute path, percent-encoded as editors write
/// it: `file:///home/a%20b`, `file:///C:/Users/x`, `file://server/share/x`.
pub fn path_to_uri(path: &Path) -> String {
    path_text_to_uri(&path.to_string_lossy(), cfg!(windows))
}

/// [`path_to_uri`] on a path's text, read as a Windows path if `windows`.
pub fn path_text_to_uri(text: &str, windows: bool) -> String {
    if !windows {
        return format!("file://{}", encode(text));
    }
    let slashed = text.replace('\\', "/");
    if let Some(unc) = slashed.strip_prefix("//") {
        let (server, rest) = unc.split_at(unc.find('/').unwrap_or(unc.len()));
        return format!("file://{server}{}", encode(rest));
    }
    format!("file:///{}", encode(&slashed))
}

/// Percent-encodes everything but unreserved characters, sub-delimiters,
/// `:`, `@` and `/`.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;

    fn rewrite() -> Rewrite {
        Rewrite::new("file:///home/u/link/proj", "file:///home/u/real/proj").unwrap()
    }

    #[test]
    fn round_trip() {
        let original = json!({"jsonrpc": "2.0", "id": 1, "method": "textDocument/hover",
            "params": {"textDocument": {"uri": "file:///home/u/link/proj/src/lib.rs"}}});
        let mut message = original.clone();
        assert!(rewrite().to_server(&mut message));
        assert_eq!(
            message["params"]["textDocument"]["uri"],
            "file:///home/u/real/proj/src/lib.rs"
        );
        let mut answer = json!({"jsonrpc": "2.0", "id": 1, "result": {"uri": message["params"]["textDocument"]["uri"]}});
        assert!(rewrite().to_client(&mut answer));
        assert_eq!(
            answer["result"]["uri"],
            original["params"]["textDocument"]["uri"]
        );
    }

    #[test]
    fn the_root_itself_and_a_trailing_slash() {
        let mut message =
            json!({"params": ["file:///home/u/link/proj", "file:///home/u/link/proj/"]});
        rewrite().to_server(&mut message);
        assert_eq!(
            message["params"],
            json!(["file:///home/u/real/proj", "file:///home/u/real/proj/"])
        );
    }

    #[test]
    fn outside_the_root_is_left_alone() {
        let outside = json!({"params": {
            "a": "file:///home/u/.cargo/registry/src/x.rs",
            "b": "file:///home/u/link/project2/src/lib.rs",
            "c": "file:///home/u/link",
            "d": "https://example.com/home/u/link/proj",
            "e": "/home/u/link/proj/src/lib.rs",
        }});
        let mut message = outside.clone();
        assert!(!rewrite().to_server(&mut message));
        assert_eq!(message, outside);
    }

    #[test]
    fn only_params_and_result_are_rewritten() {
        let mut message =
            json!({"id": "file:///home/u/link/proj/x", "method": "file:///home/u/link/proj/x"});
        assert!(!rewrite().to_server(&mut message));
    }

    #[test]
    fn object_keys_are_rewritten() {
        let mut edit = json!({"result": {"changes": {"file:///home/u/real/proj/src/a.rs": [1]}}});
        assert!(rewrite().to_client(&mut edit));
        assert_eq!(
            edit["result"]["changes"],
            json!({"file:///home/u/link/proj/src/a.rs": [1]})
        );
    }

    #[test]
    fn percent_encoding_matches_and_the_rest_keeps_its_spelling() {
        let rewrite = Rewrite::new(
            "file:///home/u/my%20link/proj",
            "file:///home/u/real%20dir/proj",
        )
        .unwrap();
        let mut message = json!({"params": {"uri": "file:///home/u/my link/proj/a%20b.rs"}});
        assert!(rewrite.to_server(&mut message));
        assert_eq!(
            message["params"]["uri"],
            "file:///home/u/real%20dir/proj/a%20b.rs"
        );
        let mut back = json!({"result": "file:///home/u/real dir/proj/a%23b.rs#L3"});
        assert!(rewrite.to_client(&mut back));
        assert_eq!(back["result"], "file:///home/u/my%20link/proj/a%23b.rs#L3");
    }

    #[test]
    fn identical_spellings_need_no_rewrite() {
        assert_eq!(
            Rewrite::new("file:///home/u/proj", "file:///home/u/proj/"),
            None
        );
        assert_eq!(
            Rewrite::new("file:///home/u/pr%6Fj", "file:///home/u/proj"),
            None
        );
        assert_eq!(
            Rewrite::new("file:///c%3A/Users/x", "file:///C:/Users/x"),
            None
        );
        assert_eq!(Rewrite::new("file://localhost/a", "file:///a"), None);
        assert_eq!(Rewrite::new("untitled:Untitled-1", "file:///a"), None);
    }

    #[test]
    fn windows_forms() {
        // VS Code's spelling of a short (8.3) path, and the canonical long one.
        let rewrite = Rewrite::new(
            "file:///c%3A/Users/RUNNER~1/proj",
            "file:///C:/Users/runneradmin/proj",
        )
        .unwrap();
        let mut message = json!({"params": {"uri": "file:///C:/Users/RUNNER~1/proj/src/lib.rs"}});
        assert!(rewrite.to_server(&mut message));
        assert_eq!(
            message["params"]["uri"],
            "file:///C:/Users/runneradmin/proj/src/lib.rs"
        );
        let mut back = json!({"result": {"uri": "file:///c%3A/Users/runneradmin/proj/src/lib.rs"}});
        assert!(rewrite.to_client(&mut back));
        assert_eq!(
            back["result"]["uri"],
            "file:///c%3A/Users/RUNNER~1/proj/src/lib.rs"
        );
    }

    #[test]
    fn paths_become_uris() {
        assert_eq!(
            path_text_to_uri("/home/a b/x#1", false),
            "file:///home/a%20b/x%231"
        );
        assert_eq!(
            path_text_to_uri(r"C:\Users\x y\proj", true),
            "file:///C:/Users/x%20y/proj"
        );
        assert_eq!(
            path_text_to_uri(r"\\server\share\proj", true),
            "file://server/share/proj"
        );
    }

    #[test]
    fn a_body_without_file_uris_is_spotted() {
        assert!(may_hold_file_uri(br#"{"uri":"FILE:///a"}"#));
        assert!(!may_hold_file_uri(br#"{"result":null}"#));
    }
}
