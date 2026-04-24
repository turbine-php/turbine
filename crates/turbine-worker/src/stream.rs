//! Streaming response protocol (Phase 1 of the streaming-body refactor).
//!
//! ## Motivation
//!
//! Historically Turbine accumulated every `ub_write` call from PHP into a
//! thread-local buffer, then shipped the whole response to the HTTP task via a
//! single `write_response` message. That was simple but had two real
//! consequences:
//!
//! 1. **No streaming.** `flush()`, SSE (`text/event-stream`), chunked exports,
//!    and any "long-running response with progressive output" cannot reach the
//!    client until PHP finishes. TTFB ≈ total request time.
//! 2. **Skewed many-write benchmarks.** Benchmarks doing many small `echo`s
//!    never measured real syscall cost — only a memcpy into a `Vec<u8>` — so
//!    numbers were optimistic compared to fpm/FrankenPHP, which actually push
//!    bytes to the socket as PHP writes.
//!
//! This module defines a **framed** protocol on the worker→master pipe so the
//! worker can emit output incrementally. Phase 1 keeps the existing
//! `NativeResponse` API intact (the reader reassembles frames into a full
//! response), but wires the mechanism that Phase 2 will use to stream bytes
//! directly into a hyper `StreamBody`.
//!
//! ## Wire format
//!
//! Every frame begins with a single byte discriminant. Bodies use little-endian
//! integers (matching the rest of our IPC). Frames are concatenated on the pipe
//! and a response ends when an `End` or `Error` frame is read.
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │ 0x10  Headers                                                │
//! │   [2] http_status                                            │
//! │   [4] header_count                                           │
//! │   for each header:                                           │
//! │     [2] name_len  [name bytes]                               │
//! │     [2] value_len [value bytes]                              │
//! ├──────────────────────────────────────────────────────────────┤
//! │ 0x11  BodyChunk                                              │
//! │   [4] chunk_len  [chunk bytes]                               │
//! ├──────────────────────────────────────────────────────────────┤
//! │ 0x12  End                                                    │
//! │   [1] ok  (1 = success, 0 = PHP execution failed)            │
//! ├──────────────────────────────────────────────────────────────┤
//! │ 0x13  Error                                                  │
//! │   [4] msg_len  [msg bytes]   (terminal; no End follows)      │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! ### Ordering contract
//!
//! * `Headers` MUST be the first frame of a response.
//! * Any number of `BodyChunk` frames may follow (including zero).
//! * Exactly one terminal frame (`End` or `Error`) closes the response.
//!
//! Phase 1 always emits `Headers` before the first `BodyChunk`, even when the
//! worker decides that at the end — this matches what PHP scripts actually
//! expect (headers sent at `ub_write` time or at `php_output_end_all`).

use std::io::{self, Read, Write};

// ── Frame discriminants ─────────────────────────────────────────────────────

/// `Headers` frame — HTTP status + headers. Always the first frame of a response.
pub const FRAME_HEADERS: u8 = 0x10;

/// `BodyChunk` frame — a slice of the response body. May appear 0..N times.
pub const FRAME_BODY_CHUNK: u8 = 0x11;

/// `End` frame — marks successful completion. Carries the `ok` flag.
pub const FRAME_END: u8 = 0x12;

/// `Error` frame — terminal error; no `End` follows.
pub const FRAME_ERROR: u8 = 0x13;

// ── Encoding helpers ────────────────────────────────────────────────────────

