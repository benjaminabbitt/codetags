//! The proxy's part of `codetags doctor` (PLAN.md D19, D20;
//! docs/proxy-zero-change.md §7): lspmux's revision, whether its config is
//! what `codetags lsp setup` writes, whether lspmux would even parse it, and
//! whether its listen address keeps other users out.

use std::path::Path;
use std::process::Command;

use crate::lspmux::{self, Address};
use crate::tools;

/// One line of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The text to print.
    pub line: String,
    /// Whether this is something the user must fix (doctor exits 1).
    pub needs_fixing: bool,
}

fn ok(line: String) -> Finding {
    Finding {
        line,
        needs_fixing: false,
    }
}

fn fix(line: String) -> Finding {
    Finding {
        line,
        needs_fixing: true,
    }
}

/// The proxy checks. A missing lspmux or config is reported, not an error:
/// the proxy is optional. A config lspmux would ignore, drift from what
/// setup writes, a wrong revision, or an address other users can reach must
/// be fixed.
pub fn report() -> Vec<Finding> {
    let mut findings = Vec::new();
    let binary = tools::find_lspmux();
    match &binary {
        None => findings.push(ok(
            "lspmux: not found (the LSP proxy needs it; install with: just setup-lspmux)".into(),
        )),
        Some(binary) => findings.push(binary_finding(binary)),
    }
    let (path, text) = match lspmux::read_config() {
        Ok(found) => found,
        Err(error) => {
            findings.push(fix(format!("lspmux config: {error}")));
            return findings;
        }
    };
    let Some(text) = text else {
        findings.push(ok(format!(
            "lspmux config: {}: not written (to use the LSP proxy, run: codetags lsp setup)",
            path.display()
        )));
        return findings;
    };
    let effective = match lspmux::check(&text) {
        Ok(effective) => effective,
        Err(reason) => {
            findings.push(fix(format!(
                "lspmux config: {}: lspmux ignores this file and runs on its defaults: {reason} \
                 (rewrite it with: codetags lsp setup)",
                path.display()
            )));
            return findings;
        }
    };
    if text == lspmux::render(&effective.listen, lspmux::DEFAULT_INSTANCE_TIMEOUT) {
        findings.push(ok(format!(
            "lspmux config: {} (as codetags lsp setup writes it)",
            path.display()
        )));
    } else {
        findings.push(fix(format!(
            "lspmux config: {} differs from what codetags lsp setup writes (see the difference, \
             and restore it, with: codetags lsp setup)",
            path.display()
        )));
    }
    findings.extend(address_findings(&effective.listen));
    if effective.connect != effective.listen {
        findings.push(fix(format!(
            "lspmux connect: {} differs from listen {}",
            effective.connect, effective.listen
        )));
    }
    if let Some(binary) = &binary {
        findings.extend(effective_check(binary, &effective.listen));
    }
    findings
}

fn binary_finding(binary: &Path) -> Finding {
    let short = &lspmux::PINNED_REV[..12];
    match tools::installed_lspmux_rev(binary) {
        Some(rev) if rev == lspmux::PINNED_REV => ok(format!(
            "lspmux: {} (rev {short}, the pinned revision)",
            binary.display()
        )),
        Some(rev) => fix(format!(
            "lspmux: {} (rev {}; codetags pins {short}: run just setup-lspmux)",
            binary.display(),
            &rev[..rev.len().min(12)]
        )),
        None => ok(format!(
            "lspmux: {} (revision unknown: not installed by cargo install; codetags pins {short})",
            binary.display()
        )),
    }
}

fn address_findings(listen: &Address) -> Vec<Finding> {
    let mut findings = vec![ok(format!("lspmux listen: {listen}"))];
    match listen {
        Address::Unix(path) => {
            let dir = path.parent().unwrap_or(Path::new("/"));
            if dir.exists() {
                if let Some(reason) = listen.unsafe_reason() {
                    findings.push(fix(format!("problem: {reason}")));
                }
            } else {
                findings.push(ok(format!(
                    "note: {} does not exist yet; codetags-lsp creates it, private, when it starts the daemon",
                    dir.display()
                )));
            }
        }
        Address::Tcp(..) => {
            if let Some(reason) = listen.unsafe_reason() {
                findings.push(fix(format!("problem: {reason}")));
            }
            if let Some(note) = listen.exposure_note() {
                findings.push(ok(format!("note: {note}")));
            }
        }
    }
    findings
}

/// Runs `lspmux config`, which prints the configuration lspmux actually
/// loaded, and checks that it listens where the file says: if not, lspmux
/// reads another file (V116).
fn effective_check(binary: &Path, listen: &Address) -> Option<Finding> {
    let output = Command::new(binary).arg("config").output().ok()?;
    let printed = String::from_utf8_lossy(&output.stdout);
    let expected = match listen {
        Address::Tcp(ip, port) => format!("Tcp(\n        {ip},\n        {port},\n    )"),
        Address::Unix(path) => format!("Unix(\n        {:?},\n    )", path),
    };
    if printed.contains(&format!("listen: {expected}")) {
        None
    } else {
        Some(fix(format!(
            "lspmux config: `{} config` does not listen on {listen}; it reads another file",
            binary.display()
        )))
    }
}
