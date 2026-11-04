//! Read-only proof surface: the receipt ledger as JSON, plus the built
//! viewer. One process, one address, so a judge can open the evidence without
//! a second toolchain.

use anyhow::{Context, Result};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};

use crate::ledger::{Ledger, Receipt};

#[derive(Clone)]
struct ApiState {
    ledger: Ledger,
}

#[derive(Serialize)]
struct ReceiptView {
    id: String,
    room_id: String,
    branch: String,
    commit_sha: String,
    verdict: String,
    reason: Option<String>,
    changed_files: Vec<String>,
    base_output: String,
    suite_output: String,
    duration_ms: i64,
    created_at: String,
    countersigned_by: Option<String>,
    countersigned_at: Option<String>,
    landed_url: Option<String>,
}

impl From<Receipt> for ReceiptView {
    fn from(receipt: Receipt) -> Self {
        Self {
            id: receipt.id,
            room_id: receipt.room_id,
            branch: receipt.branch,
            commit_sha: receipt.commit_sha,
            verdict: receipt.verdict.as_str().to_string(),
            reason: receipt.reason,
            changed_files: receipt.changed_files,
            base_output: receipt.base_output,
            suite_output: receipt.suite_output,
            duration_ms: receipt.duration_ms,
            created_at: receipt.created_at,
            countersigned_by: receipt.countersigned_by,
            countersigned_at: receipt.countersigned_at,
            landed_url: receipt.landed_url,
        }
    }
}

async fn list(State(state): State<ApiState>) -> impl IntoResponse {
    match state.ledger.recent(100) {
        Ok(receipts) => Json(
            receipts
                .into_iter()
                .map(ReceiptView::from)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("ledger read failed: {error:#}"),
        )
            .into_response(),
    }
}

async fn one(State(state): State<ApiState>, Path(id): Path<String>) -> impl IntoResponse {
    match state.ledger.get(&id) {
        Ok(Some(receipt)) => Json(ReceiptView::from(receipt)).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "no such receipt").into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("ledger read failed: {error:#}"),
        )
            .into_response(),
    }
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

pub async fn serve(ledger: Ledger, port: u16, viewer_dir: PathBuf) -> Result<()> {
    let index = viewer_dir.join("index.html");
    let has_viewer = index.exists();

    let mut app = Router::new()
        .route("/api/receipts", get(list))
        .route("/api/receipts/:id", get(one))
        .route("/api/health", get(health))
        .with_state(ApiState { ledger });

    if has_viewer {
        app = app.fallback_service(
            ServeDir::new(&viewer_dir).not_found_service(ServeFile::new(index)),
        );
    } else {
        app = app.fallback(get(|| async {
            Html(
                "<body style=\"font-family:monospace;padding:2rem;background:#f6f3ec;color:#141414\">\
                 <h1>Countersign</h1>\
                 <p>The API is live at <code>/api/receipts</code>.</p>\
                 <p>Build the viewer with <code>cd viewer &amp;&amp; pnpm install &amp;&amp; pnpm build</code>.</p>\
                 </body>",
            )
        }));
    }

    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("bind {address}"))?;
    tracing::info!("ledger viewer on http://{address}");
    axum::serve(listener, app).await.context("serve ledger viewer")?;
    Ok(())
}