/// Encode a `Headers` frame into `buf`.
pub fn encode_headers(buf: &mut Vec<u8>, http_status: u16, headers: &[(String, String)]) {
    buf.push(FRAME_HEADERS);
    buf.extend_from_slice(&http_status.to_le_bytes());
    buf.extend_from_slice(&(headers.len() as u32).to_le_bytes());
    for (name, value) in headers {
        let name_bytes = name.as_bytes();
        let value_bytes = value.as_bytes();
        buf.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(name_bytes);
        buf.extend_from_slice(&(value_bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(value_bytes);
    }
}

/// Encode a `BodyChunk` frame into `buf`.
pub fn encode_body_chunk(buf: &mut Vec<u8>, chunk: &[u8]) {
    buf.push(FRAME_BODY_CHUNK);
    buf.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
    buf.extend_from_slice(chunk);
}

/// Encode only the 5-byte prefix of a `BodyChunk` frame (discriminant +
/// `u32` length). Returned as a fixed-size array so the caller can place
/// the body bytes in a separate iovec and issue a single `writev(2)` —
/// avoiding a scratch memcpy of the body into a combined buffer.
#[inline]
pub fn encode_body_chunk_header(chunk_len: usize) -> [u8; 5] {
    let len = chunk_len as u32;
    let lb = len.to_le_bytes();
    [FRAME_BODY_CHUNK, lb[0], lb[1], lb[2], lb[3]]
}

/// Encode an `End` frame into `buf`.
pub fn encode_end(buf: &mut Vec<u8>, ok: bool) {
    buf.push(FRAME_END);
    buf.push(if ok { 1 } else { 0 });
}

/// Encode an `Error` frame into `buf`.
pub fn encode_error(buf: &mut Vec<u8>, msg: &[u8]) {
    buf.push(FRAME_ERROR);
    buf.extend_from_slice(&(msg.len() as u32).to_le_bytes());
    buf.extend_from_slice(msg);
}

// ── Synchronous decoder ─────────────────────────────────────────────────────

/// One decoded streaming frame.
#[derive(Debug, Clone)]
pub enum Frame {
    Headers {
        http_status: u16,
        headers: Vec<(String, String)>,
    },
    BodyChunk(Vec<u8>),
    End {
        ok: bool,
    },
    Error(Vec<u8>),
}

/// Read a single frame from a synchronous reader. Returns `Ok(None)` if the
/// reader is at EOF **before** any frame bytes have been consumed.
pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Option<Frame>> {
    let mut discr = [0u8; 1];
    match r.read(&mut discr)? {
        0 => return Ok(None),
        1 => {}
        _ => unreachable!(),
    }
    let frame = match discr[0] {
        FRAME_HEADERS => {
            let mut status = [0u8; 2];
            r.read_exact(&mut status)?;
            let mut count = [0u8; 4];
            r.read_exact(&mut count)?;
            let header_count = u32::from_le_bytes(count) as usize;
            let mut headers = Vec::with_capacity(header_count);
            for _ in 0..header_count {
                let mut nl = [0u8; 2];
                r.read_exact(&mut nl)?;
                let mut name = vec![0u8; u16::from_le_bytes(nl) as usize];
                r.read_exact(&mut name)?;
                let mut vl = [0u8; 2];
                r.read_exact(&mut vl)?;
                let mut val = vec![0u8; u16::from_le_bytes(vl) as usize];
                r.read_exact(&mut val)?;
                headers.push((
                    String::from_utf8_lossy(&name).into_owned(),
                    String::from_utf8_lossy(&val).into_owned(),
                ));
            }
            Frame::Headers {
                http_status: u16::from_le_bytes(status),
                headers,
            }
        }
        FRAME_BODY_CHUNK => {
            let mut cl = [0u8; 4];
            r.read_exact(&mut cl)?;
            let mut chunk = vec![0u8; u32::from_le_bytes(cl) as usize];
            r.read_exact(&mut chunk)?;
            Frame::BodyChunk(chunk)
        }
        FRAME_END => {
            let mut ok = [0u8; 1];
            r.read_exact(&mut ok)?;
            Frame::End { ok: ok[0] != 0 }
        }
        FRAME_ERROR => {
            let mut ml = [0u8; 4];
            r.read_exact(&mut ml)?;
            let mut msg = vec![0u8; u32::from_le_bytes(ml) as usize];
            r.read_exact(&mut msg)?;
            Frame::Error(msg)
        }
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown stream frame discriminant: 0x{other:02x}"),
            ))
        }
    };
    Ok(Some(frame))
}

// ── Async decoder (tokio) ───────────────────────────────────────────────────

