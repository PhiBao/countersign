//! The regression that started the incident.
//!
//! Finance reported that orders whose discounted total lands on a half cent
//! are charged one cent too little. This test is the smallest reproduction.

use orders_demo::{apply_discount_cents, order_total_cents, LineItem};

fn line(unit_price_cents: i64, quantity: i64) -> LineItem {
    LineItem {
        sku: "SKU-1".into(),
        unit_price_cents,
        quantity,
    }
}

#[test]
fn half_cent_charges_round_up() {
    // $19.99 with 50% off is $9.995, which must charge as $10.00.
    assert_eq!(apply_discount_cents(1999, 50), 1000);
}

#[test]
fn ordinary_order_total_is_exact() {
    // 3 x $12.50 = $37.50, 20% off = $30.00 exactly.
    assert_eq!(order_total_cents(&[line(1250, 3)], 20), 3000);
}

#[test]
fn quantity_and_percentage_compose() {
    // 2 x $49.99 = $99.98, 15% off = $84.983, which must charge as $84.98.
    assert_eq!(order_total_cents(&[line(4999, 2)], 15), 8498);
}
