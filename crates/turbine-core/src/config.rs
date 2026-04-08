use serde::Deserialize;
use tracing::{debug, info, warn};


#[derive(Debug, Deserialize)]
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
    /// When enabled, workers load the autoloader and optional `turbine-worker.php`
    /// once, then handle requests via include or custom handler.
    #[serde(default)]
    pub persistent_workers: Option<bool>,
    /// Number of Tokio async I/O threads (default = number of CPU cores).
    /// Increase to handle more concurrent connections; decrease to leave more
    /// cores for PHP worker processes.
    #[serde(default)]
    pub tokio_worker_threads: Option<usize>,
}

#[derive(Debug, Deserialize, Clone)]
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
    /// Rate limit: max requests per second per IP.
    #[serde(default = "default_rate_limit_rps")]
    pub max_requests_per_second: u32,
    /// Rate limit: time window in seconds.
    #[serde(default = "default_rate_limit_window")]
    pub rate_limit_window: u64,
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
#[derive(Debug, Deserialize, Clone)]
pub struct XSendfileConfig {
    /// Enable X-Sendfile support.
    #[serde(default)]
    pub enabled: bool,
    /// Base directory for X-Accel-Redirect paths (relative to app root).
    #[serde(default)]
    pub root: Option<String>,
}

impl Default for XSendfileConfig {
    fn default() -> Self {
        XSendfileConfig {
            enabled: false,
            root: None,
        }
    }
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
pub struct EmbedConfig {
    /// Enable embedded app mode. At build time, set TURBINE_EMBED_DIR env var
    /// to the directory containing the PHP app.
    #[serde(default)]
    pub enabled: bool,
    /// Directory to extract embedded files to at runtime (temp if empty).
    #[serde(default)]
    pub extract_dir: Option<String>,
}

impl Default for EmbedConfig {
    fn default() -> Self {
        EmbedConfig {
            enabled: false,
            extract_dir: None,
        }
    }
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

#[derive(Debug, Deserialize)]
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
        .into_iter().map(String::from).collect()
}

fn default_cors_headers() -> Vec<String> {
    vec!["Content-Type", "Authorization", "X-Requested-With"]
        .into_iter().map(String::from).collect()
}

fn default_cors_max_age() -> u64 {
    86400
}

fn default_rate_limit_rps() -> u32 {
    100
}

fn default_rate_limit_window() -> u64 {
    60
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

impl Default for TlsConfig {
    fn default() -> Self {
        TlsConfig {
            enabled: false,
            cert_file: None,
            key_file: None,
        }
    }
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
            tokio_worker_threads: None,
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
            max_requests_per_second: default_rate_limit_rps(),
            rate_limit_window: default_rate_limit_window(),
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

impl Default for ErrorPagesConfig {
    fn default() -> Self {
        ErrorPagesConfig {
            not_found: None,
            server_error: None,
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

impl Default for RuntimeConfig {
    fn default() -> Self {
        RuntimeConfig {
            server: ServerConfig::default(),
            php: PhpConfig::default(),
            security: SecurityConfig::default(),
            sandbox: SandboxConfig::default(),
            cache: CacheTomlConfig::default(),
            logging: LoggingConfig::default(),
            compression: CompressionConfig::default(),
            error_pages: ErrorPagesConfig::default(),
            session: SessionConfig::default(),
            cors: CorsConfig::default(),
            watcher: WatcherConfig::default(),
            early_hints: EarlyHintsConfig::default(),
            x_sendfile: XSendfileConfig::default(),
            structured_logging: StructuredLoggingConfig::default(),
            acme: AcmeConfig::default(),
            embed: EmbedConfig::default(),
            dashboard: DashboardConfig::default(),
            worker_pools: Vec::new(),
        }
    }
}

impl RuntimeConfig {
    /// Validate configuration for contradictions and warn about issues.
    pub fn validate(&self) {
        if self.server.workers == 0 && self.server.request_timeout == 0 {
            warn!("workers=0 (single-process) with request_timeout=0 (no timeout) — a slow request will block ALL subsequent requests");
        }

        if self.sandbox.execution_mode == "strict" && self.sandbox.execution_whitelist.is_empty() {
            warn!("execution_mode=\"strict\" but execution_whitelist is empty — ALL PHP requests will be blocked");
        }

        if self.security.enabled && !self.security.sql_guard && !self.security.code_injection_guard && !self.security.behaviour_guard {
            warn!("security.enabled=true but all guards are disabled — security layer has no effect");
        }

        if self.cache.enabled && self.cache.ttl_seconds == 0 {
            warn!("cache.enabled=true but ttl_seconds=0 — responses will be cached and immediately expire");
        }

        if self.server.workers > 64 {
            warn!(workers = self.server.workers, "Very high worker count — ensure your system has sufficient memory and file descriptors");
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
connection_timeout = 30# Request execution timeout in seconds (0 = no timeout)
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
# Enable automatic TLS via ACME (Let's Encrypt)
enabled = false
# Domain names for the certificate
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
"#
    }
}
