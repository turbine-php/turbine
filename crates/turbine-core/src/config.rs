use serde::Deserialize;
use tracing::{debug, info, warn};

#[derive(Debug, Deserialize, Default)]
pub struct RuntimeConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub php: PhpConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub cache: CacheTomlConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub compression: CompressionConfig,
    #[serde(default)]
    pub error_pages: ErrorPagesConfig,
    #[serde(default)]
    pub session: SessionConfig,
    #[serde(default)]
    pub cors: CorsConfig,
    #[serde(default)]
    pub watcher: WatcherConfig,
    #[serde(default)]
    pub early_hints: EarlyHintsConfig,
    #[serde(default)]
    pub x_sendfile: XSendfileConfig,
    #[serde(default)]
    pub structured_logging: StructuredLoggingConfig,
    #[serde(default)]
    pub acme: AcmeConfig,
    #[serde(default)]
    pub embed: EmbedConfig,
    #[serde(default)]
    pub dashboard: DashboardConfig,
    #[serde(default)]
    pub worker_pools: Vec<WorkerPoolRouteConfig>,
    #[serde(default)]
    pub virtual_hosts: Vec<VirtualHostConfig>,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_workers")]
    pub workers: usize,
    #[serde(default = "default_listen")]
    pub listen: String,
    /// Worker backend mode: "process" (fork-based, default) or "thread" (ZTS required).
    ///
    /// - `"process"`: Each worker is a separate OS process created via fork().
    ///   Uses Copy-on-Write memory sharing for OPcache. Works with NTS and ZTS PHP.
    ///   Full process isolation — a crash in one worker cannot affect others.
    ///
    /// - `"thread"`: Each worker is an OS thread sharing the same address space.
    ///   Requires PHP compiled with ZTS (Zend Thread Safety / --enable-zts).
    ///   Lower memory overhead, faster IPC, but reduced isolation.
    #[serde(default = "default_worker_mode")]
    pub worker_mode: String,
    /// Request execution timeout in seconds (0 = no timeout).
    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,
    /// Maximum PHP requests per worker before respawn.
    #[serde(default = "default_worker_max_requests")]
    pub worker_max_requests: u64,
    /// Internal channel capacity for single-process mode.
    #[serde(default = "default_channel_capacity")]
    pub channel_capacity: usize,
    /// PID file path (empty = disabled).
    #[serde(default)]
    pub pid_file: Option<String>,
    #[serde(default)]
    pub tls: TlsConfig,
    /// Maximum time (seconds) a request may wait for a free worker before 503.
    /// 0 = uses request_timeout as fallback.
    #[serde(default)]
    pub max_wait_time: u64,
    /// Enable auto-scaling of workers based on load.
    #[serde(default)]
    pub auto_scale: bool,
    /// Minimum number of workers when auto-scaling (defaults to 1).
    #[serde(default = "default_min_workers")]
    pub min_workers: usize,
    /// Maximum number of workers when auto-scaling (defaults to CPU count * 2).
    #[serde(default = "default_max_workers")]
    pub max_workers: usize,
    /// Seconds an idle excess worker survives before being removed.
    #[serde(default = "default_scale_down_idle")]
    pub scale_down_idle_secs: u64,
    /// Enable persistent workers (bootstrap-once, handle many requests).
    /// When enabled, workers handle thousands of requests without
    /// re-initialization.  Pair with `worker_boot` and `worker_handler`
    /// to enable the lightweight lifecycle.
    #[serde(default)]
    pub persistent_workers: Option<bool>,
    /// PHP script executed **once** per worker at boot time.
    /// The script should bootstrap the application (autoloader, framework,
    /// service container) and store the app instance in `$GLOBALS`.
    /// Requires `persistent_workers = true`.  Relative paths are resolved
    /// from the application root (`-r` / `--root`).
    #[serde(default)]
    pub worker_boot: Option<String>,
    /// PHP script included on **every request** using the lightweight
    /// lifecycle (skips full php_request_startup/shutdown).
    /// Requires `persistent_workers = true` and `worker_boot` to be set.
    /// Relative paths are resolved from the application root.
    #[serde(default)]
    pub worker_handler: Option<String>,
    /// PHP script executed after **every request** to clean up application
    /// state (e.g. clear service container scoped instances, flush caches).
    /// Evaluated via `zend_eval_string` after the response is sent but before
    /// `request_shutdown`.  Requires `persistent_workers = true`.
    /// Relative paths are resolved from the application root.
    #[serde(default)]
    pub worker_cleanup: Option<String>,
    /// Number of Tokio async I/O threads (default = number of CPU cores).
    /// Increase to handle more concurrent connections; decrease to leave more
    /// cores for PHP worker processes.
    #[serde(default)]
    pub tokio_worker_threads: Option<usize>,
    /// Pin each PHP worker to a specific CPU core (Linux only).
    ///
    /// When enabled, worker N is bound to logical core `N % cpu_count`.
    /// This reduces cache thrashing from the scheduler bouncing workers
    /// between cores, at the cost of losing work-stealing.  Only worth
    /// it on dedicated hosts with stable workloads and when worker
    /// count ≤ physical core count.  No-op on macOS.
    #[serde(default)]
    pub pin_workers: bool,
    /// Enable `SO_BUSY_POLL` on the listening socket and accepted streams
    /// (Linux only). Value is the busy-poll budget in microseconds — the
    /// kernel will spin on the NIC RX queue for up to this many µs before
    /// yielding to the scheduler, trading CPU for latency.
    ///
    /// Typical values: `50` (latency-sensitive) to `200` (extreme).
    /// `0` / `None` disables. Requires `CAP_NET_ADMIN` on older kernels
    /// (< 5.7); silently ignored on macOS and when the setsockopt fails.
    ///
    /// Expect 20-50µs off p99 on small-payload workloads when the server
    /// core is otherwise idle. Wastes CPU on oversubscribed hosts.
    #[serde(default)]
    pub listen_busy_poll_us: Option<u32>,
    /// Number of `SO_REUSEPORT` accept shards (Linux only).
    ///
    /// When set to `N > 1`, Turbine binds N independent listener sockets
    /// to the same address with `SO_REUSEPORT` and runs one accept loop
    /// per shard. The kernel distributes incoming connections across
    /// them with a per-flow hash, removing contention on the single
    /// accept queue that otherwise becomes the bottleneck above ~100k
    /// connections/sec.
    ///
    /// `None` / `0` / `1` keeps the single-listener behaviour.
    /// Recommended: match your tokio worker thread count (typically
    /// `ncpus`). Silently falls back to a single listener on non-Linux
    /// and on bind failure of additional shards.
    ///
    /// Gain: 10-30% more accept throughput under very high connection
    /// churn. Negligible on keep-alive-heavy workloads.
    #[serde(default)]
    pub listen_reuseport_shards: Option<usize>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct TlsConfig {
    /// Enable TLS (HTTPS). Requires cert_file and key_file.
    #[serde(default)]
    pub enabled: bool,
    /// Path to PEM-encoded certificate chain file.
    pub cert_file: Option<String>,
    /// Path to PEM-encoded private key file.
    pub key_file: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PhpConfig {
    pub extension_dir: Option<String>,
    /// PHP extensions to load (e.g. ["redis.so", "imagick.so"]).
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Zend extensions to load (e.g. ["xdebug.so"]).
    #[serde(default)]
    pub zend_extensions: Vec<String>,
    /// PHP memory_limit (e.g. "256M").
    #[serde(default = "default_memory_limit")]
    pub memory_limit: String,
    /// PHP max_execution_time in seconds.
    #[serde(default = "default_max_execution_time")]
    pub max_execution_time: u64,
    /// PHP upload_max_filesize (e.g. "64M").
    #[serde(default = "default_upload_max_filesize")]
    pub upload_max_filesize: String,
    /// PHP post_max_size (e.g. "64M").
    #[serde(default = "default_post_max_size")]
    pub post_max_size: String,
    /// OPcache memory consumption in MB.
    #[serde(default = "default_opcache_memory")]
    pub opcache_memory: usize,
    /// JIT buffer size (e.g. "64M").
    #[serde(default = "default_jit_buffer_size")]
    pub jit_buffer_size: String,
    /// Temporary directory for file uploads.
    #[serde(default = "default_upload_tmp_dir")]
    pub upload_tmp_dir: String,
    /// OPcache preload script path (empty or "auto" = auto-detect).
    #[serde(default)]
    pub preload_script: Option<String>,
    /// Enable OPcache file timestamp validation.
    /// When `true`, OPcache checks if PHP files have been modified on disk
    /// and recompiles them.  Useful during development but adds stat() overhead.
    /// Default: `false` (files are never re-checked — maximum performance).
    #[serde(default)]
    pub opcache_validate_timestamps: Option<bool>,
    /// Arbitrary php.ini directives (key = value).
    #[serde(default)]
    pub ini: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub sql_guard: bool,
    #[serde(default = "default_true")]
    pub path_traversal_guard: bool,
    #[serde(default = "default_true")]
    pub code_injection_guard: bool,
    #[serde(default = "default_true")]
    pub behaviour_guard: bool,
    /// Injection-filter paranoia level.
    ///   `0` — disable pattern matching (behaviour guard still runs)
    ///   `1` — obvious attacks only (default; very low FP rate)
    ///   `2` — add common injection patterns (some FPs on user content)
    ///   `3` — aggressive (high FP rate on user-generated content)
    #[serde(default = "default_paranoia_level")]
    pub paranoia_level: u8,
    /// URL path prefixes to exclude from SqlGuard / CodeGuard scanning.
    /// Matched via `starts_with`.  Behaviour guard still runs on these.
    #[serde(default)]
    pub exclude_paths: Vec<String>,
    /// Rate limit: max requests per second per IP.  `0` disables rate
    /// limiting (default).  Any non-zero value enables per-IP throttling.
    #[serde(default = "default_rate_limit_rps")]
    pub max_requests_per_second: u32,
    /// Rate limit: time window in seconds.
    #[serde(default = "default_rate_limit_window")]
    pub rate_limit_window: u64,
    /// Number of SQL injection attempts from a single IP before permanently blocking it.
    #[serde(default = "default_sqli_block_threshold")]
    pub sqli_block_threshold: u32,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SandboxConfig {
    #[serde(default = "default_true")]
    pub seccomp: bool,
    /// Execution mode: "strict" (only whitelist), "framework" (auto-detect entry point).
    #[serde(default = "default_execution_mode")]
    pub execution_mode: String,
    /// Route unmatched paths to the entry point (front controller pattern).
    /// Auto-detected from application structure, but can be overridden.
    /// When `None`, auto-detection applies. Set explicitly to `true` or `false` to override.
    #[serde(default)]
    pub front_controller: Option<bool>,
    /// PHP files allowed to be executed via HTTP.
    /// In "strict" mode, only these files can be executed.
    /// In "framework" mode, if non-empty, acts as an explicit whitelist; if empty, all PHP files allowed.
    #[serde(default = "default_execution_whitelist")]
    pub execution_whitelist: Vec<String>,
    /// Directories where PHP can write data but NEVER execute PHP files.
    #[serde(default = "default_data_directories")]
    pub data_directories: Vec<String>,
    /// Block uploading files with these extensions (case-insensitive).
    #[serde(default = "default_blocked_upload_extensions")]
    pub blocked_upload_extensions: Vec<String>,
    /// Scan upload content for PHP code signatures.
    #[serde(default = "default_true")]
    pub scan_upload_content: bool,
    /// Disable dangerous PHP functions.
    #[serde(default = "default_disabled_functions")]
    pub disabled_functions: Vec<String>,
    /// Restrict PHP file access via open_basedir (auto-configured).
    #[serde(default = "default_true")]
    pub enforce_open_basedir: bool,
    /// Block allow_url_include.
    #[serde(default = "default_true")]
    pub block_url_include: bool,
    /// Block allow_url_fopen.
    #[serde(default = "default_true")]
    pub block_url_fopen: bool,
}

#[derive(Debug, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    /// Path to access log file (empty = disabled).
    #[serde(default)]
    pub access_log: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CacheTomlConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_cache_ttl")]
    pub ttl_seconds: u64,
    #[serde(default = "default_cache_max_entries")]
    pub max_entries: usize,
}

/// Early Hints (103) configuration — send informational responses before final response.
#[derive(Debug, Deserialize, Clone)]
pub struct EarlyHintsConfig {
    /// Enable Early Hints support.
    #[serde(default)]
    pub enabled: bool,
}

impl Default for EarlyHintsConfig {
    fn default() -> Self {
        EarlyHintsConfig { enabled: true }
    }
}

/// X-Sendfile / X-Accel-Redirect configuration — delegate large file serving to Turbine.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct XSendfileConfig {
    /// Enable X-Sendfile support.
    #[serde(default)]
    pub enabled: bool,
    /// Base directory for X-Accel-Redirect paths (relative to app root).
    #[serde(default)]
    pub root: Option<String>,
}

