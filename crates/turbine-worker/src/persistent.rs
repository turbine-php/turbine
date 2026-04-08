/// Turbine persistent PHP worker mode.
///
/// Workers bootstrap once (load autoloader, warm OPcache) then handle N
/// requests using the native SAPI path (`turbine_sapi_set_request` +
/// `php_execute_script`).  This gives OPcache acceleration with a lightweight
/// per-request lifecycle that preserves PHP globals across requests.
///
/// # Binary wire protocol
///
/// Rust -> Worker (per request):
/// ```text
/// [u8  cmd:  0x01 = HandleRequest | 0xFF = Shutdown]
/// [u32 method_len LE] [method bytes]
/// [u32 uri_len LE]    [uri bytes]        // full URI including ?query
/// [u32 body_len LE]   [body bytes]
/// [u32 ip_len LE]     [client_ip bytes]
/// [u32 port LE]
/// [u8  is_https]                        // 0 or 1
/// [u32 header_count LE]
///   per header: [u32 name_len][name] [u32 value_len][value]
/// [u32 script_filename_len][script_filename]
/// [u32 query_string_len][query_string]
/// [u32 document_root_len][document_root]
/// [u32 content_type_len][content_type]
/// [u32 cookie_len][cookie]
/// [u32 path_info_len][path_info]
/// [u32 script_name_len][script_name]
/// ```
///
/// Worker -> Rust:
/// ```text
/// [u8  status: 0x01 = Ok | 0x02 = Error]
/// [u16 http_status_code LE]
/// [u32 header_count LE]
///   per header: [u32 name_len][name] [u32 value_len][value]
/// [u32 body_len LE] [body bytes]
/// ```
///
/// Worker -> Rust (ready signal, sent after bootstrap):
/// ```text
/// [u8  0xAA]
/// [u32 0 LE]
/// ```

use std::io::{self, Read};
use std::os::unix::io::RawFd;

use nix::unistd::{fork, ForkResult};
use tracing::{debug, error, info, warn};

use crate::worker::Worker;
use crate::WorkerError;

// ─────────────────────────────────────────────────────────────────────────────
// Request/response data structures
// ─────────────────────────────────────────────────────────────────────────────

/// Decoded HTTP request ready to be sent to a persistent PHP worker.
#[derive(Debug)]
pub struct PersistentRequest<'a> {
    pub method:          &'a str,
    pub uri:             &'a str,          // full URI including query string
    pub body:            &'a [u8],
    pub client_ip:       &'a str,
    pub port:            u16,
    pub is_https:        bool,
    pub headers:         &'a [(&'a str, &'a str)],
    pub script_filename: &'a str,          // absolute path to PHP script
    pub query_string:    &'a str,
    pub document_root:   &'a str,
    pub content_type:    &'a str,
    pub cookie:          &'a str,
    pub path_info:       &'a str,
    pub script_name:     &'a str,
}

/// Decoded response from a persistent PHP worker.
#[derive(Debug)]
pub struct PersistentResponse {
    pub status:  u16,
    pub headers: Vec<(String, String)>,
    pub body:    Vec<u8>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Encoding helpers
// ─────────────────────────────────────────────────────────────────────────────

#[inline]
fn write_u8(buf: &mut Vec<u8>, v: u8) {
    buf.push(v);
}

#[inline]
fn write_u32_le(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

#[inline]
fn write_bytes(buf: &mut Vec<u8>, data: &[u8]) {
    write_u32_le(buf, data.len() as u32);
    buf.extend_from_slice(data);
}

#[inline]
fn write_str(buf: &mut Vec<u8>, s: &str) {
    write_bytes(buf, s.as_bytes());
}

// ─────────────────────────────────────────────────────────────────────────────
// Decoding helpers
// ─────────────────────────────────────────────────────────────────────────────

struct FdReader(RawFd);

impl Read for FdReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let ret = unsafe { libc::read(self.0, buf.as_mut_ptr() as *mut _, buf.len()) };
            if ret < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue; // Retry on EINTR
                }
                return Err(err);
            }
            return Ok(ret as usize);
        }
    }
}

fn read_exact_fd(fd: RawFd, n: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    FdReader(fd).read_exact(&mut buf)?;
    Ok(buf)
}

