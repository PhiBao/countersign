//! A minimal Switch agent client.
//!
//! One WebSocket down (events), HTTP up (operations). The client owns a
//! connection id and reuses it across reconnects, answers the server's
//! application-level `ping` frames with `pong` + cursor, and claims its room
//! with `connect_to_room` so the room's instructions and roles arrive with the
//! claim.

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{error, info, warn};

use crate::config::Config;

pub struct SwitchClient {
    pub base_url: String,
    pub ws_url: String,
    pub agent_id: String,
    pub api_key: String,
    pub connection_id: String,
    http: reqwest::Client,
    cursor: AtomicI64,
    attached: AtomicBool,
    pub room_id: tokio::sync::Mutex<Option<String>>,
}

#[derive(Debug, Clone)]
pub struct IncomingMessage {
    pub room_id: String,
    pub sender: String,
    pub sender_name: String,
    pub body: String,
}

#[async_trait]
pub trait Handler: Send + Sync + 'static {
    /// Called once per successful connection, after `connection_state`.
    async fn on_ready(&self, rooms: Vec<String>) -> Result<()>;
    /// Called for every room message. Only addressed messages are dispatched.
    async fn on_message(&self, message: IncomingMessage) -> Result<()>;
}

impl SwitchClient {
    pub fn new(config: &Config) -> Self {
        let base_url = config.base_url.trim_end_matches('/').to_string();
        let ws_url = base_url.replacen("http", "ws", 1);
        Self {
            base_url,
            ws_url,
            agent_id: config.agent_id.clone(),
            api_key: config.api_key.clone(),
            connection_id: uuid::Uuid::new_v4().to_string(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("build HTTP client"),
            cursor: AtomicI64::new(0),
            attached: AtomicBool::new(false),
            room_id: tokio::sync::Mutex::new(None),
        }
    }

    pub fn cursor(&self) -> i64 {
        self.cursor.load(Ordering::Relaxed)
    }

    pub async fn ops(&self) -> Result<Value> {
        let url = format!("{}/agents/{}/ops", self.base_url, self.agent_id);
        let response = self
            .http
            .get(url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .context("list operations")?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            bail!("list operations failed: {status} {body}");
        }
        Ok(serde_json::from_str(&body)?)
    }

    /// Call an operation. `attached` binds the call to the live connection,
    /// which the room-scoped operations require.
    pub async fn call(&self, operation: &str, args: Value, attached: bool) -> Result<Value> {
        let url = format!("{}/agents/{}/ops/{operation}", self.base_url, self.agent_id);
        let mut request = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&args);
        if attached {
            request = request.header("X-Switch-Connection-Id", &self.connection_id);
        }
        let response = request.send().await.with_context(|| format!("call {operation}"))?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            bail!("operation {operation} failed: {status} {body}");
        }
        let envelope: Value = serde_json::from_str(&body)
            .with_context(|| format!("decode {operation} response: {body}"))?;
        Ok(envelope.get("result").cloned().unwrap_or(Value::Null))
    }

    pub async fn call_attached(&self, operation: &str, args: Value) -> Result<Value> {
        self.call(operation, args, true).await
    }

    pub async fn post_message(&self, room_id: &str, body: &str) -> Result<()> {
        let url = format!("{}/agents/{}/message", self.base_url, self.agent_id);
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&json!({ "room_id": room_id, "content": body }))
            .send()
            .await
            .context("post message")?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            warn!("post_message failed: {status} {text}");
            bail!("post_message failed: {status} {text}");
        }
        Ok(())
    }

    pub async fn connect_room(&self, room_id: &str) -> Result<Value> {
        self.call_attached("connect_to_room", json!({ "room_id": room_id }))
            .await
    }

    pub async fn assume_role(&self, role: &str) -> Result<Value> {
        self.call_attached("assume_role", json!({ "role": role }))
            .await
    }

    pub async fn list_participants(&self) -> Result<Vec<Value>> {
        let value = self.call_attached("list_participants", json!({})).await?;
        Ok(value.as_array().cloned().unwrap_or_default())
    }

    pub async fn list_rooms(&self) -> Result<Vec<Value>> {
        let value = self.call("list_rooms", json!({}), false).await?;
        Ok(value.as_array().cloned().unwrap_or_default())
    }

    /// Run the event loop forever, reconnecting with the same connection id.
    pub async fn run<H: Handler>(self: Arc<Self>, handler: Arc<H>) -> Result<()> {
        let mut delay = 1u64;
        loop {
            match self.clone().run_once(handler.clone()).await {
                Ok(()) => info!("connection closed; reconnecting"),
                Err(error) => error!("connection error: {error:#}; reconnecting"),
            }
            tokio::time::sleep(Duration::from_secs(delay)).await;
            delay = (delay * 2).min(8);
        }
    }

    async fn run_once<H: Handler>(self: Arc<Self>, handler: Arc<H>) -> Result<()> {
        let start = if self.cursor() > 0 {
            format!("start_from={}", self.cursor())
        } else {
            "start_from=head".to_string()
        };
        let url = format!(
            "{}/agents/{}/connection/ws?connection_id={}&scope=all&filter=all&{}&client=countersign&client_version={}",
            self.ws_url,
            self.agent_id,
            self.connection_id,
            start,
            env!("CARGO_PKG_VERSION")
        );
        let mut request = url.into_client_request()?;
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", self.api_key)
                .parse()
                .context("authorization header")?,
        );

        info!("opening event stream");
        let (socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .context("open Switch event stream")?;
        let (mut write, mut read) = socket.split();

        while let Some(frame) = read.next().await {
            let frame = match frame {
                Ok(frame) => frame,
                Err(error) => return Err(error).context("read event stream"),
            };
            let text = match frame {
                WsMessage::Text(text) => text.to_string(),
                WsMessage::Close(_) => return Ok(()),
                WsMessage::Binary(_) | WsMessage::Ping(_) | WsMessage::Pong(_) => continue,
                WsMessage::Frame(_) => continue,
            };
            let event: Value = match serde_json::from_str(&text) {
                Ok(event) => event,
                Err(error) => {
                    warn!("unparseable frame: {error}: {text}");
                    continue;
                }
            };
            match event.get("event").and_then(Value::as_str).unwrap_or("") {
                "ping" => {
                    let pong = json!({ "type": "pong", "cursor": self.cursor() }).to_string();
                    write.send(WsMessage::text(pong)).await?;
                }
                "connection_state" => {
                    let data = event.get("data").cloned().unwrap_or(Value::Null);
                    if let Some(cursor) = data.get("cursor").and_then(Value::as_i64) {
                        self.cursor.store(cursor, Ordering::Relaxed);
                    }
                    self.attached.store(true, Ordering::Relaxed);
                    let mut rooms = data
                        .get("rooms")
                        .and_then(Value::as_array)
                        .map(|rooms| {
                            rooms
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    // An `all`-scope connection covers rooms without claiming
                    // them, so `connection_state.rooms` is empty until the
                    // client claims one. Discover membership explicitly.
                    if rooms.is_empty() {
                        match self.list_rooms().await {
                            Ok(listed) => {
                                rooms = listed
                                    .iter()
                                    .filter_map(|room| {
                                        room.get("room_id").and_then(Value::as_str)
                                    })
                                    .map(str::to_string)
                                    .collect();
                            }
                            Err(error) => warn!("could not list rooms: {error:#}"),
                        }
                    }
                    info!(?rooms, "connection open");
                    if rooms.is_empty() {
                        warn!("agent is not a member of any room yet");
                        continue;
                    }
                    let handler = handler.clone();
                    tokio::spawn(async move {
                        if let Err(error) = handler.on_ready(rooms).await {
                            error!("ready handler failed: {error:#}");
                        }
                    });
                }
                "message" => {
                    if let Some(sequence) = event.get("id").and_then(Value::as_i64) {
                        self.cursor.fetch_max(sequence, Ordering::Relaxed);
                    }
                    let data = event.get("data").cloned().unwrap_or(Value::Null);
                    let payload = data.get("payload").cloned().unwrap_or(Value::Null);
                    let addressed = payload
                        .get("addressed")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let room_label = data
                        .get("room_id")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    let sender_label = payload
                        .get("sender_name")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    let body_label = payload
                        .get("body")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    info!(
                        room = %room_label,
                        sender = %sender_label,
                        addressed,
                        body = %body_label,
                        "room message"
                    );
                    if !addressed {
                        continue;
                    }
                    let message = IncomingMessage {
                        room_id: data
                            .get("room_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        sender: payload
                            .get("sender")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        sender_name: payload
                            .get("sender_name")
                            .and_then(Value::as_str)
                            .unwrap_or("someone")
                            .to_string(),
                        body: payload
                            .get("body")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    };
                    let handler = handler.clone();
                    tokio::spawn(async move {
                        if let Err(error) = handler.on_message(message).await {
                            error!("message handler failed: {error:#}");
                        }
                    });
                }
                "evicted" => {
                    warn!("evicted by another connection: {}", event.get("data").cloned().unwrap_or(serde_json::Value::Null));
                    self.attached.store(false, Ordering::Relaxed);
                    return Ok(());
                }
                "gap" => {
                    warn!("event gap: {}", event.get("data").cloned().unwrap_or(serde_json::Value::Null));
                }
                "refused" => {
                    self.attached.store(false, Ordering::Relaxed);
                    let detail = event.get("data").cloned().unwrap_or(Value::Null);
                    return Err(anyhow!("connection refused: {detail}"));
                }
                other => {
                    info!("frame: {other}");
                }
            }
        }
        self.attached.store(false, Ordering::Relaxed);
        Ok(())
    }
}

/// Strip a leading `@mention` (and any punctuation around it) from a message
/// body so commands read the same whether they were addressed by name, alias
/// or role.
pub fn strip_mention(body: &str) -> String {
    let trimmed = body.trim();
    let without = if let Some(rest) = trimmed.strip_prefix('@') {
        match rest.find(char::is_whitespace) {
            Some(index) => &rest[index..],
            None => "",
        }
    } else {
        trimmed
    };
    without.trim_start_matches([':', ',', ' ']).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::strip_mention;

    #[test]
    fn strips_a_leading_role_mention() {
        assert_eq!(strip_mention("@verifier verify fix/x"), "verify fix/x");
    }

    #[test]
    fn strips_a_mention_with_punctuation() {
        assert_eq!(strip_mention("@fixer: fix the bug"), "fix the bug");
    }

    #[test]
    fn leaves_plain_text_alone() {
        assert_eq!(strip_mention("  ledger  "), "ledger");
    }
}
