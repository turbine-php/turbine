//! In-process WebSocket hub and broadcast channels.
//!
//! Turbine's minimal real-time primitive.  Each named channel is a
//! `tokio::sync::broadcast` sender; clients that upgrade via
//! `/_/ws/{channel}` subscribe to it and receive every payload that is
//! later published, either from Rust, from HTTP (`POST /_/ws/publish`),
//! or from PHP (`turbine_ws_publish()`).
//!
//! # Non-goals
//!
//! - Not an application-layer protocol (rooms, presence, ACLs, history).
//!   Build those in PHP on top of the broadcast primitive.
//! - Not a persistent pub/sub.  Channels exist only for the lifetime of
//!   the process; messages published while no subscribers are attached
//!   are dropped.
//! - Not a Redis Pub/Sub replacement across nodes.  Single-process only.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use http_body_util::Full;
use hyper::header::{HeaderValue, CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, UPGRADE};
use hyper::upgrade::Upgraded;
use hyper::{body::Incoming, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use sha1::{Digest, Sha1};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::protocol::{Message, WebSocketConfig};
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, warn};

use base64::Engine as _;

/// Per-channel broadcast capacity.  Small on purpose: WS is a "real-time"
/// fan-out, not a durable queue — subscribers that fall behind this many
/// frames are considered dead and dropped.
const DEFAULT_CHANNEL_CAPACITY: usize = 256;

type HyperResponse = Response<Full<Bytes>>;

fn build_response(status: u16, content_type: &str, body: Vec<u8>) -> HyperResponse {
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
        .header("Content-Type", content_type)
        .header("Content-Length", body.len())
        .body(Full::from(Bytes::from(body)))
        .expect("static response")
}

/// Configuration knobs for the WebSocket hub.
#[derive(Debug, Clone, Copy)]
pub struct WsConfig {
    pub max_channels: usize,
    pub channel_capacity: usize,
    /// Reject upgrade requests whose payload size would exceed this (bytes).
    pub max_frame_size: usize,
    /// Close idle connections after this many seconds of no frames.
    /// 0 = disabled.
    pub idle_timeout_secs: u64,
}

impl Default for WsConfig {
    fn default() -> Self {
        WsConfig {
            max_channels: 128,
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
            max_frame_size: 65_536,
            idle_timeout_secs: 300,
        }
    }
}

/// Process-wide WebSocket hub.
pub struct WsHub {
    channels: DashMap<String, broadcast::Sender<Arc<[u8]>>>,
    config: WsConfig,
    published: AtomicU64,
    subscribed: AtomicU64,
    rejected: AtomicU64,
}

impl WsHub {
    pub fn new(config: WsConfig) -> Self {
        Self {
            channels: DashMap::new(),
            config,
            published: AtomicU64::new(0),
            subscribed: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
        }
    }

