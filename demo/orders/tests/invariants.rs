//! Invariants that must hold for every amount, in both directions.
//!
//! The checkout service prices charges and refunds with the same function, so
//! a rounding rule that only works for positive amounts is not a fix.

use orders_demo::apply_discount_cents;

/// The business rule, written as integer arithmetic so the reference cannot
/// inherit a floating-point rounding mode: nearest cent, half away from zero.
fn reference(subtotal_cents: i64, discount_pct: u8) -> i64 {
    let numerator = subtotal_cents * (100 - discount_pct as i64);
    if numerator >= 0 {
        (numerator + 50) / 100
    } else {
        (numerator - 50) / 100
    }
}

#[test]
fn refunds_round_symmetrically() {
    // A $19.99 refund with 50% off returns $10.00, not $9.99.
    assert_eq!(apply_discount_cents(-1999, 50), -1000);
}

#[test]
fn every_amount_matches_the_integer_reference() {
    let amounts = [
        -100_000, -99_999, -2_001, -1_999, -101, -99, -1, 0, 1, 99, 101, 1_999, 2_001, 99_999,
        100_000,
    ];
    for amount in amounts {
        for pct in [0u8, 1, 15, 33, 50, 75, 99, 100] {
            assert_eq!(
                apply_discount_cents(amount, pct),
                reference(amount, pct),
                "amount={amount} pct={pct}"
            );
        }
    }
}

#[test]
fn discount_never_moves_the_total_away_from_zero() {
    for amount in [-100_000i64, -1_999, -1, 0, 1, 1_999, 100_000] {
        for pct in [0u8, 25, 50, 100] {
            let total = apply_discount_cents(amount, pct);
            assert!(
                total.abs() <= amount.abs(),
                "discount increased the amount: {amount} at {pct}% -> {total}"
            );
        }
    }
}