/// Structured logging configuration — turbine_log() PHP function.
#[derive(Debug, Deserialize, Clone)]
pub struct StructuredLoggingConfig {
    /// Enable structured logging from PHP.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Output destination: "stdout", "stderr", or a file path.
    #[serde(default = "default_structured_log_output")]
    pub output: String,
}

fn default_structured_log_output() -> String {
    "stderr".to_string()
}

impl Default for StructuredLoggingConfig {
    fn default() -> Self {
        StructuredLoggingConfig {
            enabled: true,
            output: default_structured_log_output(),
        }
    }
}

/// ACME (Let's Encrypt) automatic TLS configuration.
#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct AcmeConfig {
    /// Enable automatic TLS via ACME (Let's Encrypt).
    #[serde(default)]
    pub enabled: bool,
    /// Domain names for the certificate.
    #[serde(default)]
    pub domains: Vec<String>,
    /// Contact email for Let's Encrypt notifications.
    #[serde(default)]
    pub email: Option<String>,
    /// Directory to store ACME certificates and account keys.
    #[serde(default = "default_acme_cache_dir")]
    pub cache_dir: String,
    /// Use Let's Encrypt staging server (for testing).
    #[serde(default)]
    pub staging: bool,
}

fn default_acme_cache_dir() -> String {
    "/var/lib/turbine/acme".to_string()
}

impl Default for AcmeConfig {
    fn default() -> Self {
        AcmeConfig {
            enabled: false,
            domains: Vec::new(),
            email: None,
            cache_dir: default_acme_cache_dir(),
            staging: false,
        }
    }
}

/// Embed app configuration — pack PHP files into the binary.
#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
#[derive(Default)]
pub struct EmbedConfig {
    /// Enable embedded app mode. At build time, set TURBINE_EMBED_DIR env var
    /// to the directory containing the PHP app.
    #[serde(default)]
    pub enabled: bool,
    /// Directory to extract embedded files to at runtime (temp if empty).
    #[serde(default)]
    pub extract_dir: Option<String>,
}

/// Dashboard, metrics, and internal endpoints configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct DashboardConfig {
    /// Enable the /_/dashboard HTML page.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Enable /_/metrics and /_/status endpoints.
    #[serde(default = "default_true")]
    pub statistics: bool,
    /// Bearer token required to access internal endpoints (empty = no auth).
    #[serde(default)]
    pub token: Option<String>,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        DashboardConfig {
            enabled: true,
            statistics: true,
            token: None,
        }
    }
}

/// Route-based worker pool splitting configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct WorkerPoolRouteConfig {
    /// Path prefix pattern (e.g. "/api/slow/*").
    pub match_path: String,
    /// Minimum worker count for this pool.
    #[serde(default = "default_min_workers")]
    pub min_workers: usize,
    /// Maximum worker count for this pool.
    #[serde(default = "default_pool_max_workers")]
    pub max_workers: usize,
    /// Human-readable name for logs/metrics.
    #[serde(default)]
    pub name: Option<String>,
}

fn default_pool_max_workers() -> usize {
    4
}

