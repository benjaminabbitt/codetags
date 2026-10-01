//! `dyn` calls, closures, and functions used as values.

use crate::charge::{Apply, double};
use crate::ledger::Ledger;

/// Applies every adjustment in turn, recording each step in `ledger`.
pub fn run(total: i64, adjustments: &[Box<dyn Apply>], ledger: &mut Ledger) -> i64 {
    let mut amount = total;
    for adjustment in adjustments {
        amount = apply_dyn(adjustment.as_ref(), amount);
        ledger.record(amount);
    }
    let step: fn(i64) -> i64 = double;
    let rounded = |value: i64| value - value % 10;
    rounded(transform(amount, step))
}

/// A `dyn` call: dispatch through the vtable.
pub fn apply_dyn(adjustment: &dyn Apply, amount: i64) -> i64 {
    adjustment.apply(amount)
}

/// Calls a function it was given as a value.
pub fn transform(amount: i64, step: fn(i64) -> i64) -> i64 {
    step(amount)
}

/// Passes a function as a value without calling it.
pub fn doubled_all(amounts: &[i64]) -> Vec<i64> {
    amounts.iter().copied().map(double).collect()
}