/// Read a single frame from an async reader. Returns `Ok(None)` on clean EOF.
pub async fn read_frame_async<R>(r: &mut R) -> io::Result<Option<Frame>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;

    let mut discr = [0u8; 1];
    match r.read(&mut discr).await? {
        0 => return Ok(None),
        1 => {}
        _ => unreachable!(),
    }
    let frame = match discr[0] {
        FRAME_HEADERS => {
            let mut status = [0u8; 2];
            r.read_exact(&mut status).await?;
            let mut count = [0u8; 4];
            r.read_exact(&mut count).await?;
            let header_count = u32::from_le_bytes(count) as usize;
            let mut headers = Vec::with_capacity(header_count);
            for _ in 0..header_count {
                let mut nl = [0u8; 2];
                r.read_exact(&mut nl).await?;
                let mut name = vec![0u8; u16::from_le_bytes(nl) as usize];
                r.read_exact(&mut name).await?;
                let mut vl = [0u8; 2];
                r.read_exact(&mut vl).await?;
                let mut val = vec![0u8; u16::from_le_bytes(vl) as usize];
                r.read_exact(&mut val).await?;
                headers.push((
                    String::from_utf8_lossy(&name).into_owned(),
                    String::from_utf8_lossy(&val).into_owned(),
                ));
            }
            Frame::Headers {
                http_status: u16::from_le_bytes(status),
                headers,
            }
        }
        FRAME_BODY_CHUNK => {
            let mut cl = [0u8; 4];
            r.read_exact(&mut cl).await?;
            let mut chunk = vec![0u8; u32::from_le_bytes(cl) as usize];
            r.read_exact(&mut chunk).await?;
            Frame::BodyChunk(chunk)
        }
        FRAME_END => {
            let mut ok = [0u8; 1];
            r.read_exact(&mut ok).await?;
            Frame::End { ok: ok[0] != 0 }
        }
        FRAME_ERROR => {
            let mut ml = [0u8; 4];
            r.read_exact(&mut ml).await?;
            let mut msg = vec![0u8; u32::from_le_bytes(ml) as usize];
            r.read_exact(&mut msg).await?;
            Frame::Error(msg)
        }
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown stream frame discriminant: 0x{other:02x}"),
            ))
        }
    };
    Ok(Some(frame))
}

// ── Write helpers (worker → master) ─────────────────────────────────────────

/// Write a fully-buffered streaming response (Headers → optional single
/// BodyChunk → End) to a synchronous writer.
///
/// Used by the Phase 1 compatibility path: the worker still assembles the
/// whole body, but ships it in the framed protocol so the reader path is
/// unified with future streaming workers.
pub fn write_response_framed<W: Write>(
    w: &mut W,
    ok: bool,
    http_status: u16,
    headers: &[(String, String)],
    body: &[u8],
) -> io::Result<()> {
    let mut buf = Vec::with_capacity(32 + headers.len() * 32 + body.len());
    encode_headers(&mut buf, http_status, headers);
    if !body.is_empty() {
        encode_body_chunk(&mut buf, body);
    }
    encode_end(&mut buf, ok);
    w.write_all(&buf)?;
    w.flush()
}

/// Write a framed error response (single `Error` frame; no `End` follows).
pub fn write_error_framed<W: Write>(w: &mut W, msg: &[u8]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(5 + msg.len());
    encode_error(&mut buf, msg);
    w.write_all(&buf)?;
    w.flush()
}

// ── Streaming consumer helpers (Phase 2b infra) ─────────────────────────────

/// Head of a streaming response — HTTP status and headers from the initial
/// `Headers` frame, plus a channel that will yield each subsequent
/// `BodyChunk` as it arrives on the wire. The channel closes after the
/// terminal `End` or `Error` frame.
///
/// This is the integration point for hyper `StreamBody`: the HTTP handler
/// can send back headers immediately (TTFB ≈ first echo) and feed chunks
/// into a `StreamBody` as they come.
pub struct StreamingHead {
    pub http_status: u16,
    pub headers: Vec<(String, String)>,
    pub body: tokio::sync::mpsc::Receiver<Result<Vec<u8>, io::Error>>,
    /// Completion signal: resolves to `Ok(true)` on graceful `End { ok: true }`,
    /// `Ok(false)` on `End { ok: false }`, and `Err` if the worker closed the
    /// pipe mid-stream or emitted a terminal `Error` frame.
    pub done: tokio::sync::oneshot::Receiver<io::Result<bool>>,
}

impl std::fmt::Debug for StreamingHead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamingHead")
            .field("http_status", &self.http_status)
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}