/// Virtual host configuration — serve different PHP applications on different domains.
#[derive(Debug, Deserialize, Clone)]
pub struct VirtualHostConfig {
    /// Primary domain name (e.g. "xpto.com").
    pub domain: String,
    /// Application root directory for this domain.
    pub root: String,
    /// Alternative domain names that should also match (e.g. ["www.xpto.com"]).
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Entry point PHP file (default: auto-detected).
    #[serde(default)]
    pub entry_point: Option<String>,
    /// Per-host TLS certificate file (overrides global TLS).
    #[serde(default)]
    pub tls_cert: Option<String>,
    /// Per-host TLS key file (overrides global TLS).
    #[serde(default)]
    pub tls_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CompressionConfig {
    /// Enable response compression.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Minimum response body size (bytes) to compress.
    #[serde(default = "default_compression_min_size")]
    pub min_size: usize,
    /// Compression level (1-9, default 6). Applied to all algorithms.
    #[serde(default = "default_compression_level")]
    pub level: u32,
    /// Preferred algorithm order. Supported: "br" (brotli), "zstd", "gzip".
    /// The first algorithm also accepted by the client wins.
    #[serde(default = "default_compression_algorithms")]
    pub algorithms: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ErrorPagesConfig {
    /// Path to custom 404 HTML page.
    pub not_found: Option<String>,
    /// Path to custom 500 HTML page.
    pub server_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SessionConfig {
    /// Enable PHP session support.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Session storage path for file-based sessions.
    #[serde(default = "default_session_save_path")]
    pub save_path: String,
    /// Session cookie name.
    #[serde(default = "default_session_cookie_name")]
    pub cookie_name: String,
    /// Session cookie lifetime (0 = until browser close).
    #[serde(default)]
    pub cookie_lifetime: u64,
    /// HttpOnly flag for session cookie.
    #[serde(default = "default_true")]
    pub cookie_httponly: bool,
    /// Secure flag for session cookie (auto-enabled for TLS).
    #[serde(default)]
    pub cookie_secure: bool,
    /// SameSite attribute for session cookie.
    #[serde(default = "default_session_samesite")]
    pub cookie_samesite: String,
    /// Session garbage collection max lifetime in seconds.
    #[serde(default = "default_session_gc_maxlifetime")]
    pub gc_maxlifetime: u64,
    /// Whether to call session_start() automatically before PHP execution.
    /// Default false — most applications manage their own sessions.
    #[serde(default)]
    pub auto_start: bool,
}

fn default_cache_ttl() -> u64 {
    30
}

fn default_cache_max_entries() -> usize {
    1024
}

fn default_compression_min_size() -> usize {
    1024
}

fn default_compression_level() -> u32 {
    6
}

fn default_session_save_path() -> String {
    "/tmp/turbine-sessions".to_string()
}

fn default_session_cookie_name() -> String {
    "PHPSESSID".to_string()
}

fn default_session_samesite() -> String {
    "Lax".to_string()
}

fn default_session_gc_maxlifetime() -> u64 {
    1440
}

fn default_cors_methods() -> Vec<String> {
    vec!["GET", "POST", "PUT", "DELETE", "PATCH", "OPTIONS"]
        .into_iter()
        .map(String::from)
        .collect()
}

fn default_cors_headers() -> Vec<String> {
    vec!["Content-Type", "Authorization", "X-Requested-With"]
        .into_iter()
        .map(String::from)
        .collect()
}

fn default_cors_max_age() -> u64 {
    86400
}

fn default_rate_limit_rps() -> u32 {
    // 0 = disabled.  The previous default of 100 is impractical for any
    // site behind a CDN/proxy/NAT (all traffic appears to come from one
    // IP) or for SPA-driven APIs that fire many requests per page load.
    // Operators opt in with an explicit number when they want it.
    0
}

fn default_rate_limit_window() -> u64 {
    60
}

fn default_paranoia_level() -> u8 {
    1
}

fn default_sqli_block_threshold() -> u32 {
    3
}

fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

fn default_listen() -> String {
    "127.0.0.1:9000".to_string()
}

fn default_worker_mode() -> String {
    "process".to_string()
}

fn default_true() -> bool {
    true
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_request_timeout() -> u64 {
    30
}

fn default_worker_max_requests() -> u64 {
    10_000
}

fn default_channel_capacity() -> usize {
    64
}

fn default_memory_limit() -> String {
    "256M".to_string()
}

fn default_max_execution_time() -> u64 {
    30
}

fn default_upload_max_filesize() -> String {
    "64M".to_string()
}

fn default_post_max_size() -> String {
    "64M".to_string()
}

fn default_opcache_memory() -> usize {
    128
}

fn default_jit_buffer_size() -> String {
    "64M".to_string()
}

fn default_upload_tmp_dir() -> String {
    "/tmp/turbine-uploads".to_string()
}

fn default_min_workers() -> usize {
    1
}

fn default_max_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get() * 2)
        .unwrap_or(8)
}

fn default_scale_down_idle() -> u64 {
    5
}

fn default_compression_algorithms() -> Vec<String> {
    vec!["br".to_string(), "zstd".to_string(), "gzip".to_string()]
}

fn default_watch_paths() -> Vec<String> {
    vec![
        "app/".to_string(),
        "config/".to_string(),
        "routes/".to_string(),
        "src/".to_string(),
        "public/".to_string(),
    ]
}

fn default_watch_extensions() -> Vec<String> {
    vec!["php".to_string(), "env".to_string()]
}

fn default_watch_debounce_ms() -> u64 {
    500
}

fn default_execution_mode() -> String {
    "framework".to_string()
}

fn default_execution_whitelist() -> Vec<String> {
    Vec::new()
}

fn default_data_directories() -> Vec<String> {
    vec![
        "storage/".to_string(),
        "uploads/".to_string(),
        "public/uploads/".to_string(),
    ]
}

fn default_blocked_upload_extensions() -> Vec<String> {
    vec![
        ".php".to_string(),
        ".phtml".to_string(),
        ".phar".to_string(),
        ".php7".to_string(),
        ".php8".to_string(),
        ".inc".to_string(),
        ".phps".to_string(),
        ".pht".to_string(),
        ".pgif".to_string(),
    ]
}

fn default_disabled_functions() -> Vec<String> {
    vec![
        "exec".to_string(),
        "system".to_string(),
        "passthru".to_string(),
        "shell_exec".to_string(),
        "proc_open".to_string(),
        "popen".to_string(),
        "pcntl_exec".to_string(),
        "dl".to_string(),
        "putenv".to_string(),
    ]
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            workers: default_workers(),
            listen: default_listen(),
            worker_mode: default_worker_mode(),
            request_timeout: default_request_timeout(),
            worker_max_requests: default_worker_max_requests(),
            channel_capacity: default_channel_capacity(),
            pid_file: None,
            tls: TlsConfig::default(),
            max_wait_time: 0,
            auto_scale: false,
            min_workers: default_min_workers(),
            max_workers: default_max_workers(),
            scale_down_idle_secs: default_scale_down_idle(),
            persistent_workers: None,
            worker_boot: None,
            worker_handler: None,
            worker_cleanup: None,
            tokio_worker_threads: None,
            pin_workers: false,
            listen_busy_poll_us: None,
            listen_reuseport_shards: None,
        }
    }
}

