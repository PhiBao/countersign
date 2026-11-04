//! Small HTTP surface for the checkout pricing rules.
//!
//! Run with `cargo run -p orders-demo`, then:
//!
//!     curl 'http://127.0.0.1:4080/quote?subtotal_cents=1999&discount_pct=50'
//!
//! The service is deliberately small: the bug lives in the pricing rule, not
//! in the transport, and the same rule serves charges and refunds.

use axum::extract::Query;
use axum::routing::get;
use axum::{Json, Router};
use orders_demo::apply_discount_cents;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct QuoteQuery {
    subtotal_cents: i64,
    discount_pct: u8,
}

#[derive(Debug, Serialize)]
struct Quote {
    subtotal_cents: i64,
    discount_pct: u8,
    total_cents: i64,
}

async fn quote(Query(q): Query<QuoteQuery>) -> Json<Quote> {
    Json(Quote {
        subtotal_cents: q.subtotal_cents,
        discount_pct: q.discount_pct,
        total_cents: apply_discount_cents(q.subtotal_cents, q.discount_pct),
    })
}

#[tokio::main]
async fn main() {
    let app = Router::new().route("/quote", get(quote));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:4080")
        .await
        .expect("bind 127.0.0.1:4080");
    println!("orders demo listening on http://127.0.0.1:4080");
    axum::serve(listener, app).await.expect("serve");
}