/// Consume the leading `Headers` frame from `r`, then spawn a task that
/// forwards every subsequent `BodyChunk` into the returned channel.
///
/// The task takes ownership of the reader; callers get chunks back through
/// `StreamingHead::body` and completion through `StreamingHead::done`.
///
/// ## Backpressure
///
/// The channel is bounded (64 chunks). If the HTTP client is slow the
/// worker's pipe writes will stall at kernel level — exactly the
/// propagation model we want (slow consumer → slow producer, no unbounded
/// memory growth).
///
/// ## Error semantics
///
/// * Pipe EOF mid-stream → `body` yields `Err(UnexpectedEof)` and `done`
///   resolves to the same error.
/// * Terminal `Error` frame → `body` channel closes (no error item),
///   `done` resolves to `Err` carrying the payload as the message.
/// * Duplicate `Headers` / out-of-order frames → `body` yields
///   `Err(InvalidData)` and the task exits.
pub async fn consume_streaming<R>(mut r: R) -> io::Result<StreamingHead>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    // First frame must be `Headers` (or a terminal `Error`).
    let first = read_frame_async(&mut r).await?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "consume_streaming: pipe closed before Headers frame",
        )
    })?;

    let (http_status, headers) = match first {
        Frame::Headers {
            http_status,
            headers,
        } => (http_status, headers),
        Frame::Error(msg) => {
            return Err(io::Error::other(format!(
                "consume_streaming: worker emitted Error frame: {}",
                String::from_utf8_lossy(&msg)
            )));
        }
        Frame::BodyChunk(_) | Frame::End { .. } => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "consume_streaming: first frame must be Headers",
            ));
        }
    };

    let (body_tx, body_rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, io::Error>>(64);
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<io::Result<bool>>();

    // Helper that owns `r` and returns the completion result. We run this
    // in an inner scope so `r` (the `AsyncPipe` / `AsyncFd` registration)
    // is dropped BEFORE we notify `done_tx`. This is critical for
    // persistent workers: the caller awaits `done_rx`, returns the worker
    // fd to the pool, and a follow-up request may call
    // `AsyncPipe::new(resp_fd)` on the SAME fd. If the previous
    // registration is still alive at that moment, `AsyncFd::new` hits
    // `epoll_ctl(EPOLL_CTL_ADD)` → `EEXIST` and the new request 502s.
    // Dropping `r` first guarantees the deregister (EPOLL_CTL_DEL) happens
    // before the caller is woken.
    async fn drive<R>(
        mut r: R,
        body_tx: tokio::sync::mpsc::Sender<Result<Vec<u8>, io::Error>>,
    ) -> io::Result<bool>
    where
        R: tokio::io::AsyncRead + Unpin + Send,
    {
        loop {
            let frame = match read_frame_async(&mut r).await {
                Ok(Some(f)) => f,
                Ok(None) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "streaming response pipe closed before End frame",
                    ));
                }
                Err(e) => {
                    let _ = body_tx
                        .send(Err(io::Error::new(e.kind(), e.to_string())))
                        .await;
                    return Err(e);
                }
            };

            match frame {
                Frame::BodyChunk(chunk) => {
                    if body_tx.send(Ok(chunk)).await.is_err() {
                        // Receiver dropped — client disconnected. Keep
                        // draining the pipe so the worker can finish
                        // and be returned to the pool cleanly.
                        loop {
                            match read_frame_async(&mut r).await {
                                Ok(Some(Frame::End { ok })) => return Ok(ok),
                                Ok(Some(_)) => continue,
                                Ok(None) | Err(_) => {
                                    return Err(io::Error::new(
                                        io::ErrorKind::BrokenPipe,
                                        "client disconnected mid-stream",
                                    ));
                                }
                            }
                        }
                    }
                }
                Frame::End { ok } => return Ok(ok),
                Frame::Headers { .. } => {
                    let _ = body_tx
                        .send(Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "unexpected Headers frame mid-stream",
                        )))
                        .await;
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "duplicate Headers frame",
                    ));
                }
                Frame::Error(msg) => {
                    return Err(io::Error::other(
                        String::from_utf8_lossy(&msg).into_owned(),
                    ));
                }
            }
        }
    }

    tokio::spawn(async move {
        let result = drive(r, body_tx).await;
        // `r` has been moved into `drive` and dropped as `drive`
        // returned — AsyncFd is deregistered from the reactor NOW, before
        // we wake the caller via `done_tx`. Any follow-up
        // `AsyncPipe::new(same_fd)` is safe.
        let _ = done_tx.send(result);
    });

    Ok(StreamingHead {
        http_status,
        headers,
        body: body_rx,
        done: done_rx,
    })
}