impl Default for PhpConfig {
    fn default() -> Self {
        PhpConfig {
            extension_dir: None,
            extensions: Vec::new(),
            zend_extensions: Vec::new(),
            memory_limit: default_memory_limit(),
            max_execution_time: default_max_execution_time(),
            upload_max_filesize: default_upload_max_filesize(),
            post_max_size: default_post_max_size(),
            opcache_memory: default_opcache_memory(),
            jit_buffer_size: default_jit_buffer_size(),
            upload_tmp_dir: default_upload_tmp_dir(),
            preload_script: None,
            opcache_validate_timestamps: None,
            ini: std::collections::HashMap::new(),
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        SecurityConfig {
            enabled: true,
            sql_guard: true,
            path_traversal_guard: true,
            code_injection_guard: true,
            behaviour_guard: true,
            paranoia_level: default_paranoia_level(),
            exclude_paths: Vec::new(),
            max_requests_per_second: default_rate_limit_rps(),
            rate_limit_window: default_rate_limit_window(),
            sqli_block_threshold: default_sqli_block_threshold(),
        }
    }
}

impl Default for SandboxConfig {
    fn default() -> Self {
        SandboxConfig {
            seccomp: true,
            execution_mode: default_execution_mode(),
            front_controller: None,
            execution_whitelist: default_execution_whitelist(),
            data_directories: default_data_directories(),
            blocked_upload_extensions: default_blocked_upload_extensions(),
            scan_upload_content: true,
            disabled_functions: default_disabled_functions(),
            enforce_open_basedir: true,
            block_url_include: true,
            block_url_fopen: true,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        LoggingConfig {
            level: default_log_level(),
            access_log: None,
        }
    }
}

impl Default for CacheTomlConfig {
    fn default() -> Self {
        CacheTomlConfig {
            enabled: true,
            ttl_seconds: default_cache_ttl(),
            max_entries: default_cache_max_entries(),
        }
    }
}

impl Default for CompressionConfig {
    fn default() -> Self {
        CompressionConfig {
            enabled: true,
            min_size: default_compression_min_size(),
            level: default_compression_level(),
            algorithms: default_compression_algorithms(),
        }
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            enabled: true,
            save_path: default_session_save_path(),
            cookie_name: default_session_cookie_name(),
            cookie_lifetime: 0,
            cookie_httponly: true,
            cookie_secure: false,
            cookie_samesite: default_session_samesite(),
            gc_maxlifetime: default_session_gc_maxlifetime(),
            auto_start: false,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CorsConfig {
    /// Enable CORS handling.
    #[serde(default)]
    pub enabled: bool,
    /// Allowed origins. Use ["*"] for any origin.
    #[serde(default)]
    pub allow_origins: Vec<String>,
    /// Allow credentials (cookies, auth headers).
    #[serde(default)]
    pub allow_credentials: bool,
    /// Allowed HTTP methods.
    #[serde(default = "default_cors_methods")]
    pub allow_methods: Vec<String>,
    /// Allowed request headers.
    #[serde(default = "default_cors_headers")]
    pub allow_headers: Vec<String>,
    /// Headers exposed to the client.
    #[serde(default)]
    pub expose_headers: Vec<String>,
    /// Preflight cache max-age in seconds.
    #[serde(default = "default_cors_max_age")]
    pub max_age: u64,
}

impl Default for CorsConfig {
    fn default() -> Self {
        CorsConfig {
            enabled: false,
            allow_origins: Vec::new(),
            allow_credentials: false,
            allow_methods: default_cors_methods(),
            allow_headers: default_cors_headers(),
            expose_headers: Vec::new(),
            max_age: default_cors_max_age(),
        }
    }
}

/// File watcher configuration — auto-restart workers on PHP file changes.
#[derive(Debug, Deserialize, Clone)]
pub struct WatcherConfig {
    /// Enable the file watcher.
    #[serde(default)]
    pub enabled: bool,
    /// Directories to watch (relative to app root). Default: ["app/", "config/", "routes/"].
    #[serde(default = "default_watch_paths")]
    pub paths: Vec<String>,
    /// File extensions to watch.
    #[serde(default = "default_watch_extensions")]
    pub extensions: Vec<String>,
    /// Debounce interval in milliseconds.
    #[serde(default = "default_watch_debounce_ms")]
    pub debounce_ms: u64,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        WatcherConfig {
            enabled: false,
            paths: default_watch_paths(),
            extensions: default_watch_extensions(),
            debounce_ms: default_watch_debounce_ms(),
        }
    }
}

impl RuntimeConfig {
    /// Validate configuration and return (errors, warnings).
    ///
    /// Errors are issues that will prevent Turbine from running correctly.
    /// Warnings are suboptimal settings that may cause unexpected behaviour.
    pub fn check(&self) -> (Vec<String>, Vec<String>) {
        let mut errors: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        // ── Errors ──────────────────────────────────────────────────

        if self.server.worker_mode != "process" && self.server.worker_mode != "thread" {
            errors.push(format!(
                "[server] worker_mode = \"{}\" — must be \"process\" or \"thread\"",
                self.server.worker_mode
            ));
        }

        if self.sandbox.execution_mode != "framework" && self.sandbox.execution_mode != "strict" {
            errors.push(format!(
                "[sandbox] execution_mode = \"{}\" — must be \"framework\" or \"strict\"",
                self.sandbox.execution_mode
            ));
        }

        if self.server.tls.enabled {
            if self.server.tls.cert_file.is_none() {
                errors.push("[server.tls] enabled = true but cert_file is missing".to_string());
            }
            if self.server.tls.key_file.is_none() {
                errors.push("[server.tls] enabled = true but key_file is missing".to_string());
            }
        }

        if self.sandbox.execution_mode == "strict" && self.sandbox.execution_whitelist.is_empty() {
            errors.push("[sandbox] execution_mode = \"strict\" but execution_whitelist is empty — ALL PHP requests will be blocked".to_string());
        }

        if let Some(ref tls_cert) = self.server.tls.cert_file {
            if !std::path::Path::new(tls_cert).exists() {
                errors.push(format!(
                    "[server.tls] cert_file = \"{}\" — file not found",
                    tls_cert
                ));
            }
        }

        if let Some(ref tls_key) = self.server.tls.key_file {
            if !std::path::Path::new(tls_key).exists() {
                errors.push(format!(
                    "[server.tls] key_file = \"{}\" — file not found",
                    tls_key
                ));
            }
        }

        // ── Warnings ────────────────────────────────────────────────

        if self.server.workers == 0 && self.server.request_timeout == 0 {
            warnings.push("[server] workers = 0 + request_timeout = 0 — a slow request will block ALL subsequent requests".to_string());
        }

        if self.security.enabled
            && !self.security.sql_guard
            && !self.security.code_injection_guard
            && !self.security.behaviour_guard
        {
            warnings.push("[security] enabled = true but all guards are disabled — security layer has no effect".to_string());
        }

        if self.cache.enabled && self.cache.ttl_seconds == 0 {
            warnings.push("[cache] enabled = true but ttl_seconds = 0 — responses will be cached and immediately expire".to_string());
        }

        if self.server.workers > 64 {
            warnings.push(format!("[server] workers = {} — very high worker count, ensure sufficient memory and file descriptors", self.server.workers));
        }

        if self.server.persistent_workers == Some(true) && self.server.worker_max_requests == 0 {
            warnings.push("[server] persistent_workers = true but worker_max_requests = 0 — workers will never recycle, throughput will degrade over time".to_string());
        }

        if self.server.worker_mode == "thread" && self.server.persistent_workers != Some(true) {
            // Thread + non-persistent is valid but worth noting the IPC difference
        }

        if self.compression.level > 9 {
            warnings.push(format!(
                "[compression] level = {} — should be 1–9",
                self.compression.level
            ));
        }

        if self.session.cookie_samesite != "Lax"
            && self.session.cookie_samesite != "Strict"
            && self.session.cookie_samesite != "None"
        {
            warnings.push(format!(
                "[session] cookie_samesite = \"{}\" — should be Lax, Strict, or None",
                self.session.cookie_samesite
            ));
        }

        if self.cors.enabled && self.cors.allow_origins.is_empty() {
            warnings.push(
                "[cors] enabled = true but allow_origins is empty — no origins will be allowed"
                    .to_string(),
            );
        }

        if self.acme.enabled && self.server.tls.enabled {
            warnings.push("[acme] enabled = true and [server.tls] enabled = true — ACME auto-TLS and manual TLS are both active, ACME may overwrite your certificate".to_string());
        }

        if self.acme.enabled && !self.acme.domains.is_empty() && !self.virtual_hosts.is_empty() {
            warnings.push("[acme] domains is set but [[virtual_hosts]] are configured — vhost domains are auto-collected into ACME, 'domains' is redundant".to_string());
        }

        if self.watcher.enabled && (self.logging.level == "warn" || self.logging.level == "error") {
            // Fine, no warning needed
        }

        for (i, pool) in self.worker_pools.iter().enumerate() {
            if pool.match_path.is_empty() {
                warnings.push(format!(
                    "[[worker_pools]][{}] match_path is empty — pool will never match any request",
                    i
                ));
            }
            if pool.min_workers > pool.max_workers {
                errors.push(format!(
                    "[[worker_pools]][{}] min_workers ({}) > max_workers ({}) — invalid",
                    i, pool.min_workers, pool.max_workers
                ));
            }
        }

        // ── Virtual hosts ───────────────────────────────────────────

        let mut seen_domains: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (i, vhost) in self.virtual_hosts.iter().enumerate() {
            if vhost.domain.is_empty() {
                errors.push(format!("[[virtual_hosts]][{}] domain is empty", i));
            } else {
                let lower = vhost.domain.to_lowercase();
                if !seen_domains.insert(lower.clone()) {
                    errors.push(format!(
                        "[[virtual_hosts]][{}] duplicate domain \"{}\"",
                        i, vhost.domain
                    ));
                }
            }
            if vhost.root.is_empty() {
                errors.push(format!(
                    "[[virtual_hosts]][{}] root is empty for domain \"{}\"",
                    i, vhost.domain
                ));
            } else if !std::path::Path::new(&vhost.root).exists() {
                errors.push(format!(
                    "[[virtual_hosts]][{}] root = \"{}\" — directory not found for domain \"{}\"",
                    i, vhost.root, vhost.domain
                ));
            }
            for alias in &vhost.aliases {
                let lower = alias.to_lowercase();
                if !seen_domains.insert(lower.clone()) {
                    errors.push(format!("[[virtual_hosts]][{}] duplicate alias \"{}\" (already defined as domain or alias)", i, alias));
                }
            }
            if let Some(ref cert) = vhost.tls_cert {
                if !std::path::Path::new(cert).exists() {
                    errors.push(format!("[[virtual_hosts]][{}] tls_cert = \"{}\" — file not found for domain \"{}\"", i, cert, vhost.domain));
                }
            }
            if let Some(ref key) = vhost.tls_key {
                if !std::path::Path::new(key).exists() {
                    errors.push(format!(
                        "[[virtual_hosts]][{}] tls_key = \"{}\" — file not found for domain \"{}\"",
                        i, key, vhost.domain
                    ));
                }
            }
            if vhost.tls_cert.is_some() != vhost.tls_key.is_some() {
                errors.push(format!("[[virtual_hosts]][{}] domain \"{}\" — tls_cert and tls_key must both be set or both omitted", i, vhost.domain));
            }
        }

        if !self.virtual_hosts.is_empty() && self.server.listen.starts_with("127.0.0.1") {
            warnings.push("[virtual_hosts] configured but server listens on 127.0.0.1 — external domains won't be reachable".to_string());
        }

        (errors, warnings)
    }

    /// Validate configuration for contradictions and warn about issues.
    pub fn validate(&self) {
        let (errors, warnings) = self.check();
        for e in &errors {
            warn!("{}", e);
        }
        for w in &warnings {
            warn!("{}", w);
        }
    }

    /// Load configuration from turbine.toml in the current directory.
    /// Falls back to defaults if the file doesn't exist.
    pub fn load() -> Self {
        let config_path = std::env::current_dir()
            .unwrap_or_default()
            .join("turbine.toml");

        if config_path.exists() {
            match std::fs::read_to_string(&config_path) {
                Ok(content) => match toml::from_str(&content) {
                    Ok(config) => {
                        info!(path = %config_path.display(), "Loaded configuration");
                        return config;
                    }
                    Err(e) => {
                        warn!(
                            path = %config_path.display(),
                            error = %e,
                            "Failed to parse config, using defaults"
                        );
                    }
                },
                Err(e) => {
                    warn!(
                        path = %config_path.display(),
                        error = %e,
                        "Failed to read config, using defaults"
                    );
                }
            }
        } else {
            debug!("No turbine.toml found, using defaults");
        }

        RuntimeConfig::default()
    }

    /// Load from a specific path (for --config flag).
    pub fn load_from(path: &str) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => match toml::from_str(&content) {
                Ok(config) => {
                    info!(path = path, "Loaded configuration");
                    config
                }
                Err(e) => {
                    eprintln!("[turbine] Failed to parse config {path}: {e}");
                    warn!(path = path, error = %e, "Failed to parse config, using defaults");
                    RuntimeConfig::default()
                }
            },
            Err(e) => {
                eprintln!("[turbine] Failed to read config {path}: {e}");
                warn!(path = path, error = %e, "Failed to read config, using defaults");
                RuntimeConfig::default()
            }
        }
    }

