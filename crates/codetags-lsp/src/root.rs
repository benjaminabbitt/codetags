//! Project roots (brief §4.2): the directory a language server is started
//! for, found from the root the client sent, and respelled the way the
//! client spelled it (the P3 draft's review question 5 default: the server's
//! root and the documents' URIs then share one spelling).

use std::path::{Component, Path, PathBuf};

/// How to find the project root from the client's root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootRule {
    /// The outermost Cargo workspace that contains the crate around the
    /// client's root (rust-analyzer).
    Cargo,
    /// The client's root, unchanged. TODO(P3.2): Go (`go.work`/`go.mod`),
    /// TypeScript and Python per brief §4.2.
    Client,
}

impl RootRule {
    /// The rule for a server binary: [`RootRule::Cargo`] for rust-analyzer,
    /// otherwise [`RootRule::Client`].
    pub fn for_server(server: &Path) -> Self {
        let stem = server
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if stem.starts_with("rust-analyzer") {
            Self::Cargo
        } else {
            Self::Client
        }
    }
}

/// The project root for `client_root` under `rule`: `client_root` or one
/// of its ancestors, never canonicalized.
pub fn project_root(client_root: &Path, rule: RootRule) -> PathBuf {
    match rule {
        RootRule::Client => client_root.to_path_buf(),
        RootRule::Cargo => cargo_root(client_root).unwrap_or_else(|| client_root.to_path_buf()),
    }
}

/// The outermost directory, at or above the nearest `Cargo.toml` around
/// `start`, whose `Cargo.toml` declares a `[workspace]` containing that
/// crate; the crate's own directory if none does; `None` if no
/// `Cargo.toml` is at or above `start`.
fn cargo_root(start: &Path) -> Option<PathBuf> {
    let crate_dir = start
        .ancestors()
        .find(|dir| dir.join("Cargo.toml").is_file())?;
    let mut root = crate_dir;
    for dir in crate_dir.ancestors() {
        let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
            continue;
        };
        let Ok(manifest) = text.parse::<toml::Table>() else {
            continue;
        };
        let Some(workspace) = manifest.get("workspace").and_then(toml::Value::as_table) else {
            continue;
        };
        if dir == crate_dir || workspace_contains(workspace, dir, crate_dir) {
            root = dir;
        }
    }
    Some(root.to_path_buf())
}

/// Whether `workspace` (the `[workspace]` table of `dir/Cargo.toml`) lists
/// `crate_dir` in `members` and not in `exclude`. Patterns match path
/// segments with `*` and `?`.
fn workspace_contains(workspace: &toml::Table, dir: &Path, crate_dir: &Path) -> bool {
    let Ok(relative) = crate_dir.strip_prefix(dir) else {
        return false;
    };
    let segments: Vec<String> = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    let listed = |key: &str| {
        workspace
            .get(key)
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(toml::Value::as_str)
            .any(|pattern| pattern_matches(pattern, &segments))
    };
    listed("members") && !listed("exclude")
}

fn pattern_matches(pattern: &str, segments: &[String]) -> bool {
    let parts: Vec<&str> = pattern
        .trim_end_matches('/')
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    parts.len() == segments.len()
        && parts
            .iter()
            .zip(segments)
            .all(|(part, segment)| glob_segment(part.as_bytes(), segment.as_bytes()))
}

/// `*` (any run) and `?` (any one byte) within one path segment.
fn glob_segment(pattern: &[u8], text: &[u8]) -> bool {
    match (pattern.first(), text.first()) {
        (None, None) => true,
        (Some(b'*'), _) => {
            glob_segment(&pattern[1..], text)
                || (!text.is_empty() && glob_segment(pattern, &text[1..]))
        }
        (Some(b'?'), Some(_)) => glob_segment(&pattern[1..], &text[1..]),
        (Some(p), Some(t)) if p == t => glob_segment(&pattern[1..], &text[1..]),
        _ => false,
    }
}

