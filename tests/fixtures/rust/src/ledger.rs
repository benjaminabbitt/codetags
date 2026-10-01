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