/// Read only the leading `Headers` frame from `r` and return
/// `(http_status, headers, reader)`. The caller keeps `reader` and can
/// either drive true streaming (spawn a forwarder via
/// `start_streaming_forwarder`) or drain the body inline via
/// `drain_body_buffered`.
///
/// Splitting the dispatch this way avoids the unconditional
/// `tokio::spawn + mpsc::channel(64) + extend_from_slice` overhead of
/// `consume_streaming` when the response is actually buffered (no SSE /
/// `X-Accel-Buffering: no` opt-in). For large buffered bodies (e.g.
/// 50 KB binary responses) that overhead dominates — the mpsc hop plus
/// the body_buf extension adds a second ~body_len memcpy on top of the
/// one already done by `read_exact` inside `read_frame_async`.
pub async fn read_response_head<R>(
    mut r: R,
) -> io::Result<(u16, Vec<(String, String)>, R)>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let first = read_frame_async(&mut r).await?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "read_response_head: pipe closed before Headers frame",
        )
    })?;
    match first {
        Frame::Headers {
            http_status,
            headers,
        } => Ok((http_status, headers, r)),
        Frame::Error(msg) => Err(io::Error::other(format!(
            "read_response_head: worker emitted Error frame: {}",
            String::from_utf8_lossy(&msg)
        ))),
        Frame::BodyChunk(_) | Frame::End { .. } => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "read_response_head: first frame must be Headers",
        )),
    }
}

/// Drain `BodyChunk` frames inline into a single `Vec<u8>` until the
/// terminal `End` (or `Error`) frame, returning `(body, ok)`.
///
/// Inline variant of the buffered-fallback path: no `tokio::spawn`, no
/// bounded mpsc channel, and `BodyChunk` bytes are read straight into the
/// accumulator via `read_exact` so we pay exactly one alloc + one memcpy
/// per chunk (vs the `consume_streaming` path which allocates a per-chunk
/// `Vec<u8>`, sends it over mpsc, then re-copies it via
/// `extend_from_slice` on the consumer side).
pub async fn drain_body_buffered<R>(r: &mut R) -> io::Result<(Vec<u8>, bool)>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;

    let mut body: Vec<u8> = Vec::new();
    loop {
        let mut discr = [0u8; 1];
        match r.read(&mut discr).await? {
            0 => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "drain_body_buffered: pipe closed before End frame",
                ));
            }
            1 => {}
            _ => unreachable!(),
        }
        match discr[0] {
            FRAME_BODY_CHUNK => {
                let mut cl = [0u8; 4];
                r.read_exact(&mut cl).await?;
                let chunk_len = u32::from_le_bytes(cl) as usize;
                // Extend body in-place and read chunk bytes DIRECTLY into
                // the accumulator — no per-chunk Vec allocation and no
                // second memcpy.
                let old_len = body.len();
                body.resize(old_len + chunk_len, 0);
                r.read_exact(&mut body[old_len..old_len + chunk_len]).await?;
            }
            FRAME_END => {
                let mut ok = [0u8; 1];
                r.read_exact(&mut ok).await?;
                return Ok((body, ok[0] != 0));
            }
            FRAME_ERROR => {
                let mut ml = [0u8; 4];
                r.read_exact(&mut ml).await?;
                let mut msg = vec![0u8; u32::from_le_bytes(ml) as usize];
                r.read_exact(&mut msg).await?;
                return Err(io::Error::other(
                    String::from_utf8_lossy(&msg).into_owned(),
                ));
            }
            FRAME_HEADERS => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "drain_body_buffered: unexpected Headers frame mid-body",
                ));
            }
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("drain_body_buffered: unknown frame discriminant: 0x{other:02x}"),
                ));
            }
        }
    }
}

