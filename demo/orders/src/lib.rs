//! Pricing rules for the demo checkout service.
//!
//! Money is stored in integer cents. Discounts are percentages. A negative
//! subtotal represents a refund, and the same rounding rule has to hold in
//! both directions.

/// One line of an order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineItem {
    pub sku: String,
    pub unit_price_cents: i64,
    pub quantity: i64,
}

/// Sum of every line, in cents. Quantities may be negative for returns.
pub fn subtotal_cents(items: &[LineItem]) -> i64 {
    items
        .iter()
        .map(|item| item.unit_price_cents * item.quantity)
        .sum()
}

/// Total after a percentage discount, in cents, rounded to the nearest cent
/// with half cents rounded away from zero (so a refund mirrors its charge).
pub fn apply_discount_cents(subtotal_cents: i64, discount_pct: u8) -> i64 {
    let numerator = i128::from(subtotal_cents) * (100 - i128::from(discount_pct));
    let rounded = if numerator >= 0 {
        (numerator + 50) / 100
    } else {
        (numerator - 50) / 100
    };

    rounded as i64
}

/// The amount the customer is charged (or refunded, when negative).
pub fn order_total_cents(items: &[LineItem], discount_pct: u8) -> i64 {
    apply_discount_cents(subtotal_cents(items), discount_pct)
}

#[cfg(test)]
mod unit {
    use super::*;

    fn item(price: i64, qty: i64) -> LineItem {
        LineItem {
            sku: "sku".into(),
            unit_price_cents: price,
            quantity: qty,
        }
    }

    #[test]
    fn subtotal_multiplies_quantity() {
        assert_eq!(subtotal_cents(&[item(1250, 3), item(400, 1)]), 4150);
    }

    #[test]
    fn zero_discount_keeps_the_subtotal() {
        assert_eq!(apply_discount_cents(1999, 0), 1999);
    }
}