    fn channel(&self, name: &str) -> Option<broadcast::Sender<Arc<[u8]>>> {
        if let Some(tx) = self.channels.get(name) {
            return Some(tx.clone());
        }
        if self.channels.len() >= self.config.max_channels {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        let (tx, _) = broadcast::channel(self.config.channel_capacity);
        Some(self.channels.entry(name.to_string()).or_insert(tx).clone())
    }

    /// Publish `payload` to every subscriber of `channel`.  Returns the
    /// number of active subscribers that received the frame, or `None`
    /// if the channel cap is hit.
    pub fn publish(&self, channel: &str, payload: Vec<u8>) -> Option<usize> {
        let tx = self.channel(channel)?;
        let data: Arc<[u8]> = Arc::from(payload);
        // `send` returns Err if there are no receivers — that's fine.
        let n = tx.send(data).unwrap_or(0);
        self.published.fetch_add(1, Ordering::Relaxed);
        Some(n)
    }

    fn subscribe(&self, channel: &str) -> Option<broadcast::Receiver<Arc<[u8]>>> {
        let tx = self.channel(channel)?;
        self.subscribed.fetch_add(1, Ordering::Relaxed);
        Some(tx.subscribe())
    }

    /// Current subscriber count for a channel (0 if absent).
    pub fn subscriber_count(&self, channel: &str) -> usize {
        self.channels
            .get(channel)
            .map(|tx| tx.receiver_count())
            .unwrap_or(0)
    }

    pub fn stats(&self) -> WsStats {
        WsStats {
            channels: self.channels.len(),
            published: self.published.load(Ordering::Relaxed),
            subscribed: self.subscribed.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WsStats {
    pub channels: usize,
    pub published: u64,
    pub subscribed: u64,
    pub rejected: u64,
}

/// Return `true` if the request is a valid WebSocket upgrade request.
pub fn is_upgrade(req: &Request<Incoming>) -> bool {
    let headers = req.headers();
    headers
        .get(UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
        && headers
            .get(CONNECTION)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_ascii_lowercase().contains("upgrade"))
            .unwrap_or(false)
}

const WS_MAGIC: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Compute the `Sec-WebSocket-Accept` value from the client's key.
fn compute_accept(key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(WS_MAGIC.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}

/// Perform the WebSocket upgrade handshake and, on success, spawn a
/// task that pumps `broadcast::Receiver` frames to the client until
/// the connection closes.
///
/// The returned `Response` is the 101 Switching Protocols response the
/// caller must send back to the client.  The upgrade itself is driven
/// asynchronously via `hyper::upgrade::on(req)`.
pub fn handle_ws_upgrade(
    hub: Arc<WsHub>,
    req: Request<Incoming>,
    channel: String,
) -> HyperResponse {
    if !is_upgrade(&req) {
        return build_response(400, "text/plain", b"expected WebSocket upgrade".to_vec());
    }
    let key = match req
        .headers()
        .get(SEC_WEBSOCKET_KEY)
        .and_then(|v| v.to_str().ok())
    {
        Some(k) => k.to_owned(),
        None => {
            return build_response(400, "text/plain", b"missing Sec-WebSocket-Key".to_vec());
        }
    };

    // Pre-register subscriber *before* we release the 101 response so a
    // producer that fires immediately after the handshake cannot race.
    let rx = match hub.subscribe(&channel) {
        Some(r) => r,
        None => {
            return build_response(
                507,
                "application/json",
                b"{\"error\":\"ws channel cap reached\"}".to_vec(),
            );
        }
    };

    let cfg = WebSocketConfig {
        max_message_size: Some(hub.config.max_frame_size),
        max_frame_size: Some(hub.config.max_frame_size),
        accept_unmasked_frames: false,
        ..Default::default()
    };
    let idle = hub.config.idle_timeout_secs;

    // The response must be built BEFORE `on(req)` consumes the Request.
    let accept = compute_accept(&key);
    let response = Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(CONNECTION, HeaderValue::from_static("Upgrade"))
        .header(UPGRADE, HeaderValue::from_static("websocket"))
        .header(SEC_WEBSOCKET_ACCEPT, accept)
        .body(Full::new(Bytes::new()))
        .expect("static 101 response");

    // Spawn the actual WS pump in the background.  Hyper drives the
    // upgrade after this handler returns.
    tokio::spawn(async move {
        match hyper::upgrade::on(req).await {
            Ok(upgraded) => {
                let io = TokioIo::new(upgraded);
                let ws = WebSocketStream::from_raw_socket(
                    io,
                    tokio_tungstenite::tungstenite::protocol::Role::Server,
                    Some(cfg),
                )
                .await;
                if let Err(e) = run_ws_session(ws, rx, idle, &channel).await {
                    debug!(error = %e, channel = %channel, "ws session ended with error");
                }
            }
            Err(e) => {
                warn!(error = %e, "websocket upgrade failed");
            }
        }
    });

    response
}

/// Drive a single WebSocket connection.  We forward every broadcast frame
/// to the client as a binary message, respond to pings, and tear down on
/// a close frame or idle timeout.
async fn run_ws_session(
    mut ws: WebSocketStream<TokioIo<Upgraded>>,
    mut rx: broadcast::Receiver<Arc<[u8]>>,
    idle_secs: u64,
    channel: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let idle = if idle_secs == 0 {
        Duration::from_secs(u64::MAX / 4) // effectively off
    } else {
        Duration::from_secs(idle_secs)
    };

    loop {
        tokio::select! {
            // Frames published to the channel — fan them out to the client.
            msg = rx.recv() => match msg {
                Ok(payload) => {
                    ws.send(Message::Binary(payload.as_ref().to_vec())).await?;
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    debug!(channel = %channel, lost = n, "ws subscriber lagging, dropping");
                    // Fast producer: tell the client we dropped frames and close.
                    let _ = ws.send(Message::Close(Some(
                        tokio_tungstenite::tungstenite::protocol::CloseFrame {
                            code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy,
                            reason: "lagging".into(),
                        },
                    ))).await;
                    break;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },

            // Inbound from the client — handle control frames, ignore data
            // (this hub is server-push only).
            inbound = tokio::time::timeout(idle, ws.next()) => match inbound {
                Ok(Some(Ok(Message::Ping(p)))) => { ws.send(Message::Pong(p)).await?; }
                Ok(Some(Ok(Message::Close(_)))) | Ok(None) => break,
                Ok(Some(Ok(_))) => { /* drop data frames silently */ }
                Ok(Some(Err(e))) => return Err(Box::new(e)),
                Err(_) => {
                    // idle timeout
                    let _ = ws.close(None).await;
                    break;
                }
            },
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_key_matches_rfc_example() {
        // Example from RFC 6455 §1.3
        assert_eq!(
            compute_accept("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn publish_to_absent_channel_creates_it() {
        let hub = WsHub::new(WsConfig::default());
        let n = hub.publish("c", b"hello".to_vec()).unwrap();
        // No subscribers yet — 0 receivers.
        assert_eq!(n, 0);
        assert_eq!(hub.stats().channels, 1);
        assert_eq!(hub.stats().published, 1);
    }

    #[test]
    fn publish_rejects_past_channel_cap() {
        let hub = WsHub::new(WsConfig {
            max_channels: 1,
            ..Default::default()
        });
        hub.publish("a", b"x".to_vec()).unwrap();
        assert!(hub.publish("b", b"y".to_vec()).is_none());
        assert_eq!(hub.stats().rejected, 1);
    }

    #[tokio::test]
    async fn subscriber_receives_publish() {
        let hub = WsHub::new(WsConfig::default());
        let mut rx = hub.subscribe("c").unwrap();
        hub.publish("c", b"hello".to_vec()).unwrap();
        let frame = rx.recv().await.unwrap();
        assert_eq!(frame.as_ref(), b"hello");
    }
}