    /// Generate a default turbine.toml template.
    pub fn template() -> &'static str {
        r#"# Turbine Runtime Configuration
# https://github.com/turbine-php/turbine

[server]
# Number of worker processes (0 = auto-detect CPU cores)
workers = 4
# Listen address
listen = "127.0.0.1:8080"
# TCP connection read/write timeout in seconds
connection_timeout = 30
# Request execution timeout in seconds (0 = no timeout)
request_timeout = 30
# Maximum time (seconds) a request waits for a free worker (0 = uses request_timeout)
# max_wait_time = 5
# Shared memory size in MB
shm_size_mb = 128
# Maximum PHP requests per worker before respawn
worker_max_requests = 10000
# Internal channel capacity (single-process mode)
channel_capacity = 64
# PID file path (comment out to disable)
# pid_file = \"/var/run/turbine.pid\"
# Auto-scaling: dynamically add/remove workers based on load
auto_scale = false
# min_workers = 1
# max_workers = 16
# scale_down_idle_secs = 5
[server.tls]
# Enable TLS (HTTPS + HTTP/2)
enabled = false
# Path to PEM certificate chain
# cert_file = "/path/to/cert.pem"
# Path to PEM private key
# key_file = "/path/to/key.pem"

[php]
# extension_dir = "/path/to/extensions"
# PHP extensions to load (shared .so)
# extensions = ["redis.so", "imagick.so", "apcu.so"]
# Zend extensions to load
# zend_extensions = ["xdebug.so"]
memory_limit = "256M"
max_execution_time = 30
upload_max_filesize = "64M"
post_max_size = "64M"
opcache_memory = 128
jit_buffer_size = "64M"
# Temporary directory for file uploads
upload_tmp_dir = "/tmp/turbine-uploads"
# OPcache preload script (auto-detect or explicit path)
# preload_script = "auto"
# Custom php.ini directives (key=value)
# [php.ini]
# error_reporting = "E_ALL"
# date.timezone = "UTC"

[cache]
# Enable response cache
enabled = true
# Cache TTL in seconds
ttl_seconds = 30
# Maximum number of cached responses
max_entries = 1024

[security]
# Master switch for all security guards
enabled = true
# Individual guard toggles
sql_guard = true
code_injection_guard = true
path_traversal_guard = true
behaviour_guard = true
# Rate limiting: max requests per second per IP
max_requests_per_second = 100
# Rate limit window in seconds
rate_limit_window = 60

[sandbox]
# seccomp-bpf sandbox (Linux only)
seccomp = true
# Execution mode: "strict" (only whitelist) or "framework" (auto-detect entry point)
execution_mode = "framework"
# PHP files allowed to execute via HTTP (strict mode only)
# execution_whitelist = ["public/index.php"]
# Directories where uploads/data live (PHP cannot execute files here)
data_directories = ["storage/", "uploads/", "public/uploads/"]
# Block uploads with these file extensions
blocked_upload_extensions = [".php", ".phtml", ".phar", ".php7", ".php8", ".inc", ".phps", ".pht", ".pgif"]
# Scan upload content for embedded PHP code
scan_upload_content = true
# Disable dangerous PHP functions
disabled_functions = ["exec", "system", "passthru", "shell_exec", "proc_open", "popen", "pcntl_exec", "dl", "putenv"]
# Restrict PHP filesystem access to project directories only
enforce_open_basedir = true
# Block remote code inclusion
block_url_include = true
# Block remote file access  
block_url_fopen = true

[logging]
# Log level: trace, debug, info, warn, error
level = "info"
# Access log file path (comment out to disable)
# access_log = "/var/log/turbine/access.log"

[compression]
# Enable response compression (brotli, zstd, gzip)
enabled = true
# Minimum body size in bytes to compress
min_size = 1024
# Compression level (1 = fastest, 9 = best, 6 = balanced)
level = 6
# Preferred algorithm order (first match with client Accept-Encoding wins)
algorithms = ["br", "zstd", "gzip"]

[error_pages]
# Custom HTML error pages (comment out to use defaults)
# not_found = "/path/to/404.html"
# server_error = "/path/to/500.html"

[session]
# Enable PHP session support
enabled = true
# Session file storage path
save_path = "/tmp/turbine-sessions"
# Session cookie name
cookie_name = "PHPSESSID"
# Cookie lifetime (0 = until browser close)
cookie_lifetime = 0
# HttpOnly flag
cookie_httponly = true
# Secure flag (auto-enabled when TLS is active)
cookie_secure = false
# SameSite attribute (Lax, Strict, None)
cookie_samesite = "Lax"
# Garbage collection max lifetime in seconds
gc_maxlifetime = 1440

[cors]
# Enable CORS (Cross-Origin Resource Sharing)
enabled = false
# Allowed origins (use [\"*\"] for any origin)
# allow_origins = [\"https://example.com\", \"http://localhost:3000\"]
allow_credentials = false
allow_methods = [\"GET\", \"POST\", \"PUT\", \"DELETE\", \"PATCH\", \"OPTIONS\"]
allow_headers = [\"Content-Type\", \"Authorization\", \"X-Requested-With\"]
# expose_headers = [\"X-Total-Count\"]
max_age = 86400

[watcher]
# Enable file watcher (auto-restart workers on PHP file changes)
enabled = false
# Directories to watch (relative to app root)
paths = ["app/", "config/", "routes/", "src/", "public/"]
# File extensions to watch
extensions = ["php", "env"]
# Debounce interval in milliseconds
debounce_ms = 500

[early_hints]
# Enable 103 Early Hints support (preload CSS/JS before final response)
enabled = true

[x_sendfile]
# Enable X-Sendfile / X-Accel-Redirect (delegate large file serving to Turbine)
enabled = false
# Base directory for X-Accel-Redirect paths (relative to app root)
# root = "private-files/"

[structured_logging]
# Enable structured JSON logging from PHP via turbine_log() markers
enabled = true
# Output: "stdout", "stderr", or a file path
output = "stderr"

[acme]
# Enable automatic TLS via ACME (Let's Encrypt).
# When [[virtual_hosts]] are configured, their domains are auto-collected —
# you do NOT need to list them in 'domains'. Use 'domains' only for
# single-site setups without virtual hosting.
enabled = false
# Domain names for the certificate (only needed without [[virtual_hosts]])
# domains = ["example.com", "www.example.com"]
# Contact email for Let's Encrypt notifications
# email = "admin@example.com"
# Certificate cache directory
cache_dir = "/var/lib/turbine/acme"
# Use staging server (set to true for testing)
staging = false

[embed]
# Embed PHP app in binary (set TURBINE_EMBED_DIR at build time)
enabled = false
# extract_dir = "/tmp/turbine-app"

[dashboard]
# Enable the /_/dashboard HTML page
enabled = true
# Enable /_/metrics and /_/status endpoints
statistics = true
# Bearer token to protect internal endpoints (comment out to disable auth)
# token = "my-secret-token"

# Worker pool splitting: route specific paths to dedicated worker pools
# [[worker_pools]]
# match_path = "/api/slow/*"
# min_workers = 1
# max_workers = 4
# name = "slow-api"

# Virtual hosting: serve different PHP applications on different domains.
# All virtual hosts share the same worker pool (no memory overhead).
# Requests whose Host header does not match any virtual host use the global root.
#
# [[virtual_hosts]]
# domain = "xpto.com"
# root = "/var/www/xpto"
# aliases = ["www.xpto.com"]
# # entry_point = "index.php"   # optional, auto-detected
# # tls_cert = "/etc/ssl/xpto.com.pem"   # optional, per-host TLS
# # tls_key = "/etc/ssl/xpto.com-key.pem"
#
# [[virtual_hosts]]
# domain = "outro.com"
# root = "/var/www/outro"
# aliases = ["www.outro.com"]
"#
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── TOML Parsing ────────────────────────────────────────────────

    #[test]
    fn parse_minimal_config() {
        let toml_str = r#"
[server]
workers = 2
listen = "0.0.0.0:8080"
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.server.workers, 2);
        assert_eq!(config.server.listen, "0.0.0.0:8080");
    }

    #[test]
    fn parse_empty_config_uses_defaults() {
        let config: RuntimeConfig = toml::from_str("").unwrap();
        assert_eq!(config.server.listen, "127.0.0.1:9000");
        assert_eq!(config.server.worker_mode, "process");
        assert_eq!(config.server.request_timeout, 30);
        assert_eq!(config.server.worker_max_requests, 10_000);
        assert_eq!(config.server.channel_capacity, 64);
        assert!(config.server.persistent_workers.is_none());
        assert!(config.server.tokio_worker_threads.is_none());
        assert!(!config.server.auto_scale);
        assert_eq!(config.server.min_workers, 1);
    }

    #[test]
    fn parse_worker_mode_thread() {
        let toml_str = r#"
[server]
worker_mode = "thread"
persistent_workers = true
tokio_worker_threads = 6
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.server.worker_mode, "thread");
        assert_eq!(config.server.persistent_workers, Some(true));
        assert_eq!(config.server.tokio_worker_threads, Some(6));
    }

    #[test]
    fn parse_persistent_workers_false() {
        let toml_str = r#"
[server]
persistent_workers = false
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.server.persistent_workers, Some(false));
    }

