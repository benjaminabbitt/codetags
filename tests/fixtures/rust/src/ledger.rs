//! A small ledger the pipeline records into.

/// Amounts recorded in order.
#[derive(Debug, Default)]
pub struct Ledger {
    /// Every recorded amount.
    pub entries: Vec<i64>,
}

impl Ledger {
    /// Records `amount`.
    pub fn record(&mut self, amount: i64) {
        self.entries.push(amount);
    }

    /// Writes a labelled line of the latest amount.
    pub fn line(&self, label: &str) -> String {
        let latest = self.entries.last().copied().unwrap_or_default();
        format!("{label}: {latest}")
    }
}

/// A second inherent impl block. Its `Self` gives the block itself a SCIP
/// symbol, `ledger/impl#[Ledger]` (V97), which is a container, not a symbol.
impl Ledger {
    /// The recorded amounts: a getter named like its field.
    pub fn entries(&self) -> &[i64] {
        &self.entries
    }

    /// A ledger holding `entries`.
    pub fn with(entries: Vec<i64>) -> Self {
        Self { entries }
    }
}
