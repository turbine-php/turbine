//! Prometheus exposition for the Swoole-like primitives.
//!
//! Exposes `/_/metrics` in Prometheus text format (v0.0.4) with
//! counters/gauges for SharedTable, TaskQueue, WsHub and AsyncIo.
//!
//! Only primitives that are actually configured emit metrics — we skip
//! anything that is `None` on [`ServerState`].  The endpoint itself is
//! always mounted; if no primitive is enabled the body is just the
//! process-level stub so Prometheus scrapes do not 404.

use std::fmt::Write as _;

use bytes::Bytes;
use http_body_util::Full;
use hyper::Response;

use crate::ServerState;
use crate::build_response;

type HyperResponse = Response<Full<Bytes>>;

/// Render `/_/metrics` in Prometheus text format.
pub fn handle_metrics(state: &ServerState) -> HyperResponse {
    let body = render(state);
    build_response(
        200,
        "text/plain; version=0.0.4; charset=utf-8",
        body.into_bytes(),
        &[],
    )
}

/// Build the metrics body.  Kept separate so tests can assert on the
/// exact output without needing a real response.
pub fn render(state: &ServerState) -> String {
    let mut out = String::with_capacity(1024);

    // Always emit a tiny server marker so scrapes never return empty.
    out.push_str("# HELP turbine_build_info Build/runtime marker (always 1).\n");
    out.push_str("# TYPE turbine_build_info gauge\n");
    out.push_str("turbine_build_info 1\n");

    if let Some(t) = state.shared_table.as_ref() {
        let _ = writeln!(
            out,
            "# HELP turbine_shared_table_size Current number of entries in the shared table."
        );
        let _ = writeln!(out, "# TYPE turbine_shared_table_size gauge");
        let _ = writeln!(out, "turbine_shared_table_size {}", t.size());

        let _ = writeln!(
            out,
            "# HELP turbine_shared_table_evictions_total Total entries evicted due to capacity/TTL."
        );
        let _ = writeln!(out, "# TYPE turbine_shared_table_evictions_total counter");
        let _ = writeln!(
            out,
            "turbine_shared_table_evictions_total {}",
            t.evictions()
        );
    }

    if let Some(q) = state.task_queue.as_ref() {
        let s = q.stats();
        let _ = writeln!(out, "# HELP turbine_task_queue_channels Active channels.");
        let _ = writeln!(out, "# TYPE turbine_task_queue_channels gauge");
        let _ = writeln!(out, "turbine_task_queue_channels {}", s.channels);

        let _ = writeln!(
            out,
            "# HELP turbine_task_queue_pushed_total Jobs enqueued since startup."
        );
        let _ = writeln!(out, "# TYPE turbine_task_queue_pushed_total counter");
        let _ = writeln!(out, "turbine_task_queue_pushed_total {}", s.pushed);

        let _ = writeln!(
            out,
            "# HELP turbine_task_queue_popped_total Jobs consumed since startup."
        );
        let _ = writeln!(out, "# TYPE turbine_task_queue_popped_total counter");
        let _ = writeln!(out, "turbine_task_queue_popped_total {}", s.popped);

        let _ = writeln!(
            out,
            "# HELP turbine_task_queue_rejected_total Jobs rejected (queue full / channel cap)."
        );
        let _ = writeln!(out, "# TYPE turbine_task_queue_rejected_total counter");
        let _ = writeln!(out, "turbine_task_queue_rejected_total {}", s.rejected);
    }

    if let Some(ws) = state.ws_hub.as_ref() {
        let s = ws.stats();
        let _ = writeln!(out, "# HELP turbine_ws_channels Active websocket channels.");
        let _ = writeln!(out, "# TYPE turbine_ws_channels gauge");
        let _ = writeln!(out, "turbine_ws_channels {}", s.channels);

        let _ = writeln!(
            out,
            "# HELP turbine_ws_published_total Messages published to websocket channels."
        );
        let _ = writeln!(out, "# TYPE turbine_ws_published_total counter");
        let _ = writeln!(out, "turbine_ws_published_total {}", s.published);

        let _ = writeln!(
            out,
            "# HELP turbine_ws_subscribed_total Websocket subscribe events since startup."
        );
        let _ = writeln!(out, "# TYPE turbine_ws_subscribed_total counter");
        let _ = writeln!(out, "turbine_ws_subscribed_total {}", s.subscribed);

        let _ = writeln!(
            out,
            "# HELP turbine_ws_rejected_total Websocket subscribe/publish rejections."
        );
        let _ = writeln!(out, "# TYPE turbine_ws_rejected_total counter");
        let _ = writeln!(out, "turbine_ws_rejected_total {}", s.rejected);
    }

    if let Some(a) = state.async_io.as_ref() {
        let s = a.stats();
        let _ = writeln!(
            out,
            "# HELP turbine_async_reads_total Async file reads completed."
        );
        let _ = writeln!(out, "# TYPE turbine_async_reads_total counter");
        let _ = writeln!(out, "turbine_async_reads_total {}", s.reads);

        let _ = writeln!(
            out,
            "# HELP turbine_async_writes_total Async file writes completed."
        );
        let _ = writeln!(out, "# TYPE turbine_async_writes_total counter");
        let _ = writeln!(out, "turbine_async_writes_total {}", s.writes);

        let _ = writeln!(
            out,
            "# HELP turbine_async_timers_scheduled_total Timers scheduled."
        );
        let _ = writeln!(out, "# TYPE turbine_async_timers_scheduled_total counter");
        let _ = writeln!(
            out,
            "turbine_async_timers_scheduled_total {}",
            s.timers_scheduled
        );

        let _ = writeln!(
            out,
            "# HELP turbine_async_timers_fired_total Timers that fired and enqueued a task."
        );
        let _ = writeln!(out, "# TYPE turbine_async_timers_fired_total counter");
        let _ = writeln!(
            out,
            "turbine_async_timers_fired_total {}",
            s.timers_fired
        );

        let _ = writeln!(
            out,
            "# HELP turbine_async_allowed_roots Number of configured async I/O roots."
        );
        let _ = writeln!(out, "# TYPE turbine_async_allowed_roots gauge");
        let _ = writeln!(out, "turbine_async_allowed_roots {}", s.allowed_roots);
    }

    out
}