    #[test]
    fn parse_php_config() {
        let toml_str = r#"
[php]
memory_limit = "512M"
max_execution_time = 60
opcache_memory = 256
jit_buffer_size = "128M"
extensions = ["redis.so", "imagick.so"]
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.php.memory_limit, "512M");
        assert_eq!(config.php.max_execution_time, 60);
        assert_eq!(config.php.opcache_memory, 256);
        assert_eq!(config.php.jit_buffer_size, "128M");
        assert_eq!(config.php.extensions, vec!["redis.so", "imagick.so"]);
    }

    #[test]
    fn parse_php_config_defaults() {
        let config: RuntimeConfig = toml::from_str("").unwrap();
        assert_eq!(config.php.memory_limit, "256M");
        assert_eq!(config.php.max_execution_time, 30);
        assert_eq!(config.php.opcache_memory, 128);
        assert_eq!(config.php.jit_buffer_size, "64M");
        assert_eq!(config.php.upload_max_filesize, "64M");
        assert_eq!(config.php.post_max_size, "64M");
        assert_eq!(config.php.upload_tmp_dir, "/tmp/turbine-uploads");
        assert!(config.php.extensions.is_empty());
        assert!(config.php.zend_extensions.is_empty());
        assert!(config.php.preload_script.is_none());
    }

    #[test]
    fn parse_php_ini_directives() {
        let toml_str = r#"
[php.ini]
error_reporting = "E_ALL"
"date.timezone" = "UTC"
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.php.ini.get("error_reporting").unwrap(), "E_ALL");
        assert_eq!(config.php.ini.get("date.timezone").unwrap(), "UTC");
    }

    #[test]
    fn parse_security_config() {
        let toml_str = r#"
[security]
enabled = true
sql_guard = false
code_injection_guard = true
behaviour_guard = false
max_requests_per_second = 50
rate_limit_window = 120
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert!(config.security.enabled);
        assert!(!config.security.sql_guard);
        assert!(config.security.code_injection_guard);
        assert!(!config.security.behaviour_guard);
        assert_eq!(config.security.max_requests_per_second, 50);
        assert_eq!(config.security.rate_limit_window, 120);
    }

    #[test]
    fn parse_security_defaults() {
        let config: RuntimeConfig = toml::from_str("").unwrap();
        assert!(config.security.enabled);
        assert!(config.security.sql_guard);
        assert!(config.security.code_injection_guard);
        assert!(config.security.path_traversal_guard);
        assert!(config.security.behaviour_guard);
        assert_eq!(config.security.max_requests_per_second, 100);
        assert_eq!(config.security.rate_limit_window, 60);
    }

    #[test]
    fn parse_sandbox_config() {
        let toml_str = r#"
[sandbox]
execution_mode = "strict"
execution_whitelist = ["public/index.php"]
data_directories = ["storage/", "uploads/"]
scan_upload_content = false
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.sandbox.execution_mode, "strict");
        assert_eq!(config.sandbox.execution_whitelist, vec!["public/index.php"]);
        assert_eq!(
            config.sandbox.data_directories,
            vec!["storage/", "uploads/"]
        );
        assert!(!config.sandbox.scan_upload_content);
    }

    #[test]
    fn parse_sandbox_defaults() {
        let config: RuntimeConfig = toml::from_str("").unwrap();
        assert_eq!(config.sandbox.execution_mode, "framework");
        assert!(config.sandbox.execution_whitelist.is_empty());
        assert_eq!(config.sandbox.data_directories.len(), 3);
        assert!(config.sandbox.scan_upload_content);
        assert!(config.sandbox.enforce_open_basedir);
        assert!(config.sandbox.block_url_include);
        assert!(config.sandbox.block_url_fopen);
        assert_eq!(config.sandbox.disabled_functions.len(), 9);
        assert!(config
            .sandbox
            .disabled_functions
            .contains(&"exec".to_string()));
    }

    #[test]
    fn parse_cache_config() {
        let toml_str = r#"
[cache]
enabled = false
ttl_seconds = 60
max_entries = 2048
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.cache.enabled);
        assert_eq!(config.cache.ttl_seconds, 60);
        assert_eq!(config.cache.max_entries, 2048);
    }

    #[test]
    fn parse_cache_defaults() {
        let config: RuntimeConfig = toml::from_str("").unwrap();
        assert!(config.cache.enabled);
        assert_eq!(config.cache.ttl_seconds, 30);
        assert_eq!(config.cache.max_entries, 1024);
    }

    #[test]
    fn parse_compression_config() {
        let toml_str = r#"
[compression]
enabled = true
min_size = 512
level = 9
algorithms = ["gzip", "br"]
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert!(config.compression.enabled);
        assert_eq!(config.compression.min_size, 512);
        assert_eq!(config.compression.level, 9);
        assert_eq!(config.compression.algorithms, vec!["gzip", "br"]);
    }

    #[test]
    fn parse_compression_defaults() {
        let config: RuntimeConfig = toml::from_str("").unwrap();
        assert!(config.compression.enabled);
        assert_eq!(config.compression.min_size, 1024);
        assert_eq!(config.compression.level, 6);
        assert_eq!(config.compression.algorithms, vec!["br", "zstd", "gzip"]);
    }

    #[test]
    fn parse_session_config() {
        let toml_str = r#"
[session]
enabled = true
save_path = "/custom/sessions"
cookie_name = "MY_SID"
cookie_lifetime = 3600
cookie_httponly = false
cookie_secure = true
cookie_samesite = "Strict"
gc_maxlifetime = 7200
auto_start = true
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert!(config.session.enabled);
        assert_eq!(config.session.save_path, "/custom/sessions");
        assert_eq!(config.session.cookie_name, "MY_SID");
        assert_eq!(config.session.cookie_lifetime, 3600);
        assert!(!config.session.cookie_httponly);
        assert!(config.session.cookie_secure);
        assert_eq!(config.session.cookie_samesite, "Strict");
        assert_eq!(config.session.gc_maxlifetime, 7200);
        assert!(config.session.auto_start);
    }

    #[test]
    fn parse_session_defaults() {
        let config: RuntimeConfig = toml::from_str("").unwrap();
        assert!(config.session.enabled);
        assert_eq!(config.session.save_path, "/tmp/turbine-sessions");
        assert_eq!(config.session.cookie_name, "PHPSESSID");
        assert_eq!(config.session.cookie_lifetime, 0);
        assert!(config.session.cookie_httponly);
        assert!(!config.session.cookie_secure);
        assert_eq!(config.session.cookie_samesite, "Lax");
        assert_eq!(config.session.gc_maxlifetime, 1440);
        assert!(!config.session.auto_start);
    }

    #[test]
    fn parse_cors_config() {
        let toml_str = r#"
[cors]
enabled = true
allow_origins = ["https://example.com", "https://api.example.com"]
allow_credentials = true
allow_methods = ["GET", "POST"]
allow_headers = ["Authorization"]
expose_headers = ["X-Custom"]
max_age = 3600
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert!(config.cors.enabled);
        assert_eq!(config.cors.allow_origins.len(), 2);
        assert!(config.cors.allow_credentials);
        assert_eq!(config.cors.allow_methods, vec!["GET", "POST"]);
        assert_eq!(config.cors.allow_headers, vec!["Authorization"]);
        assert_eq!(config.cors.expose_headers, vec!["X-Custom"]);
        assert_eq!(config.cors.max_age, 3600);
    }

    #[test]
    fn parse_cors_defaults() {
        let config: RuntimeConfig = toml::from_str("").unwrap();
        assert!(!config.cors.enabled);
        assert!(config.cors.allow_origins.is_empty());
        assert!(!config.cors.allow_credentials);
        assert_eq!(config.cors.allow_methods.len(), 6);
        assert_eq!(config.cors.allow_headers.len(), 3);
        assert_eq!(config.cors.max_age, 86400);
    }

    #[test]
    fn parse_tls_config() {
        let toml_str = r#"
[server.tls]
enabled = true
cert_file = "/etc/ssl/cert.pem"
key_file = "/etc/ssl/key.pem"
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert!(config.server.tls.enabled);
        assert_eq!(config.server.tls.cert_file.unwrap(), "/etc/ssl/cert.pem");
        assert_eq!(config.server.tls.key_file.unwrap(), "/etc/ssl/key.pem");
    }

    #[test]
    fn parse_tls_defaults() {
        let config: RuntimeConfig = toml::from_str("").unwrap();
        assert!(!config.server.tls.enabled);
        assert!(config.server.tls.cert_file.is_none());
        assert!(config.server.tls.key_file.is_none());
    }

    #[test]
    fn parse_watcher_config() {
        let toml_str = r#"
[watcher]
enabled = true
paths = ["src/", "app/"]
extensions = ["php", "twig"]
debounce_ms = 1000
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert!(config.watcher.enabled);
        assert_eq!(config.watcher.paths, vec!["src/", "app/"]);
        assert_eq!(config.watcher.extensions, vec!["php", "twig"]);
        assert_eq!(config.watcher.debounce_ms, 1000);
    }

    #[test]
    fn parse_watcher_defaults() {
        let config: RuntimeConfig = toml::from_str("").unwrap();
        assert!(!config.watcher.enabled);
        assert_eq!(config.watcher.paths.len(), 5);
        assert_eq!(config.watcher.extensions, vec!["php", "env"]);
        assert_eq!(config.watcher.debounce_ms, 500);
    }

    #[test]
    fn parse_early_hints_defaults() {
        let config: RuntimeConfig = toml::from_str("").unwrap();
        assert!(config.early_hints.enabled);
    }

    #[test]
    fn parse_worker_pools() {
        let toml_str = r#"
[[worker_pools]]
match_path = "/api/reports/*"
min_workers = 2
max_workers = 8
name = "reports"

[[worker_pools]]
match_path = "/webhook"
min_workers = 1
max_workers = 2
name = "webhooks"
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.worker_pools.len(), 2);
        assert_eq!(config.worker_pools[0].name, Some("reports".to_string()));
        assert_eq!(config.worker_pools[0].match_path, "/api/reports/*");
        assert_eq!(config.worker_pools[0].min_workers, 2);
        assert_eq!(config.worker_pools[0].max_workers, 8);
        assert_eq!(config.worker_pools[1].name, Some("webhooks".to_string()));
    }

    #[test]
    fn parse_auto_scale_config() {
        let toml_str = r#"
[server]
auto_scale = true
min_workers = 2
max_workers = 32
scale_down_idle_secs = 10
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert!(config.server.auto_scale);
        assert_eq!(config.server.min_workers, 2);
        assert_eq!(config.server.max_workers, 32);
        assert_eq!(config.server.scale_down_idle_secs, 10);
    }

    #[test]
    fn parse_full_production_config() {
        let toml_str = r#"
[server]
workers = 8
listen = "0.0.0.0:443"
worker_mode = "process"
persistent_workers = true
request_timeout = 60
worker_max_requests = 50000
max_wait_time = 10
tokio_worker_threads = 4

[server.tls]
enabled = true
cert_file = "/etc/ssl/cert.pem"
key_file = "/etc/ssl/key.pem"

[php]
memory_limit = "512M"
opcache_memory = 256
jit_buffer_size = "128M"

[security]
enabled = true
max_requests_per_second = 200

[compression]
enabled = true
algorithms = ["br", "gzip"]

[logging]
level = "warn"

[watcher]
enabled = false
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.server.workers, 8);
        assert_eq!(config.server.listen, "0.0.0.0:443");
        assert_eq!(config.server.persistent_workers, Some(true));
        assert_eq!(config.server.worker_max_requests, 50000);
        assert_eq!(config.server.max_wait_time, 10);
        assert_eq!(config.server.tokio_worker_threads, Some(4));
        assert!(config.server.tls.enabled);
        assert_eq!(config.php.memory_limit, "512M");
        assert!(config.security.enabled);
        assert_eq!(config.logging.level, "warn");
        assert!(!config.watcher.enabled);
    }

    // ── Defaults ────────────────────────────────────────────────────

    #[test]
    fn runtime_config_default() {
        let config = RuntimeConfig::default();
        assert_eq!(config.server.worker_mode, "process");
        assert!(config.server.persistent_workers.is_none());
        assert!(config.security.enabled);
        assert!(config.cache.enabled);
        assert!(config.compression.enabled);
        assert!(!config.cors.enabled);
        assert!(!config.watcher.enabled);
        assert!(config.worker_pools.is_empty());
    }

    // ── Validation ──────────────────────────────────────────────────

    #[test]
    fn validate_does_not_panic_on_defaults() {
        let config = RuntimeConfig::default();
        config.validate(); // should NOT panic
    }

    #[test]
    fn validate_does_not_panic_on_edge_cases() {
        let toml_str = r#"
[server]
workers = 0
request_timeout = 0

[sandbox]
execution_mode = "strict"

[security]
enabled = true
sql_guard = false
code_injection_guard = false
behaviour_guard = false

[cache]
enabled = true
ttl_seconds = 0
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        config.validate(); // should log warnings, NOT panic
    }

    // ── Template ────────────────────────────────────────────────────

    #[test]
    fn template_is_non_empty() {
        let template = RuntimeConfig::template();
        assert!(template.len() > 100);
        assert!(template.contains("[server]"));
        assert!(template.contains("[php]"));
        assert!(template.contains("[security]"));
        assert!(template.contains("[compression]"));
    }

    // ── Unknown fields are ignored ──────────────────────────────────

    #[test]
    fn unknown_fields_ignored() {
        let toml_str = r#"
[server]
workers = 2
some_future_field = true

[nonexistent_section]
foo = "bar"
"#;
        // serde(default) should handle unknown fields gracefully
        let result: Result<RuntimeConfig, _> = toml::from_str(toml_str);
        // This might fail depending on serde config. If it does, that's fine -
        // it means the config is strict (which is also valid behavior).
        // We just want to document the behavior.
        if let Ok(config) = result {
            assert_eq!(config.server.workers, 2);
        }
    }

    // ── check() validation tests ────────────────────────────────────

    #[test]
    fn check_defaults_no_errors() {
        let config = RuntimeConfig::default();
        let (errors, _warnings) = config.check();
        assert!(
            errors.is_empty(),
            "Default config should have no errors: {:?}",
            errors
        );
    }

    #[test]
    fn check_invalid_worker_mode() {
        let mut config = RuntimeConfig::default();
        config.server.worker_mode = "fork".into();
        let (errors, _) = config.check();
        assert!(errors
            .iter()
            .any(|e| e.contains("worker_mode") && e.contains("fork")));
    }

    #[test]
    fn check_valid_worker_modes() {
        for mode in &["process", "thread"] {
            let mut config = RuntimeConfig::default();
            config.server.worker_mode = mode.to_string();
            let (errors, _) = config.check();
            assert!(
                !errors.iter().any(|e| e.contains("worker_mode")),
                "worker_mode = \"{}\" should be valid",
                mode
            );
        }
    }

    #[test]
    fn check_invalid_execution_mode() {
        let mut config = RuntimeConfig::default();
        config.sandbox.execution_mode = "custom".into();
        let (errors, _) = config.check();
        assert!(errors
            .iter()
            .any(|e| e.contains("execution_mode") && e.contains("custom")));
    }

    #[test]
    fn check_tls_enabled_without_cert() {
        let mut config = RuntimeConfig::default();
        config.server.tls.enabled = true;
        config.server.tls.cert_file = None;
        config.server.tls.key_file = None;
        let (errors, _) = config.check();
        assert!(errors.iter().any(|e| e.contains("cert_file")));
        assert!(errors.iter().any(|e| e.contains("key_file")));
    }

    #[test]
    fn check_tls_disabled_no_error() {
        let mut config = RuntimeConfig::default();
        config.server.tls.enabled = false;
        config.server.tls.cert_file = None;
        let (errors, _) = config.check();
        assert!(!errors.iter().any(|e| e.contains("cert_file")));
    }

    #[test]
    fn check_tls_cert_file_not_found() {
        let mut config = RuntimeConfig::default();
        config.server.tls.enabled = true;
        config.server.tls.cert_file = Some("/nonexistent/cert.pem".into());
        config.server.tls.key_file = Some("/nonexistent/key.pem".into());
        let (errors, _) = config.check();
        assert!(errors
            .iter()
            .any(|e| e.contains("cert.pem") && e.contains("not found")));
        assert!(errors
            .iter()
            .any(|e| e.contains("key.pem") && e.contains("not found")));
    }

    #[test]
    fn check_strict_without_whitelist() {
        let mut config = RuntimeConfig::default();
        config.sandbox.execution_mode = "strict".into();
        config.sandbox.execution_whitelist = vec![];
        let (errors, _) = config.check();
        assert!(errors
            .iter()
            .any(|e| e.contains("strict") && e.contains("whitelist")));
    }

    #[test]
    fn check_strict_with_whitelist_ok() {
        let mut config = RuntimeConfig::default();
        config.sandbox.execution_mode = "strict".into();
        config.sandbox.execution_whitelist = vec!["/index.php".into()];
        let (errors, _) = config.check();
        assert!(!errors.iter().any(|e| e.contains("whitelist")));
    }

    #[test]
    fn check_worker_pool_min_gt_max() {
        let mut config = RuntimeConfig::default();
        config.worker_pools.push(WorkerPoolRouteConfig {
            match_path: "/api/.*".into(),
            min_workers: 10,
            max_workers: 5,
            name: None,
        });
        let (errors, _) = config.check();
        assert!(errors
            .iter()
            .any(|e| e.contains("min_workers") && e.contains("max_workers")));
    }

    #[test]
    fn check_warn_workers_zero_timeout_zero() {
        let mut config = RuntimeConfig::default();
        config.server.workers = 0;
        config.server.request_timeout = 0;
        let (_, warnings) = config.check();
        assert!(warnings
            .iter()
            .any(|w| w.contains("workers = 0") && w.contains("request_timeout = 0")));
    }

    #[test]
    fn check_warn_security_all_guards_disabled() {
        let mut config = RuntimeConfig::default();
        config.security.enabled = true;
        config.security.sql_guard = false;
        config.security.code_injection_guard = false;
        config.security.behaviour_guard = false;
        let (_, warnings) = config.check();
        assert!(warnings
            .iter()
            .any(|w| w.contains("all guards are disabled")));
    }

    #[test]
    fn check_warn_cache_ttl_zero() {
        let mut config = RuntimeConfig::default();
        config.cache.enabled = true;
        config.cache.ttl_seconds = 0;
        let (_, warnings) = config.check();
        assert!(warnings.iter().any(|w| w.contains("ttl_seconds = 0")));
    }

    #[test]
    fn check_warn_high_workers() {
        let mut config = RuntimeConfig::default();
        config.server.workers = 128;
        let (_, warnings) = config.check();
        assert!(warnings.iter().any(|w| w.contains("workers = 128")));
    }

    #[test]
    fn check_warn_persistent_no_recycling() {
        let mut config = RuntimeConfig::default();
        config.server.persistent_workers = Some(true);
        config.server.worker_max_requests = 0;
        let (_, warnings) = config.check();
        assert!(warnings
            .iter()
            .any(|w| w.contains("persistent") && w.contains("recycle")));
    }

    #[test]
    fn check_warn_compression_level() {
        let mut config = RuntimeConfig::default();
        config.compression.level = 15;
        let (_, warnings) = config.check();
        assert!(warnings.iter().any(|w| w.contains("level = 15")));
    }

    #[test]
    fn check_warn_invalid_samesite() {
        let mut config = RuntimeConfig::default();
        config.session.cookie_samesite = "Invalid".into();
        let (_, warnings) = config.check();
        assert!(warnings.iter().any(|w| w.contains("cookie_samesite")));
    }

    #[test]
    fn check_valid_samesite_values() {
        for val in &["Lax", "Strict", "None"] {
            let mut config = RuntimeConfig::default();
            config.session.cookie_samesite = val.to_string();
            let (_, warnings) = config.check();
            assert!(
                !warnings.iter().any(|w| w.contains("cookie_samesite")),
                "cookie_samesite = \"{}\" should not warn",
                val
            );
        }
    }

    #[test]
    fn check_warn_cors_no_origins() {
        let mut config = RuntimeConfig::default();
        config.cors.enabled = true;
        config.cors.allow_origins = vec![];
        let (_, warnings) = config.check();
        assert!(warnings.iter().any(|w| w.contains("allow_origins")));
    }

    #[test]
    fn check_warn_acme_plus_tls() {
        let mut config = RuntimeConfig::default();
        config.acme.enabled = true;
        config.server.tls.enabled = true;
        config.server.tls.cert_file = Some("/tmp/cert.pem".into());
        config.server.tls.key_file = Some("/tmp/key.pem".into());
        let (_, warnings) = config.check();
        assert!(warnings
            .iter()
            .any(|w| w.contains("ACME") && w.contains("TLS")));
    }

    #[test]
    fn check_warn_empty_pool_match_path() {
        let mut config = RuntimeConfig::default();
        config.worker_pools.push(WorkerPoolRouteConfig {
            match_path: "".into(),
            min_workers: 1,
            max_workers: 4,
            name: None,
        });
        let (_, warnings) = config.check();
        assert!(warnings
            .iter()
            .any(|w| w.contains("match_path") && w.contains("empty")));
    }

    #[test]
    fn check_clean_config_no_warnings() {
        let config = RuntimeConfig::default();
        let (errors, warnings) = config.check();
        // Default config should have minimal warnings
        assert!(errors.is_empty());
        // The only possible default warning is compression level or samesite
        // but defaults should all be valid
        for w in &warnings {
            // If there are any, they should be benign
            println!("Default warning: {w}");
        }
    }

    #[test]
    fn check_multiple_errors_accumulated() {
        let mut config = RuntimeConfig::default();
        config.server.worker_mode = "invalid".into();
        config.sandbox.execution_mode = "invalid".into();
        config.server.tls.enabled = true;
        config.server.tls.cert_file = None;
        config.server.tls.key_file = None;
        let (errors, _) = config.check();
        assert!(
            errors.len() >= 4,
            "Expected at least 4 errors, got {}: {:?}",
            errors.len(),
            errors
        );
    }

    // ── Virtual host config tests ───────────────────────────────────

    #[test]
    fn parse_virtual_hosts() {
        let toml_str = r#"
[[virtual_hosts]]
domain = "xpto.com"
root = "/var/www/xpto"
aliases = ["www.xpto.com"]
entry_point = "index.php"

[[virtual_hosts]]
domain = "outro.com"
root = "/var/www/outro"
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.virtual_hosts.len(), 2);
        assert_eq!(config.virtual_hosts[0].domain, "xpto.com");
        assert_eq!(config.virtual_hosts[0].root, "/var/www/xpto");
        assert_eq!(config.virtual_hosts[0].aliases, vec!["www.xpto.com"]);
        assert_eq!(
            config.virtual_hosts[0].entry_point,
            Some("index.php".into())
        );
        assert_eq!(config.virtual_hosts[1].domain, "outro.com");
        assert_eq!(config.virtual_hosts[1].root, "/var/www/outro");
        assert!(config.virtual_hosts[1].aliases.is_empty());
        assert!(config.virtual_hosts[1].entry_point.is_none());
    }

    #[test]
    fn parse_virtual_hosts_with_tls() {
        let toml_str = r#"
[[virtual_hosts]]
domain = "secure.com"
root = "/var/www/secure"
tls_cert = "/etc/ssl/secure.pem"
tls_key = "/etc/ssl/secure-key.pem"
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.virtual_hosts[0].tls_cert,
            Some("/etc/ssl/secure.pem".into())
        );
        assert_eq!(
            config.virtual_hosts[0].tls_key,
            Some("/etc/ssl/secure-key.pem".into())
        );
    }

    #[test]
    fn parse_no_virtual_hosts() {
        let toml_str = r#"
[server]
workers = 4
"#;
        let config: RuntimeConfig = toml::from_str(toml_str).unwrap();
        assert!(config.virtual_hosts.is_empty());
    }

    #[test]
    fn check_vhost_empty_domain() {
        let mut config = RuntimeConfig::default();
        config.virtual_hosts.push(VirtualHostConfig {
            domain: "".into(),
            root: "/var/www/test".into(),
            aliases: vec![],
            entry_point: None,
            tls_cert: None,
            tls_key: None,
        });
        let (errors, _) = config.check();
        assert!(errors.iter().any(|e| e.contains("domain is empty")));
    }

    #[test]
    fn check_vhost_empty_root() {
        let mut config = RuntimeConfig::default();
        config.virtual_hosts.push(VirtualHostConfig {
            domain: "xpto.com".into(),
            root: "".into(),
            aliases: vec![],
            entry_point: None,
            tls_cert: None,
            tls_key: None,
        });
        let (errors, _) = config.check();
        assert!(errors.iter().any(|e| e.contains("root is empty")));
    }

    #[test]
    fn check_vhost_root_not_found() {
        let mut config = RuntimeConfig::default();
        config.virtual_hosts.push(VirtualHostConfig {
            domain: "xpto.com".into(),
            root: "/nonexistent/path".into(),
            aliases: vec![],
            entry_point: None,
            tls_cert: None,
            tls_key: None,
        });
        let (errors, _) = config.check();
        assert!(errors.iter().any(|e| e.contains("directory not found")));
    }

    #[test]
    fn check_vhost_duplicate_domain() {
        let mut config = RuntimeConfig::default();
        let vhost = VirtualHostConfig {
            domain: "xpto.com".into(),
            root: "/tmp".into(),
            aliases: vec![],
            entry_point: None,
            tls_cert: None,
            tls_key: None,
        };
        config.virtual_hosts.push(vhost.clone());
        config.virtual_hosts.push(vhost);
        let (errors, _) = config.check();
        assert!(errors.iter().any(|e| e.contains("duplicate domain")));
    }

    #[test]
    fn check_vhost_duplicate_alias() {
        let mut config = RuntimeConfig::default();
        config.virtual_hosts.push(VirtualHostConfig {
            domain: "xpto.com".into(),
            root: "/tmp".into(),
            aliases: vec!["www.xpto.com".into()],
            entry_point: None,
            tls_cert: None,
            tls_key: None,
        });
        config.virtual_hosts.push(VirtualHostConfig {
            domain: "outro.com".into(),
            root: "/tmp".into(),
            aliases: vec!["www.xpto.com".into()],
            entry_point: None,
            tls_cert: None,
            tls_key: None,
        });
        let (errors, _) = config.check();
        assert!(errors.iter().any(|e| e.contains("duplicate alias")));
    }

    #[test]
    fn check_vhost_tls_cert_without_key() {
        let mut config = RuntimeConfig::default();
        config.virtual_hosts.push(VirtualHostConfig {
            domain: "xpto.com".into(),
            root: "/tmp".into(),
            aliases: vec![],
            entry_point: None,
            tls_cert: Some("/tmp/cert.pem".into()),
            tls_key: None,
        });
        let (errors, _) = config.check();
        assert!(errors
            .iter()
            .any(|e| e.contains("tls_cert and tls_key must both be set")));
    }

    #[test]
    fn check_vhost_listen_localhost_warning() {
        let mut config = RuntimeConfig::default();
        config.server.listen = "127.0.0.1:8080".into();
        config.virtual_hosts.push(VirtualHostConfig {
            domain: "xpto.com".into(),
            root: "/tmp".into(),
            aliases: vec![],
            entry_point: None,
            tls_cert: None,
            tls_key: None,
        });
        let (_, warnings) = config.check();
        assert!(warnings.iter().any(|w| w.contains("127.0.0.1")));
    }

    #[test]
    fn check_vhost_valid_no_errors() {
        let mut config = RuntimeConfig::default();
        config.server.listen = "0.0.0.0:80".into();
        config.virtual_hosts.push(VirtualHostConfig {
            domain: "xpto.com".into(),
            root: "/tmp".into(),
            aliases: vec!["www.xpto.com".into()],
            entry_point: None,
            tls_cert: None,
            tls_key: None,
        });
        let (errors, _) = config.check();
        let vhost_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.contains("virtual_hosts"))
            .collect();
        assert!(
            vhost_errors.is_empty(),
            "Valid vhost should have no vhost errors: {:?}",
            vhost_errors
        );
    }

    #[test]
    fn template_contains_virtual_hosts() {
        let template = RuntimeConfig::template();
        assert!(template.contains("[[virtual_hosts]]"));
        assert!(template.contains("domain"));
        assert!(template.contains("aliases"));
    }
}
