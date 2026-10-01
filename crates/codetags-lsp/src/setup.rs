//! `codetags lsp setup`: the only writer of lspmux's config file (PLAN.md
//! D20). It never runs implicitly. It shows the difference from the file
//! already there, writes only once confirmed, and keeps the old file as a
//! timestamped backup beside it.

use std::path::{Path, PathBuf};

use crate::lspmux::{self, Address};

/// What setup would write, and where.
#[derive(Debug, Clone)]
pub struct Plan {
    /// lspmux's config file.
    pub path: PathBuf,
    /// The file's current text, if it exists.
    pub old: Option<String>,
    /// The text setup writes.
    pub new: String,
    /// The listen address in `new`.
    pub listen: Address,
}

impl Plan {
    /// Plans a config listening on `listen` (default: [`lspmux::default_listen`])
    /// with the idle timeout `instance_timeout`.
    pub fn new(listen: Option<Address>, instance_timeout: u32) -> Result<Self, String> {
        let listen = match listen {
            Some(listen) => listen,
            None => lspmux::default_listen()?,
        };
        lspmux::validate_listen(&listen)?;
        let (path, old) = lspmux::read_config()?;
        let new = lspmux::render(&listen, instance_timeout);
        Ok(Self {
            path,
            old,
            new,
            listen,
        })
    }

    /// Whether the file already holds exactly what setup writes.
    pub fn is_current(&self) -> bool {
        self.old.as_deref() == Some(self.new.as_str())
    }

    /// The change as a line diff: `---`/`+++` headers, then every line
    /// prefixed with ` `, `-` or `+`.
    pub fn diff(&self) -> String {
        let old = self.old.as_deref().unwrap_or("");
        let mut out = format!(
            "--- {} ({})\n+++ {} (codetags lsp setup)\n",
            self.path.display(),
            if self.old.is_some() {
                "current"
            } else {
                "missing"
            },
            self.path.display()
        );
        for (sign, line) in line_diff(old, &self.new) {
            out.push(sign);
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    /// Writes the file: creates its directory and, for a socket, the
    /// socket's private directory; renames any different existing file to a
    /// backup, which it returns; then writes the new text through a
    /// temporary file and a rename.
    pub fn apply(&self) -> Result<Option<PathBuf>, String> {
        if let Address::Unix(socket) = &self.listen
            && let Some(dir) = socket.parent()
        {
            lspmux::ensure_private_dir(dir)?;
        }
        let dir = self
            .path
            .parent()
            .ok_or_else(|| format!("{} has no directory", self.path.display()))?;
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
        let backup = match &self.old {
            Some(old) if old != &self.new => Some(backup(&self.path)?),
            _ => None,
        };
        let temporary = self.path.with_extension("toml.codetags-new");
        std::fs::write(&temporary, &self.new)
            .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
        std::fs::rename(&temporary, &self.path)
            .map_err(|error| format!("cannot replace {}: {error}", self.path.display()))?;
        Ok(backup)
    }
}

/// Copies `path` to `<path>.bak-<UTC time>` (with `-N` added if that exists).
fn backup(path: &Path) -> Result<PathBuf, String> {
    let stamp = crate::logpath::utc_stamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_secs())
            .unwrap_or(0),
    );
    let base = format!("{}.bak-{stamp}", path.display());
    let mut target = PathBuf::from(&base);
    let mut n = 1;
    while target.exists() {
        n += 1;
        target = PathBuf::from(format!("{base}-{n}"));
    }
    std::fs::copy(path, &target).map_err(|error| {
        format!(
            "cannot back up {} to {}: {error}",
            path.display(),
            target.display()
        )
    })?;
    Ok(target)
}

/// The backups [`Plan::apply`] made of `path`.
pub fn backups(path: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Vec::new();
    };
    let prefix = format!("{}.bak-", name.to_string_lossy());
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .map(|entry| entry.path())
        .collect();
    found.sort();
    found
}

/// A minimal line diff (longest common subsequence): each line of `old`
/// and `new` once, in order, marked ` ` (both), `-` (old only) or `+`
/// (new only). Config files are a few lines, so quadratic time is fine.
fn line_diff<'a>(old: &'a str, new: &'a str) -> Vec<(char, &'a str)> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let mut common = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            common[i][j] = if a[i] == b[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            out.push((' ', a[i]));
            i += 1;
            j += 1;
        } else if i < a.len() && (j == b.len() || common[i + 1][j] >= common[i][j + 1]) {
            out.push(('-', a[i]));
            i += 1;
        } else {
            out.push(('+', b[j]));
            j += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::line_diff;

    #[test]
    fn diff_keeps_common_lines_and_marks_changes() {
        let diff = line_diff("a\nb\nc\n", "a\nx\nc\nd\n");
        assert_eq!(
            diff,
            vec![(' ', "a"), ('-', "b"), ('+', "x"), (' ', "c"), ('+', "d")]
        );
    }

    #[test]
    fn diff_from_nothing_adds_everything() {
        assert_eq!(line_diff("", "a\nb"), vec![('+', "a"), ('+', "b")]);
    }
}
