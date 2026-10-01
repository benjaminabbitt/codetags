//! A declarative macro.

/// Formats a ledger line; expands to a method call on the ledger.
macro_rules! log_line {
    ($ledger:expr, $label:expr) => {
        let _ = $ledger.line($label);
    };
}