/// The local path a `file:` URI names, percent-decoded; on Windows a
/// leading `/C:` becomes `C:`. `None` for other schemes or a remote host.
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri
        .strip_prefix("file://")
        .or_else(|| uri.strip_prefix("FILE://"))?;
    let path = if let Some(stripped) = rest.strip_prefix("localhost") {
        stripped
    } else {
        rest
    };
    if !path.starts_with('/') {
        return None;
    }
    let decoded = percent_decode(path)?;
    if cfg!(windows) {
        let bytes = decoded.as_bytes();
        if bytes.len() >= 3
            && bytes[0] == b'/'
            && bytes[1].is_ascii_alphabetic()
            && bytes[2] == b':'
        {
            return Some(PathBuf::from(decoded[1..].replace('/', "\\")));
        }
    }
    Some(PathBuf::from(decoded))
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

/// `uri` with its last `levels` path segments removed (a trailing `/` is
/// ignored), so a URI for an ancestor directory keeps the client's spelling.
pub fn uri_parent(uri: &str, levels: usize) -> String {
    let mut text = uri.trim_end_matches('/').to_string();
    for _ in 0..levels {
        match text.rfind('/') {
            Some(slash) if slash > "file://".len() => text.truncate(slash),
            _ => break,
        }
    }
    text
}

/// How many levels `ancestor` is above `path` (0 when they are equal),
/// counting normal components only.
pub fn levels_above(ancestor: &Path, path: &Path) -> usize {
    let count = |p: &Path| {
        p.components()
            .filter(|component| matches!(component, Component::Normal(_)))
            .count()
    };
    count(path).saturating_sub(count(ancestor))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn decodes_file_uris() {
        assert_eq!(
            file_uri_to_path("file:///home/a%20b/x").unwrap(),
            PathBuf::from("/home/a b/x")
        );
        assert!(file_uri_to_path("https://x/y").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn decodes_windows_drive_uris() {
        assert_eq!(
            file_uri_to_path("file:///c%3A/Users/x").unwrap(),
            PathBuf::from("c:\\Users\\x")
        );
    }

    #[test]
    fn uri_parents_keep_the_spelling() {
        assert_eq!(uri_parent("file:///C%3A/a/b/c/", 2), "file:///C%3A/a");
        assert_eq!(uri_parent("file:///a", 0), "file:///a");
        assert_eq!(uri_parent("file:///a", 5), "file:///a");
    }

    #[test]
    fn member_globs_match_whole_segments() {
        let segments = |path: &str| path.split('/').map(String::from).collect::<Vec<_>>();
        assert!(pattern_matches("crates/*", &segments("crates/member")));
        assert!(!pattern_matches("crates/*", &segments("crates/a/b")));
        assert!(pattern_matches("crates/mem?er", &segments("crates/member")));
        assert!(!pattern_matches("tools/*", &segments("crates/member")));
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn cargo_root_is_the_workspace_containing_the_crate() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("project");
        write(
            &root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        );
        write(
            &root.join("crates/member/Cargo.toml"),
            "[package]\nname = \"m\"\n",
        );
        write(
            &root.join("tools/other/Cargo.toml"),
            "[package]\nname = \"o\"\n",
        );
        let member = root.join("crates/member");
        assert_eq!(project_root(&member.join("src"), RootRule::Cargo), root);
        assert_eq!(project_root(&member, RootRule::Cargo), root);
        assert_eq!(project_root(&root, RootRule::Cargo), root);
        // Not a member: its own crate.
        let other = root.join("tools/other");
        assert_eq!(project_root(&other, RootRule::Cargo), other);
        assert_eq!(project_root(&member, RootRule::Client), member);
        assert_eq!(levels_above(&root, &member), 2);
    }

    #[test]
    fn no_manifest_keeps_the_client_root() {
        let scratch = tempfile::tempdir().unwrap();
        let dir = scratch.path().join("plain");
        std::fs::create_dir_all(&dir).unwrap();
        // The scratch directory is inside a temp dir with no Cargo.toml.
        assert_eq!(project_root(&dir, RootRule::Cargo), dir);
    }

    #[test]
    fn rules_follow_the_server_name() {
        assert_eq!(
            RootRule::for_server(Path::new("/x/rust-analyzer")),
            RootRule::Cargo
        );
        assert_eq!(
            RootRule::for_server(Path::new("/x/gopls")),
            RootRule::Client
        );
    }
}
