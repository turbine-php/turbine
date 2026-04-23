//! Admin/internal HTTP endpoints under `/_/`.
//!
//! Handles the shared-table and task-queue APIs (and will host the
//! websocket upgrade later).  Extracted from `main.rs` to keep the
//! request-handler hot path readable.
//!
//! Each handler returns a `HyperResponse`.  Callers pre-filter on path
//! prefix; unknown sub-paths fall through to a 404 from the handler.

use std::net::SocketAddr;
use std::time::Duration;

use hyper::{body::Incoming, Request};

use crate::async_io::AsyncIoError;
use crate::compat::{self, FullHttpRequest};
use crate::http_helpers::HyperResponse;
use crate::shared_table::TableError;
use crate::task_queue::QueueError;
use crate::websocket;
use crate::ServerState;
use crate::{build_response, query_param};

fn json_err(code: u16, body: &str) -> HyperResponse {
    build_response(code, "application/json", body.as_bytes().to_vec(), &[])
}

/// Shared-table endpoints (`/_/table/*`).  Callers must check the path
/// prefix and `state.shared_table.is_some()` before invoking.
pub async fn handle_shared_table(
    state: &ServerState,
    req: Request<Incoming>,
    method: &str,
    clean_path: &str,
    remote_addr: SocketAddr,
) -> HyperResponse {
    let Some(table) = state.shared_table.as_ref() else {
        return build_response(404, "text/plain", b"Not found".to_vec(), &[]);
    };

    let qs = req.uri().query().unwrap_or("");
    let key = query_param(qs, "key");
    let ttl_ms: u64 = query_param(qs, "ttl_ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let delta: i64 = query_param(qs, "delta")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let sub = clean_path.strip_prefix("/_/table").unwrap_or("");

    let resp = match (method, sub) {
        ("GET", "/size") => {
            let body = format!(
                "{{\"size\":{},\"evictions\":{}}}",
                table.size(),
                table.evictions()
            );
            build_response(200, "application/json", body.into_bytes(), &[])
        }
        ("GET", "/exists") => {
            let Some(k) = key else {
                return json_err(400, "{\"error\":\"missing key\"}");
            };
            let code = if table.exists(k) { 200 } else { 404 };
            build_response(
                code,
                "application/json",
                format!("{{\"exists\":{}}}", code == 200).into_bytes(),
                &[],
            )
        }
        ("GET", "" | "/" | "/get") => {
            let Some(k) = key else {
                return json_err(400, "{\"error\":\"missing key\"}");
            };
            match table.get(k) {
                Some(v) => build_response(200, "application/octet-stream", v, &[]),
                None => build_response(404, "text/plain", Vec::new(), &[]),
            }
        }
        ("POST", "/incr") => {
            let Some(k) = key else {
                return json_err(400, "{\"error\":\"missing key\"}");
            };
            match table.incr(k, delta) {
                Ok(v) => build_response(
                    200,
                    "application/json",
                    format!("{{\"value\":{v}}}").into_bytes(),
                    &[],
                ),
                Err(TableError::Full(n)) => build_response(
                    507,
                    "application/json",
                    format!("{{\"error\":\"table full\",\"max_entries\":{n}}}").into_bytes(),
                    &[],
                ),
                Err(TableError::NotACounter(_)) => {
                    json_err(409, "{\"error\":\"key exists but is not a counter\"}")
                }
            }
        }
        ("POST", "" | "/" | "/set") => {
            let Some(k) = key.map(str::to_string) else {
                return json_err(400, "{\"error\":\"missing key\"}");
            };
            let (inner_req, _) = match FullHttpRequest::from_hyper(
                req,
                remote_addr,
                &state.upload_tmp_dir,
                &state.upload_security,
                Some(state.max_body_bytes.unwrap_or(1_048_576).max(1_048_576)),
            )
            .await
            {
                Ok(pair) => pair,
                Err(compat::RequestBuildError::PayloadTooLarge) => {
                    return json_err(413, "{\"error\":\"payload too large\"}");
                }
                Err(_) => {
                    return json_err(400, "{\"error\":\"invalid request\"}");
                }
            };
            let ttl = if ttl_ms > 0 {
                Some(Duration::from_millis(ttl_ms))
            } else {
                None
            };
            match table.set(k, inner_req.body, ttl) {
                Ok(()) => build_response(204, "text/plain", Vec::new(), &[]),
                Err(TableError::Full(n)) => build_response(
                    507,
                    "application/json",
                    format!("{{\"error\":\"table full\",\"max_entries\":{n}}}").into_bytes(),
                    &[],
                ),
                Err(TableError::NotACounter(_)) => json_err(500, "{\"error\":\"internal\"}"),
            }
        }
        ("DELETE", "/clear") => {
            let n = table.clear();
            build_response(
                200,
                "application/json",
                format!("{{\"cleared\":{n}}}").into_bytes(),
                &[],
            )
        }
        ("DELETE", "" | "/" | "/del") => {
            let Some(k) = key else {
                return json_err(400, "{\"error\":\"missing key\"}");
            };
            let removed = table.del(k);
            build_response(
                200,
                "application/json",
                format!("{{\"deleted\":{removed}}}").into_bytes(),
                &[],
            )
        }
        _ => build_response(404, "text/plain", b"Not found".to_vec(), &[]),
    };
    resp
}

/// Task-queue endpoints (`/_/task/*`).  Callers must check the path
/// prefix and `state.task_queue.is_some()` before invoking.
pub async fn handle_task_queue(
    state: &ServerState,
    req: Request<Incoming>,
    method: &str,
    clean_path: &str,
    remote_addr: SocketAddr,
) -> HyperResponse {
    let Some(queue) = state.task_queue.as_ref() else {
        return build_response(404, "text/plain", b"Not found".to_vec(), &[]);
    };

    let qs = req.uri().query().unwrap_or("");
    let channel = query_param(qs, "channel");
    let wait_ms: u64 = query_param(qs, "wait_ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
        .min(state.task_max_wait_ms);
    let sub = clean_path.strip_prefix("/_/task").unwrap_or("");

    let resp = match (method, sub) {
        ("GET", "/stats") => {
            let s = queue.stats();
            let body = format!(
                "{{\"channels\":{},\"pushed\":{},\"popped\":{},\"rejected\":{}}}",
                s.channels, s.pushed, s.popped, s.rejected
            );
            build_response(200, "application/json", body.into_bytes(), &[])
        }
        ("GET", "/size") => {
            let Some(c) = channel else {
                return json_err(400, "{\"error\":\"missing channel\"}");
            };
            build_response(
                200,
                "application/json",
                format!("{{\"size\":{}}}", queue.size(c)).into_bytes(),
                &[],
            )
        }
        ("POST", "" | "/" | "/push") => {
            let Some(c) = channel.map(str::to_string) else {
                return json_err(400, "{\"error\":\"missing channel\"}");
            };
            let (inner_req, _) = match FullHttpRequest::from_hyper(
                req,
                remote_addr,
                &state.upload_tmp_dir,
                &state.upload_security,
                Some(state.max_body_bytes.unwrap_or(1_048_576).max(1_048_576)),
            )
            .await
            {
                Ok(pair) => pair,
                Err(compat::RequestBuildError::PayloadTooLarge) => {
                    return json_err(413, "{\"error\":\"payload too large\"}");
                }
                Err(_) => return json_err(400, "{\"error\":\"invalid request\"}"),
            };
            match queue.push(&c, inner_req.body) {
                Ok(id) => build_response(
                    200,
                    "application/json",
                    format!("{{\"id\":{id}}}").into_bytes(),
                    &[],
                ),
                Err(QueueError::Full(_, n)) => build_response(
                    507,
                    "application/json",
                    format!("{{\"error\":\"channel full\",\"capacity\":{n}}}").into_bytes(),
                    &[],
                ),
                Err(QueueError::TooManyChannels(n)) => build_response(
                    507,
                    "application/json",
                    format!("{{\"error\":\"too many channels\",\"max\":{n}}}").into_bytes(),
                    &[],
                ),
            }
        }
        ("POST", "/pop") | ("GET", "/pop") => {
            let Some(c) = channel.map(str::to_string) else {
                return json_err(400, "{\"error\":\"missing channel\"}");
            };
            let wait = Duration::from_millis(wait_ms);
            match queue.pop(&c, wait).await {
                Some(job) => {
                    let id_str = job.id.to_string();
                    build_response(
                        200,
                        "application/octet-stream",
                        job.payload,
                        &[("X-Task-Id", id_str.as_str())],
                    )
                }
                None => build_response(204, "text/plain", Vec::new(), &[]),
            }
        }
        ("DELETE", "/clear") => {
            let Some(c) = channel else {
                return json_err(400, "{\"error\":\"missing channel\"}");
            };
            let n = queue.clear(c);
            build_response(
                200,
                "application/json",
                format!("{{\"cleared\":{n}}}").into_bytes(),
                &[],
            )
        }
        _ => return build_response(404, "text/plain", b"Not found".to_vec(), &[]),
    };
    resp
}

/// WebSocket endpoints (`/_/ws/*`).  Callers must check the path prefix
/// and `state.ws_hub.is_some()` before invoking.
///
/// Routing:
/// - `POST /_/ws/publish?channel=X`  — publish raw body to subscribers
/// - `GET  /_/ws/stats`              — hub stats
/// - `GET  /_/ws/subscribers?channel=X` — live subscriber count
/// - `GET  /_/ws/{channel}` with `Upgrade: websocket` — subscribe
pub async fn handle_websocket(
    state: &ServerState,
    req: Request<Incoming>,
    method: &str,
    clean_path: &str,
    remote_addr: SocketAddr,
) -> HyperResponse {
    let Some(hub) = state.ws_hub.as_ref().cloned() else {
        return build_response(404, "text/plain", b"Not found".to_vec(), &[]);
    };
    let qs = req.uri().query().unwrap_or("").to_string();
    let channel_q = query_param(&qs, "channel").map(str::to_string);
    let sub = clean_path.strip_prefix("/_/ws").unwrap_or("");

    match (method, sub) {
        ("GET", "/stats") => {
            let s = hub.stats();
            let body = format!(
                "{{\"channels\":{},\"published\":{},\"subscribed\":{},\"rejected\":{}}}",
                s.channels, s.published, s.subscribed, s.rejected
            );
            build_response(200, "application/json", body.into_bytes(), &[])
        }
        ("GET", "/subscribers") => {
            let Some(c) = channel_q else {
                return json_err(400, "{\"error\":\"missing channel\"}");
            };
            build_response(
                200,
                "application/json",
                format!("{{\"subscribers\":{}}}", hub.subscriber_count(&c)).into_bytes(),
                &[],
            )
        }
        ("POST", "/publish") => {
            let Some(c) = channel_q else {
                return json_err(400, "{\"error\":\"missing channel\"}");
            };
            let (inner_req, _) = match FullHttpRequest::from_hyper(
                req,
                remote_addr,
                &state.upload_tmp_dir,
                &state.upload_security,
                Some(state.max_body_bytes.unwrap_or(1_048_576).max(1_048_576)),
            )
            .await
            {
                Ok(pair) => pair,
                Err(compat::RequestBuildError::PayloadTooLarge) => {
                    return json_err(413, "{\"error\":\"payload too large\"}");
                }
                Err(_) => return json_err(400, "{\"error\":\"invalid request\"}"),
            };
            match hub.publish(&c, inner_req.body) {
                Some(n) => build_response(
                    200,
                    "application/json",
                    format!("{{\"delivered\":{n}}}").into_bytes(),
                    &[],
                ),
                None => build_response(
                    507,
                    "application/json",
                    b"{\"error\":\"ws channel cap reached\"}".to_vec(),
                    &[],
                ),
            }
        }
        // Subscribe: `GET /_/ws/{channel}` with Upgrade headers.
        ("GET", path) if !path.is_empty() && path != "/" => {
            // Drop the leading '/'; the rest is the channel name.
            let channel = path.trim_start_matches('/').to_string();
            if channel.is_empty() {
                return json_err(400, "{\"error\":\"missing channel in path\"}");
            }
            // Reserve these names for HTTP-only routes.
            if matches!(channel.as_str(), "publish" | "stats" | "subscribers") {
                return json_err(404, "{\"error\":\"not a channel\"}");
            }
            websocket::handle_ws_upgrade(hub, req, channel)
        }
        _ => build_response(404, "text/plain", b"Not found".to_vec(), &[]),
    }
}

fn map_async_err(e: AsyncIoError) -> HyperResponse {
    match e {
        AsyncIoError::PathNotAllowed => json_err(403, "{\"error\":\"path not allowed\"}"),
        AsyncIoError::TooLarge(n) => build_response(
            413,
            "application/json",
            format!("{{\"error\":\"payload too large\",\"max\":{n}}}").into_bytes(),
            &[],
        ),
        AsyncIoError::Io(err) => build_response(
            502,
            "application/json",
            format!("{{\"error\":\"io\",\"kind\":\"{:?}\"}}", err.kind()).into_bytes(),
            &[],
        ),
        AsyncIoError::TimerWithoutQueue => {
            json_err(409, "{\"error\":\"timer requires [task_queue] enabled\"}")
        }
        AsyncIoError::DelayTooLong(n) => build_response(
            400,
            "application/json",
            format!("{{\"error\":\"delay too long\",\"max_ms\":{n}}}").into_bytes(),
            &[],
        ),
    }
}

/// Async-I/O endpoints (`/_/async/*`).  Callers must check the path
/// prefix and `state.async_io.is_some()` before invoking.
pub async fn handle_async_io(
    state: &ServerState,
    req: Request<Incoming>,
    method: &str,
    clean_path: &str,
    remote_addr: SocketAddr,
) -> HyperResponse {
    let Some(io) = state.async_io.as_ref().cloned() else {
        return build_response(404, "text/plain", b"Not found".to_vec(), &[]);
    };
    let qs = req.uri().query().unwrap_or("").to_string();
    let sub = clean_path.strip_prefix("/_/async").unwrap_or("");

    match (method, sub) {
        ("GET", "/stats") => {
            let s = io.stats();
            let body = format!(
                "{{\"reads\":{},\"writes\":{},\"timers_scheduled\":{},\"timers_fired\":{},\"allowed_roots\":{}}}",
                s.reads, s.writes, s.timers_scheduled, s.timers_fired, s.allowed_roots
            );
            build_response(200, "application/json", body.into_bytes(), &[])
        }
        ("POST", "/read") => {
            // Body carries the path as a plain string.  Query has offset/length.
            let offset: u64 = query_param(&qs, "offset")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let length: usize = query_param(&qs, "length")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let (inner_req, _) = match FullHttpRequest::from_hyper(
                req,
                remote_addr,
                &state.upload_tmp_dir,
                &state.upload_security,
                Some(4096),
            )
            .await
            {
                Ok(pair) => pair,
                Err(_) => return json_err(400, "{\"error\":\"invalid request\"}"),
            };
            let path = match std::str::from_utf8(&inner_req.body) {
                Ok(s) => s.trim().to_string(),
                Err(_) => return json_err(400, "{\"error\":\"non-utf8 path\"}"),
            };
            match io.read(&path, offset, length).await {
                Ok(bytes) => build_response(200, "application/octet-stream", bytes, &[]),
                Err(AsyncIoError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                    build_response(404, "text/plain", Vec::new(), &[])
                }
                Err(e) => map_async_err(e),
            }
        }
        ("POST", "/write") => {
            let Some(path) = query_param(&qs, "path").map(str::to_string) else {
                return json_err(400, "{\"error\":\"missing path\"}");
            };
            let append = query_param(&qs, "append")
                .map(|v| matches!(v, "1" | "true"))
                .unwrap_or(false);
            let (inner_req, _) = match FullHttpRequest::from_hyper(
                req,
                remote_addr,
                &state.upload_tmp_dir,
                &state.upload_security,
                Some(
                    state
                        .max_body_bytes
                        .unwrap_or(16 * 1024 * 1024)
                        .max(1_048_576),
                ),
            )
            .await
            {
                Ok(pair) => pair,
                Err(compat::RequestBuildError::PayloadTooLarge) => {
                    return json_err(413, "{\"error\":\"payload too large\"}");
                }
                Err(_) => return json_err(400, "{\"error\":\"invalid request\"}"),
            };
            match io.write(&path, &inner_req.body, append).await {
                Ok(n) => build_response(
                    200,
                    "application/json",
                    format!("{{\"bytes\":{n}}}").into_bytes(),
                    &[],
                ),
                Err(e) => map_async_err(e),
            }
        }
        ("POST", "/timer") => {
            // Schedule a push to task_queue after delay_ms.
            let Some(channel) = query_param(&qs, "channel").map(str::to_string) else {
                return json_err(400, "{\"error\":\"missing channel\"}");
            };
            let delay_ms: u64 = match query_param(&qs, "delay_ms").and_then(|s| s.parse().ok()) {
                Some(v) => v,
                None => return json_err(400, "{\"error\":\"missing delay_ms\"}"),
            };
            let (inner_req, _) = match FullHttpRequest::from_hyper(
                req,
                remote_addr,
                &state.upload_tmp_dir,
                &state.upload_security,
                Some(state.max_body_bytes.unwrap_or(1_048_576).max(1_048_576)),
            )
            .await
            {
                Ok(pair) => pair,
                Err(compat::RequestBuildError::PayloadTooLarge) => {
                    return json_err(413, "{\"error\":\"payload too large\"}");
                }
                Err(_) => return json_err(400, "{\"error\":\"invalid request\"}"),
            };
            match io.schedule_timer(
                state.task_queue.clone(),
                channel,
                inner_req.body,
                std::time::Duration::from_millis(delay_ms),
            ) {
                Ok(()) => build_response(
                    202,
                    "application/json",
                    b"{\"scheduled\":true}".to_vec(),
                    &[],
                ),
                Err(e) => map_async_err(e),
            }
        }
        _ => build_response(404, "text/plain", b"Not found".to_vec(), &[]),
    }
}
