//! A trait with several implementations, one of them a generic impl block.

/// Something that adjusts an amount.
pub trait Apply {
    /// Returns `amount` adjusted.
    fn apply(&self, amount: i64) -> i64;
}

/// A flat fee added to the amount.
pub struct Fee(pub i64);

/// A flat discount taken off the amount.
pub struct Discount(pub i64);

/// A charge carrying some payload `T`.
pub struct Charge<T> {
    /// The payload.
    pub payload: T,
    /// The amount the charge adds.
    pub amount: i64,
}

impl Apply for Fee {
    fn apply(&self, amount: i64) -> i64 {
        amount + self.0
    }
}

impl Apply for Discount {
    fn apply(&self, amount: i64) -> i64 {
        amount - self.0
    }
}

impl<T> Apply for Charge<T> {
    fn apply(&self, amount: i64) -> i64 {
        amount + self.amount
    }
}

/// Doubles an amount; used as a value in `pipeline`.
pub fn double(amount: i64) -> i64 {
    amount * 2
}