/// Spawn the streaming forwarder task on an already-head-consumed reader.
///
/// Use-case: caller has already parsed the `Headers` frame via
/// `read_response_head` and decided (based on the headers) that the
/// response is a true stream (SSE / `X-Accel-Buffering: no`). This starts
/// the same body-forwarding machinery that `consume_streaming` uses: a
/// spawned task reads each subsequent `BodyChunk` frame and feeds the
/// returned channel; on `End` or pipe error the `done` oneshot resolves.
///
/// The reader is dropped inside the spawned task **before** `done_tx`
/// fires, so the `AsyncFd` epoll registration is torn down before the
/// caller is woken (avoids `EEXIST` on fd reuse — see `consume_streaming`
/// for the full explanation).
pub fn start_streaming_forwarder<R>(
    r: R,
) -> (
    tokio::sync::mpsc::Receiver<Result<Vec<u8>, io::Error>>,
    tokio::sync::oneshot::Receiver<io::Result<bool>>,
)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let (body_tx, body_rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, io::Error>>(64);
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<io::Result<bool>>();

    async fn drive<R>(
        mut r: R,
        body_tx: tokio::sync::mpsc::Sender<Result<Vec<u8>, io::Error>>,
    ) -> io::Result<bool>
    where
        R: tokio::io::AsyncRead + Unpin + Send,
    {
        loop {
            let frame = match read_frame_async(&mut r).await {
                Ok(Some(f)) => f,
                Ok(None) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "streaming response pipe closed before End frame",
                    ));
                }
                Err(e) => {
                    let _ = body_tx
                        .send(Err(io::Error::new(e.kind(), e.to_string())))
                        .await;
                    return Err(e);
                }
            };

            match frame {
                Frame::BodyChunk(chunk) => {
                    if body_tx.send(Ok(chunk)).await.is_err() {
                        // Client disconnected — drain the pipe so the worker
                        // can finish cleanly.
                        loop {
                            match read_frame_async(&mut r).await {
                                Ok(Some(Frame::End { ok })) => return Ok(ok),
                                Ok(Some(_)) => continue,
                                Ok(None) | Err(_) => {
                                    return Err(io::Error::new(
                                        io::ErrorKind::BrokenPipe,
                                        "client disconnected mid-stream",
                                    ));
                                }
                            }
                        }
                    }
                }
                Frame::End { ok } => return Ok(ok),
                Frame::Headers { .. } => {
                    let _ = body_tx
                        .send(Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "unexpected Headers frame mid-stream",
                        )))
                        .await;
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "duplicate Headers frame",
                    ));
                }
                Frame::Error(msg) => {
                    return Err(io::Error::other(
                        String::from_utf8_lossy(&msg).into_owned(),
                    ));
                }
            }
        }
    }

    tokio::spawn(async move {
        let result = drive(r, body_tx).await;
        let _ = done_tx.send(result);
    });

    (body_rx, done_rx)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn roundtrip_headers_chunk_end() {
        let mut buf = Vec::new();
        encode_headers(
            &mut buf,
            200,
            &[("Content-Type".into(), "text/plain".into())],
        );
        encode_body_chunk(&mut buf, b"hello ");
        encode_body_chunk(&mut buf, b"world");
        encode_end(&mut buf, true);

        let mut cur = Cursor::new(buf);
        let f1 = read_frame(&mut cur).unwrap().unwrap();
        match f1 {
            Frame::Headers {
                http_status,
                headers,
            } => {
                assert_eq!(http_status, 200);
                assert_eq!(headers, vec![("Content-Type".into(), "text/plain".into())]);
            }
            _ => panic!("expected Headers"),
        }
        let f2 = read_frame(&mut cur).unwrap().unwrap();
        assert!(matches!(f2, Frame::BodyChunk(ref c) if c == b"hello "));
        let f3 = read_frame(&mut cur).unwrap().unwrap();
        assert!(matches!(f3, Frame::BodyChunk(ref c) if c == b"world"));
        let f4 = read_frame(&mut cur).unwrap().unwrap();
        assert!(matches!(f4, Frame::End { ok: true }));
        let f5 = read_frame(&mut cur).unwrap();
        assert!(f5.is_none());
    }

    #[test]
    fn roundtrip_error() {
        let mut buf = Vec::new();
        encode_error(&mut buf, b"boom");
        let mut cur = Cursor::new(buf);
        match read_frame(&mut cur).unwrap().unwrap() {
            Frame::Error(msg) => assert_eq!(&msg, b"boom"),
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn write_response_framed_produces_parseable_stream() {
        let mut buf = Vec::new();
        write_response_framed(
            &mut buf,
            true,
            201,
            &[("X-Test".into(), "1".into())],
            b"body",
        )
        .unwrap();

        let mut cur = Cursor::new(buf);
        let h = read_frame(&mut cur).unwrap().unwrap();
        assert!(matches!(
            h,
            Frame::Headers {
                http_status: 201,
                ..
            }
        ));
        let b = read_frame(&mut cur).unwrap().unwrap();
        assert!(matches!(b, Frame::BodyChunk(ref c) if c == b"body"));
        let e = read_frame(&mut cur).unwrap().unwrap();
        assert!(matches!(e, Frame::End { ok: true }));
    }

    #[test]
    fn empty_body_omits_body_chunk() {
        let mut buf = Vec::new();
        write_response_framed(&mut buf, true, 204, &[], b"").unwrap();
        let mut cur = Cursor::new(buf);
        assert!(matches!(
            read_frame(&mut cur).unwrap().unwrap(),
            Frame::Headers {
                http_status: 204,
                ..
            }
        ));
        assert!(matches!(
            read_frame(&mut cur).unwrap().unwrap(),
            Frame::End { ok: true }
        ));
    }

    #[tokio::test]
    async fn async_decoder_matches_sync() {
        let mut buf = Vec::new();
        encode_headers(&mut buf, 200, &[]);
        encode_body_chunk(&mut buf, b"abc");
        encode_end(&mut buf, true);

        let mut cur = std::io::Cursor::new(buf);
        // tokio::io::AsyncRead is implemented for Cursor via compat? use tokio::io helpers.
        // Simpler: wrap in tokio::io::BufReader via duplex — use a Vec+Cursor through
        // tokio::io::AsyncReadExt by feeding bytes into a duplex pipe.
        let data = {
            let mut v = Vec::new();
            std::io::copy(&mut cur, &mut v).unwrap();
            v
        };
        let (mut client, mut server) = tokio::io::duplex(1024);
        tokio::io::AsyncWriteExt::write_all(&mut server, &data)
            .await
            .unwrap();
        drop(server);

        let f1 = read_frame_async(&mut client).await.unwrap().unwrap();
        assert!(matches!(
            f1,
            Frame::Headers {
                http_status: 200,
                ..
            }
        ));
        let f2 = read_frame_async(&mut client).await.unwrap().unwrap();
        assert!(matches!(f2, Frame::BodyChunk(ref c) if c == b"abc"));
        let f3 = read_frame_async(&mut client).await.unwrap().unwrap();
        assert!(matches!(f3, Frame::End { ok: true }));
        let f4 = read_frame_async(&mut client).await.unwrap();
        assert!(f4.is_none());
    }

    #[tokio::test]
    async fn consume_streaming_yields_chunks_incrementally() {
        let (client, server) = tokio::io::duplex(4096);

        // Simulate a worker streaming Headers → chunk → chunk → chunk → End.
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let mut w = server;
            let mut buf = Vec::new();
            encode_headers(
                &mut buf,
                200,
                &[("Content-Type".into(), "text/event-stream".into())],
            );
            w.write_all(&buf).await.unwrap();
            for i in 0..3 {
                let mut chunk_buf = Vec::new();
                encode_body_chunk(&mut chunk_buf, format!("data: {i}\n\n").as_bytes());
                w.write_all(&chunk_buf).await.unwrap();
            }
            let mut end_buf = Vec::new();
            encode_end(&mut end_buf, true);
            w.write_all(&end_buf).await.unwrap();
        });

        let mut head = consume_streaming(client).await.unwrap();
        assert_eq!(head.http_status, 200);
        assert_eq!(head.headers.len(), 1);
        assert_eq!(head.headers[0].0, "Content-Type");
        assert_eq!(head.headers[0].1, "text/event-stream");

        let mut received = Vec::new();
        while let Some(chunk) = head.body.recv().await {
            received.push(chunk.unwrap());
        }
        assert_eq!(received.len(), 3);
        assert_eq!(received[0], b"data: 0\n\n");
        assert_eq!(received[2], b"data: 2\n\n");

        let ok = head.done.await.unwrap().unwrap();
        assert!(ok);
    }

    #[tokio::test]
    async fn consume_streaming_propagates_worker_error_frame() {
        let (client, server) = tokio::io::duplex(1024);
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let mut w = server;
            let mut buf = Vec::new();
            encode_error(&mut buf, b"boot failure");
            w.write_all(&buf).await.unwrap();
        });

        let err = consume_streaming(client).await.unwrap_err();
        assert!(err.to_string().contains("boot failure"));
    }
}