fn read_u8_fd(fd: RawFd) -> io::Result<u8> {
    Ok(read_exact_fd(fd, 1)?[0])
}

fn read_u16_le_fd(fd: RawFd) -> io::Result<u16> {
    let b = read_exact_fd(fd, 2)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn read_u32_le_fd(fd: RawFd) -> io::Result<u32> {
    let b = read_exact_fd(fd, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_string_fd(fd: RawFd) -> io::Result<String> {
    let len = read_u32_le_fd(fd)? as usize;
    let bytes = read_exact_fd(fd, len)?;
    String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn read_bytes_fd(fd: RawFd) -> io::Result<Vec<u8>> {
    let len = read_u32_le_fd(fd)? as usize;
    read_exact_fd(fd, len)
}

// ─────────────────────────────────────────────────────────────────────────────
// Public encoding / decoding API
// ─────────────────────────────────────────────────────────────────────────────

/// Encode a `PersistentRequest` into the binary wire format.
pub fn encode_request(req: &PersistentRequest<'_>) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256 + req.body.len());

    write_u8(&mut buf, 0x01);  // HandleRequest command
    write_str(&mut buf, req.method);
    write_str(&mut buf, req.uri);
    write_bytes(&mut buf, req.body);
    write_str(&mut buf, req.client_ip);
    write_u32_le(&mut buf, req.port as u32);
    write_u8(&mut buf, u8::from(req.is_https));
    write_u32_le(&mut buf, req.headers.len() as u32);
    for (name, value) in req.headers {
        write_str(&mut buf, name);
        write_str(&mut buf, value);
    }
    // Extended fields for native SAPI execution
    write_str(&mut buf, req.script_filename);
    write_str(&mut buf, req.query_string);
    write_str(&mut buf, req.document_root);
    write_str(&mut buf, req.content_type);
    write_str(&mut buf, req.cookie);
    write_str(&mut buf, req.path_info);
    write_str(&mut buf, req.script_name);

    buf
}

/// Decode a `PersistentResponse` from the worker's resp pipe (blocking).
pub fn decode_response(resp_fd: RawFd) -> io::Result<PersistentResponse> {
    let _marker    = read_u8_fd(resp_fd)?;
    let status     = read_u16_le_fd(resp_fd)?;
    let hdr_count  = read_u32_le_fd(resp_fd)?;

    if hdr_count > 256 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("decode_response: invalid header_count={hdr_count} — pipe desynced"),
        ));
    }

    let mut headers = Vec::with_capacity(hdr_count as usize);
    for _ in 0..hdr_count {
        let name  = read_string_fd(resp_fd)?;
        let value = read_string_fd(resp_fd)?;
        headers.push((name, value));
    }

    let body = read_bytes_fd(resp_fd)?;
    Ok(PersistentResponse { status, headers, body })
}

/// Read and validate the ready signal from a persistent PHP worker.
pub fn read_ready_signal(resp_fd: RawFd) -> io::Result<bool> {
    let marker = read_u8_fd(resp_fd)?;

    if marker == 0xAA {
        let _ = read_u32_le_fd(resp_fd)?;
        Ok(true)
    } else if marker == 0x02 {
        let _ = read_u16_le_fd(resp_fd)?;
        let _ = read_u32_le_fd(resp_fd)?;
        let msg = read_bytes_fd(resp_fd)?;
        warn!(msg = %String::from_utf8_lossy(&msg), "Persistent worker bootstrap failed");
        Ok(false)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected ready byte: 0x{:X}", marker),
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Worker-side request decoding (from cmd pipe)
// ─────────────────────────────────────────────────────────────────────────────

/// Decoded request read from the cmd pipe inside the worker process.
struct DecodedRequest {
    method:          String,
    uri:             String,
    body:            Vec<u8>,
    client_ip:       String,
    port:            u16,
    is_https:        bool,
    headers:         Vec<(String, String)>,
    script_filename: String,
    query_string:    String,
    document_root:   String,
    content_type:    String,
    cookie:          String,
    path_info:       String,
    script_name:     String,
}

/// Decode a full request from the cmd pipe (blocking).
fn decode_request_from_fd(cmd_fd: RawFd) -> io::Result<DecodedRequest> {
    let method    = read_string_fd(cmd_fd)?;
    let uri       = read_string_fd(cmd_fd)?;
    let body      = read_bytes_fd(cmd_fd)?;
    let client_ip = read_string_fd(cmd_fd)?;
    let port      = read_u32_le_fd(cmd_fd)? as u16;
    let is_https  = read_u8_fd(cmd_fd)? != 0;
    let hdr_count = read_u32_le_fd(cmd_fd)? as usize;
    let mut headers = Vec::with_capacity(hdr_count);
    for _ in 0..hdr_count {
        let name  = read_string_fd(cmd_fd)?;
        let value = read_string_fd(cmd_fd)?;
        headers.push((name, value));
    }
    let script_filename = read_string_fd(cmd_fd)?;
    let query_string    = read_string_fd(cmd_fd)?;
    let document_root   = read_string_fd(cmd_fd)?;
    let content_type    = read_string_fd(cmd_fd)?;
    let cookie          = read_string_fd(cmd_fd)?;
    let path_info       = read_string_fd(cmd_fd)?;
    let script_name     = read_string_fd(cmd_fd)?;

    Ok(DecodedRequest {
        method, uri, body, client_ip, port, is_https, headers,
        script_filename, query_string, document_root, content_type,
        cookie, path_info, script_name,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Worker-side response encoding (to resp pipe)
// ─────────────────────────────────────────────────────────────────────────────

/// Write the ready signal to the response pipe.
fn write_ready_signal(resp_fd: RawFd) -> io::Result<()> {
    let mut buf = Vec::with_capacity(5);
    write_u8(&mut buf, 0xAA);
    write_u32_le(&mut buf, 0);
    write_all_fd(resp_fd, &buf)
}

/// Write a response to the response pipe.
fn write_response(
    resp_fd: RawFd,
    ok: bool,
    http_status: u16,
    headers: &[(String, String)],
    body: &[u8],
) -> io::Result<()> {
    let mut buf = Vec::with_capacity(32 + body.len());
    write_u8(&mut buf, if ok { 0x01 } else { 0x02 });
    buf.extend_from_slice(&http_status.to_le_bytes());
    write_u32_le(&mut buf, headers.len() as u32);
    for (name, value) in headers {
        write_str(&mut buf, name);
        write_str(&mut buf, value);
    }
    write_bytes(&mut buf, body);
    write_all_fd(resp_fd, &buf)
}

/// Write all bytes to a raw fd, retrying on EINTR.
fn write_all_fd(fd: RawFd, data: &[u8]) -> io::Result<()> {
    let mut offset = 0;
    while offset < data.len() {
        let ret = unsafe {
            libc::write(fd, data[offset..].as_ptr() as *const _, data.len() - offset)
        };
        if ret < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue; // Retry on EINTR
            }
            return Err(err);
        }
        offset += ret as usize;
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Persistent worker event loop (runs in the child process after fork)
// ─────────────────────────────────────────────────────────────────────────────

/// Entry point for persistent worker processes.
///
/// Uses the native SAPI path: `turbine_sapi_set_request` +
/// `php_request_startup` + `php_execute_script` per request.
/// Each request gets a clean PHP state; OPcache keeps compiled
/// opcodes warm across requests for near-zero compilation overhead.
pub fn worker_event_loop_persistent(cmd_fd: RawFd, resp_fd: RawFd, app_root: &str) {
    debug!(pid = std::process::id(), app_root = app_root, "Persistent worker started (native SAPI)");

    use crate::pool::safe_cstring;
    use turbine_engine::output;

    // When PHP is compiled with ZTS, the forked child needs its own TSRM context.
    // The parent's TSRM thread-ID mapping is stale after fork (the child has a new
    // thread ID). Without this, SG()/EG()/PG() macros resolve to invalid memory.
    // In NTS mode, turbine_thread_init() is a no-op.
    unsafe {
        if turbine_php_sys::turbine_php_is_thread_safe() != 0 {
            if turbine_php_sys::turbine_thread_init() != 0 {
                error!(pid = std::process::id(), "Failed to initialize TSRM context after fork");
                std::process::exit(1);
            }
            debug!(pid = std::process::id(), "TSRM context initialized after fork (ZTS)");
        }
    }

    // Install Turbine SAPI hooks and output capture.
    unsafe {
        turbine_php_sys::turbine_sapi_install_hooks();
        output::install_output_capture();
    }

    // Signal ready to the parent.
    if let Err(e) = write_ready_signal(resp_fd) {
        error!(pid = std::process::id(), error = %e, "Failed to send ready signal");
        std::process::exit(1);
    }

    // ── Request loop ─────────────────────────────────────────────────
    loop {
        // Read command byte.
        let cmd = match read_u8_fd(cmd_fd) {
            Ok(c) => c,
            Err(e) => {
                debug!(pid = std::process::id(), error = %e, "Command pipe closed — shutting down");
                break;
            }
        };

        if cmd == 0xFF {
            info!(pid = std::process::id(), "Worker received shutdown command");
            break;
        }

        if cmd != 0x01 {
            warn!(pid = std::process::id(), cmd = cmd, "Unknown command byte");
            let _ = write_response(resp_fd, false, 500, &[], b"Unknown command");
            continue;
        }

        // Decode request payload.
        let req = match decode_request_from_fd(cmd_fd) {
            Ok(r) => r,
            Err(e) => {
                error!(pid = std::process::id(), error = %e, "Failed to decode request");
                let _ = write_response(resp_fd, false, 500, &[], b"Failed to decode request");
                continue;
            }
        };

        debug!(
            pid = std::process::id(),
            script = %req.script_filename,
            method = %req.method,
            uri = %req.uri,
            "Executing via native SAPI (persistent)"
        );

        // Build CStrings for the C API.
        let c_method     = safe_cstring(req.method.as_bytes());
        let c_uri        = safe_cstring(req.uri.as_bytes());
        let c_qs         = safe_cstring(req.query_string.as_bytes());
        let c_ct         = safe_cstring(req.content_type.as_bytes());
        let c_cookie     = safe_cstring(req.cookie.as_bytes());
        let c_script     = safe_cstring(req.script_filename.as_bytes());
        let c_docroot    = safe_cstring(req.document_root.as_bytes());
        let c_addr       = safe_cstring(req.client_ip.as_bytes());
        let c_pathinfo   = safe_cstring(req.path_info.as_bytes());
        let c_scriptname = safe_cstring(req.script_name.as_bytes());

        let c_keys: Vec<std::ffi::CString> = req.headers.iter()
            .map(|(k, _)| safe_cstring(k.as_bytes()))
            .collect();
        let c_vals: Vec<std::ffi::CString> = req.headers.iter()
            .map(|(_, v)| safe_cstring(v.as_bytes()))
            .collect();
        let key_ptrs: Vec<*const std::ffi::c_char> = c_keys.iter().map(|k| k.as_ptr()).collect();
        let val_ptrs: Vec<*const std::ffi::c_char> = c_vals.iter().map(|v| v.as_ptr()).collect();

        let content_length: libc::c_long = if req.body.is_empty() {
            -1
        } else {
            req.body.len() as libc::c_long
        };

        unsafe {
            // 1. Set SAPI request info (BEFORE request startup).
            turbine_php_sys::turbine_sapi_set_request(
                c_method.as_ptr(),
                c_uri.as_ptr(),
                c_qs.as_ptr(),
                if req.content_type.is_empty() { std::ptr::null() } else { c_ct.as_ptr() },
                content_length,
                if req.cookie.is_empty() { std::ptr::null() } else { c_cookie.as_ptr() },
                c_script.as_ptr(),
                c_docroot.as_ptr(),
                c_addr.as_ptr(),
                0, // remote_port
                req.port as libc::c_int,
                req.is_https as libc::c_int,
                c_pathinfo.as_ptr(),
                c_scriptname.as_ptr(),
                if req.body.is_empty() { std::ptr::null() } else { req.body.as_ptr() as *const _ },
                req.body.len(),
                req.headers.len() as libc::c_int,
                if key_ptrs.is_empty() { std::ptr::null() } else { key_ptrs.as_ptr() },
                if val_ptrs.is_empty() { std::ptr::null() } else { val_ptrs.as_ptr() },
            );

            // 2. Full request startup — clean PHP state, OPcache stays warm.
            turbine_php_sys::php_request_startup();

            // 3. Install output capture AFTER startup (startup resets SAPI callbacks).
            output::install_output_capture();
            output::clear_output_buffer();

            // 4. Execute PHP script — uses OPcache, standard Zend VM path.
            let result = turbine_php_sys::turbine_execute_script(c_script.as_ptr());

            // 5. Capture output, headers, status code BEFORE shutdown.
            //    php_request_shutdown may reset SAPI state or free globals.
            let body = output::take_output();
            let headers = output::take_headers();
            let status = output::take_response_code();

            // 6. Full request shutdown — resets all PHP state for next request.
            turbine_php_sys::php_request_shutdown(std::ptr::null_mut());

            let ok = result == turbine_php_sys::SUCCESS;
            if let Err(e) = write_response(resp_fd, ok, status, &headers, &body) {
                error!(pid = std::process::id(), error = %e, "Failed to write response — exiting worker");
                break;
            }
        }
    }

    // Clean up before exiting.
    debug!(pid = std::process::id(), "Persistent worker exited");
}

// ─────────────────────────────────────────────────────────────────────────────
// WorkerPool extension
// ─────────────────────────────────────────────────────────────────────────────

use crate::pool::WorkerPool;

impl WorkerPool {
    /// Spawn persistent PHP workers (bootstrap-once model).
    pub fn spawn_persistent_workers(&mut self, app_root: &str) -> Result<bool, WorkerError> {
        let count = self.config().workers;
        info!(count = count, app_root = app_root, "Spawning persistent workers");
        let owned_root = app_root.to_string();

        for i in 0..count {
            let root = owned_root.clone();
            let is_master = self.spawn_one_persistent(i, root)?;
            if !is_master {
                return Ok(false);
            }
        }

        info!(spawned = self.worker_count(), "All persistent workers spawned");
        Ok(true)
    }

    fn spawn_one_persistent(&mut self, index: usize, app_root: String) -> Result<bool, WorkerError> {
        let mut cmd_pipe  = [0i32; 2];
        let mut resp_pipe = [0i32; 2];

        if unsafe { libc::pipe(cmd_pipe.as_mut_ptr())  } != 0 {
            return Err(WorkerError::Pipe(nix::Error::last()));
        }
        if unsafe { libc::pipe(resp_pipe.as_mut_ptr()) } != 0 {
            return Err(WorkerError::Pipe(nix::Error::last()));
        }

        let (cmd_read,  cmd_write)  = (cmd_pipe[0],  cmd_pipe[1]);
        let (resp_read, resp_write) = (resp_pipe[0], resp_pipe[1]);

        match unsafe { fork() }.map_err(WorkerError::Fork)? {
            ForkResult::Parent { child } => {
                unsafe {
                    libc::close(cmd_read);
                    libc::close(resp_write);
                }
                let max_req = self.config().max_requests;
                let worker = Worker::new(child, max_req, cmd_write, resp_read);
                self.push_worker(worker);
                debug!(pid = child.as_raw(), index = index, "Persistent worker forked");
                Ok(true)
            }
            ForkResult::Child => {
                unsafe {
                    libc::close(cmd_write);
                    libc::close(resp_read);
                }
                worker_event_loop_persistent(cmd_read, resp_write, &app_root);
                std::process::exit(0);
            }
        }
    }

    /// Reap dead persistent workers and respawn them.
    pub fn reap_and_respawn_persistent(&mut self, app_root: &str) -> Result<(), WorkerError> {
        let mut to_respawn = Vec::new();

        for (idx, worker) in self.workers_mut().iter_mut().enumerate() {
            if !worker.is_alive() {
                info!(
                    pid = worker.pid().as_raw(),
                    index = idx,
                    "Persistent worker exited — will respawn"
                );
                to_respawn.push(idx);
            }
        }

        for idx in to_respawn {
            self.respawn_persistent_at(idx, app_root.to_string())?;
        }

        Ok(())
    }

    /// Respawn a persistent worker at a specific index (replacing a dead one).
    pub fn respawn_persistent_at(&mut self, index: usize, app_root: String) -> Result<bool, WorkerError> {
        let mut cmd_pipe  = [0i32; 2];
        let mut resp_pipe = [0i32; 2];

        if unsafe { libc::pipe(cmd_pipe.as_mut_ptr())  } != 0 {
            return Err(WorkerError::Pipe(nix::Error::last()));
        }
        if unsafe { libc::pipe(resp_pipe.as_mut_ptr()) } != 0 {
            return Err(WorkerError::Pipe(nix::Error::last()));
        }

        let (cmd_read,  cmd_write)  = (cmd_pipe[0],  cmd_pipe[1]);
        let (resp_read, resp_write) = (resp_pipe[0], resp_pipe[1]);

        match unsafe { fork() }.map_err(WorkerError::Fork)? {
            ForkResult::Parent { child } => {
                unsafe {
                    libc::close(cmd_read);
                    libc::close(resp_write);
                }
                let max_req = self.config().max_requests;
                let worker = Worker::new(child, max_req, cmd_write, resp_read);
                self.replace_worker(index, worker);

                // Read the ready signal from the respawned worker before accepting requests.
                match read_ready_signal(resp_read) {
                    Ok(true) => {
                        info!(pid = child.as_raw(), index = index, "Persistent worker respawned and ready");
                    }
                    Ok(false) => {
                        warn!(pid = child.as_raw(), index = index, "Respawned persistent worker bootstrap failed");
                    }
                    Err(e) => {
                        error!(pid = child.as_raw(), index = index, error = %e, "Failed to read ready signal from respawned worker");
                    }
                }
                Ok(true)
            }
            ForkResult::Child => {
                unsafe {
                    libc::close(cmd_write);
                    libc::close(resp_read);
                }
                worker_event_loop_persistent(cmd_read, resp_write, &app_root);
                std::process::exit(0);
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────
    // Thread-mode persistent workers (ZTS required)
    // ─────────────────────────────────────────────────────────────────

    /// Spawn persistent workers as OS threads instead of forked processes.
    ///
    /// Each thread bootstraps the application once and handles N requests.
    /// Requires PHP compiled with ZTS.
    pub fn spawn_persistent_workers_threaded(&mut self, app_root: &str) -> Result<(), WorkerError> {
        let count = self.config().workers;
        info!(count = count, app_root = app_root, mode = "thread", "Spawning persistent worker threads");

        let is_zts = unsafe { turbine_php_sys::turbine_php_is_thread_safe() };
        if is_zts == 0 {
            error!("Thread worker mode requires PHP compiled with ZTS (--enable-zts)");
            return Err(WorkerError::Fork(nix::Error::ENOTSUP));
        }

        for i in 0..count {
            self.spawn_one_persistent_thread(i, app_root.to_string())?;
        }

        info!(spawned = self.worker_count(), "All persistent worker threads spawned");
        Ok(())
    }

    fn spawn_one_persistent_thread(&mut self, index: usize, app_root: String) -> Result<(), WorkerError> {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

        static PERSISTENT_THREAD_ID: AtomicU64 = AtomicU64::new(1);

        let mut cmd_pipe = [0i32; 2];
        let mut resp_pipe = [0i32; 2];
        if unsafe { libc::pipe(cmd_pipe.as_mut_ptr()) } != 0 {
            return Err(WorkerError::Pipe(nix::Error::last()));
        }
        if unsafe { libc::pipe(resp_pipe.as_mut_ptr()) } != 0 {
            return Err(WorkerError::Pipe(nix::Error::last()));
        }
        let (cmd_read, cmd_write) = (cmd_pipe[0], cmd_pipe[1]);
        let (resp_read, resp_write) = (resp_pipe[0], resp_pipe[1]);

        let alive = Arc::new(AtomicBool::new(true));
        let alive_clone = alive.clone();
        let thread_id = PERSISTENT_THREAD_ID.fetch_add(1, Ordering::Relaxed);

        std::thread::Builder::new()
            .name(format!("turbine-persistent-{index}"))
            .spawn(move || {
                // Initialize TSRM context
                let init_rc = unsafe { turbine_php_sys::turbine_thread_init() };
                if init_rc != 0 {
                    error!(thread_id = thread_id, "Failed to initialize TSRM for persistent thread");
                    alive_clone.store(false, Ordering::Release);
                    unsafe {
                        libc::close(cmd_read);
                        libc::close(resp_write);
                    }
                    return;
                }

                // Run the persistent event loop (same as process mode)
                worker_event_loop_persistent(cmd_read, resp_write, &app_root);

                // Clean up
                unsafe { turbine_php_sys::turbine_thread_cleanup(); }
                unsafe {
                    libc::close(cmd_read);
                    libc::close(resp_write);
                }
                alive_clone.store(false, Ordering::Release);
                debug!(thread_id = thread_id, "Persistent worker thread exited");
            })
            .map_err(|_| WorkerError::Fork(nix::Error::ENOMEM))?;

        let max_req = self.config().max_requests;
        let worker = Worker::new_thread(alive, thread_id, max_req, cmd_write, resp_read);
        self.push_worker(worker);

        debug!(thread_id = thread_id, index = index, "Persistent worker thread spawned");
        Ok(())
    }

    /// Reap dead persistent thread workers and respawn them.
    pub fn reap_and_respawn_persistent_threaded(&mut self, app_root: &str) -> Result<(), WorkerError> {
        let mut to_respawn = Vec::new();

        for (idx, worker) in self.workers_mut().iter_mut().enumerate() {
            if !worker.is_alive() {
                info!(index = idx, "Persistent worker thread exited — will respawn");
                to_respawn.push(idx);
            }
        }

        for idx in to_respawn {
            self.respawn_persistent_thread_at(idx, app_root.to_string())?;
        }

        Ok(())
    }

    fn respawn_persistent_thread_at(&mut self, index: usize, app_root: String) -> Result<(), WorkerError> {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

        static RESPAWN_THREAD_ID: AtomicU64 = AtomicU64::new(10000);

        let mut cmd_pipe = [0i32; 2];
        let mut resp_pipe = [0i32; 2];
        if unsafe { libc::pipe(cmd_pipe.as_mut_ptr()) } != 0 {
            return Err(WorkerError::Pipe(nix::Error::last()));
        }
        if unsafe { libc::pipe(resp_pipe.as_mut_ptr()) } != 0 {
            return Err(WorkerError::Pipe(nix::Error::last()));
        }
        let (cmd_read, cmd_write) = (cmd_pipe[0], cmd_pipe[1]);
        let (resp_read, resp_write) = (resp_pipe[0], resp_pipe[1]);

        let alive = Arc::new(AtomicBool::new(true));
        let alive_clone = alive.clone();
        let thread_id = RESPAWN_THREAD_ID.fetch_add(1, Ordering::Relaxed);

        std::thread::Builder::new()
            .name(format!("turbine-persistent-{index}"))
            .spawn(move || {
                let init_rc = unsafe { turbine_php_sys::turbine_thread_init() };
                if init_rc != 0 {
                    alive_clone.store(false, Ordering::Release);
                    unsafe {
                        libc::close(cmd_read);
                        libc::close(resp_write);
                    }
                    return;
                }
                worker_event_loop_persistent(cmd_read, resp_write, &app_root);
                unsafe { turbine_php_sys::turbine_thread_cleanup(); }
                unsafe {
                    libc::close(cmd_read);
                    libc::close(resp_write);
                }
                alive_clone.store(false, Ordering::Release);
            })
            .map_err(|_| WorkerError::Fork(nix::Error::ENOMEM))?;

        let max_req = self.config().max_requests;
        let worker = Worker::new_thread(alive, thread_id, max_req, cmd_write, resp_read);
        self.replace_worker(index, worker);

        // Read the ready signal from the respawned thread worker.
        match read_ready_signal(resp_read) {
            Ok(true) => {
                info!(index = index, thread_id = thread_id, "Persistent thread worker respawned and ready");
            }
            Ok(false) => {
                warn!(index = index, "Respawned persistent thread worker bootstrap failed");
            }
            Err(e) => {
                error!(index = index, error = %e, "Failed to read ready signal from respawned thread worker");
            }
        }
        info!(thread_id = thread_id, index = index, "Persistent worker thread respawned");
        Ok(())
    }
}
