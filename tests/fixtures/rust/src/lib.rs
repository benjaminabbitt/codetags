//! Billing: the Rust provider fixture.
//!
//! Each module exercises a case the brief's Rust row cares about (brief §4.4):
//! traits with several implementations, `dyn` calls, a generic impl block,
//! closures and functions used as values, a declarative macro, and local
//! bindings.

#[macro_use]
mod macros;
pub mod charge;
pub mod ledger;
pub mod pipeline;

pub use charge::{Apply, Charge, Discount, Fee};
pub use ledger::Ledger;

/// Runs every adjustment over `total` through the pipeline.
pub fn settle(total: i64) -> i64 {
    let adjustments: Vec<Box<dyn Apply>> = vec![Box::new(Fee(3)), Box::new(Discount(2))];
    let mut ledger = Ledger::default();
    let settled = pipeline::run(total, &adjustments, &mut ledger);
    log_line!(ledger, "settled");
    settled
}
