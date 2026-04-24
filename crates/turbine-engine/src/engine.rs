use std::ffi::{c_char, CString};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use tracing::{debug, error, info, warn};

use turbine_php_sys::{
    php_embed_init, php_embed_module, php_embed_shutdown, php_request_shutdown,
    php_request_startup, sapi_send_headers, zend_eval_string, FAILURE, SUCCESS,
};

use crate::output;
use crate::EngineError;

/// Captured PHP response (body + headers + status code).
pub struct PhpResponse {
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
    pub status_code: u16,
}

/// Global flag to track whether the PHP engine is initialized.
/// PHP embed SAPI is not re-entrant — only one instance per process.
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Path to the generated php.ini file (must live for the process lifetime).
///
/// We use `php_ini_path_override` instead of `ini_entries` because PHP's
/// `sapi_startup()` resets `ini_entries` to NULL before copying the module
/// struct. The `php_ini_path_override` field is NOT reset, so it survives.
static INI_PATH: std::sync::OnceLock<CString> = std::sync::OnceLock::new();

/// SAPI name override — must live for the process lifetime.
///
/// PHP < 8.5: OPcache's `accel_find_sapi()` only enables when `sapi_module.name`
/// is on a hard-coded whitelist. The "embed" SAPI isn't on it, so we register
/// as "cli-server" (which is on the whitelist) to get OPcache support.
///
/// PHP >= 8.5: the whitelist was removed (php-src#19351), so we register
/// our real name "turbine" — distinguishing Turbine from PHP's built-in
/// dev server for user code, profilers, and framework SAPI sniffing.
static SAPI_NAME: std::sync::OnceLock<CString> = std::sync::OnceLock::new();

/// Configuration for PHP ini generation, passed from the runtime config.
pub struct PhpIniOverrides {
    pub memory_limit: String,
    pub max_execution_time: u64,
    pub upload_max_filesize: String,
    pub post_max_size: String,
    pub opcache_memory: usize,
    pub jit_buffer_size: String,
    pub session_save_path: String,
    pub session_cookie_name: String,
    pub session_cookie_lifetime: u64,
    pub session_cookie_httponly: bool,
    pub session_cookie_secure: bool,
    pub session_cookie_samesite: String,
    pub session_gc_maxlifetime: u64,
    /// open_basedir restriction (empty = disabled).
    pub open_basedir: String,
    /// Comma-separated list of disabled PHP functions.
    pub disabled_functions: String,
    /// Block allow_url_include.
    pub block_url_include: bool,
    /// Block allow_url_fopen.
    pub block_url_fopen: bool,
    /// OPcache preload script path (empty = disabled).
    pub preload_script: String,
    /// Arbitrary extra php.ini directives from TOML [php.ini] config.
    pub extra_ini: std::collections::HashMap<String, String>,
    /// PHP extensions to load (e.g. ["redis.so"]).
    pub extensions: Vec<String>,
    /// Zend extensions to load (e.g. ["xdebug.so"]).
    pub zend_extensions: Vec<String>,
    /// Enable OPcache timestamp validation (default: false).
    pub opcache_validate_timestamps: bool,
}

impl Default for PhpIniOverrides {
    fn default() -> Self {
        PhpIniOverrides {
            memory_limit: "256M".to_string(),
            max_execution_time: 30,
            upload_max_filesize: "64M".to_string(),
            post_max_size: "64M".to_string(),
            opcache_memory: 128,
            jit_buffer_size: "64M".to_string(),
            session_save_path: "/tmp/turbine-sessions".to_string(),
            session_cookie_name: "PHPSESSID".to_string(),
            session_cookie_lifetime: 0,
            session_cookie_httponly: true,
            session_cookie_secure: false,
            session_cookie_samesite: "Lax".to_string(),
            session_gc_maxlifetime: 1440,
            open_basedir: String::new(),
            disabled_functions: String::new(),
            block_url_include: true,
            block_url_fopen: true,
            preload_script: String::new(),
            extra_ini: std::collections::HashMap::new(),
            extensions: Vec::new(),
            zend_extensions: Vec::new(),
            opcache_validate_timestamps: false,
        }
    }
}

/// Write a php.ini file to a temporary location and return its path.
///
/// The INI configures Zend OPcache with aggressive caching settings
/// suitable for a long-running embedded server process.
fn write_php_ini(overrides: &PhpIniOverrides) -> std::path::PathBuf {
    let ext_dir = turbine_php_sys::PHP_EXTENSION_DIR;
    let opcache_path = format!("{}/opcache.so", ext_dir);

    let mut ini = String::with_capacity(1024);

    // PHP 8.5+ compiles OPcache statically into libphp (no separate .so).
    // For older versions, we need to load it as a zend_extension.
    let opcache_so_exists = std::path::Path::new(&opcache_path).exists();
    if opcache_so_exists {
        ini.push_str(&format!("zend_extension={}\n", opcache_path));
    }

    // Configure OPcache settings (works for both static and shared builds)
    {
        ini.push_str("opcache.enable=1\n");
        ini.push_str("opcache.enable_cli=1\n");
        ini.push_str(&format!(
            "opcache.memory_consumption={}\n",
            overrides.opcache_memory
        ));
        ini.push_str("opcache.interned_strings_buffer=16\n");
        ini.push_str("opcache.max_accelerated_files=10000\n");
        ini.push_str(&format!(
            "opcache.validate_timestamps={}\n",
            if overrides.opcache_validate_timestamps {
                1
            } else {
                0
            }
        ));
        ini.push_str("opcache.revalidate_freq=0\n");
        ini.push_str("opcache.save_comments=1\n");
        // SHM mode: bytecode stays in shared memory (fastest for long-running process).
        // file_cache as L2: persists across restarts so second startup is warm.
        ini.push_str("opcache.file_cache=/tmp/turbine-opcache\n");
        // Disable mprotect to avoid SHM protection issues in embed SAPI
        ini.push_str("opcache.protect_memory=0\n");
        // JIT function mode: compiles hot functions to native code
        ini.push_str("opcache.jit=function\n");
        ini.push_str(&format!(
            "opcache.jit_buffer_size={}\n",
            overrides.jit_buffer_size
        ));
        // OPcache preload: compile classes/functions once at startup
        if !overrides.preload_script.is_empty() {
            let preload_path = std::path::Path::new(&overrides.preload_script);
            if preload_path.exists() {
                // PHP requires opcache.preload_user to be set to the username
                // PHP will run as. When running as root (uid 0), an empty value
                // triggers a fatal error: `opcache.preload requires
                // opcache.preload_user when running under uid 0`. Resolve the
                // current user's name via getpwuid() and fall back to "root"
                // if the lookup fails but uid is 0.
                let preload_user = unsafe {
                    let uid = libc::getuid();
                    let pw = libc::getpwuid(uid);
                    if !pw.is_null() && !(*pw).pw_name.is_null() {
                        std::ffi::CStr::from_ptr((*pw).pw_name)
                            .to_string_lossy()
                            .into_owned()
                    } else if uid == 0 {
                        "root".to_string()
                    } else {
                        String::new()
                    }
                };
                ini.push_str(&format!("opcache.preload={}\n", overrides.preload_script));
                ini.push_str(&format!("opcache.preload_user={}\n", preload_user));
                info!(preload = %overrides.preload_script, user = %preload_user, "OPcache preload configured");
            } else {
                warn!(preload = %overrides.preload_script, "OPcache preload script not found — skipping");
            }
        }
        if opcache_so_exists {
            info!(
                opcache_path = opcache_path,
                "OPcache configured (shared .so, SHM + file L2 cache)"
            );
        } else {
            info!("OPcache configured (built-in, SHM + file L2 cache)");
        }
    }

    ini.push_str("output_buffering=0\n");
    ini.push_str("display_errors=0\n");
    ini.push_str("log_errors=1\n");
    ini.push_str(&format!("memory_limit={}\n", overrides.memory_limit));
    ini.push_str(&format!(
        "max_execution_time={}\n",
        overrides.max_execution_time
    ));
    ini.push_str(&format!(
        "upload_max_filesize={}\n",
        overrides.upload_max_filesize
    ));
    ini.push_str(&format!("post_max_size={}\n", overrides.post_max_size));

    // Session configuration
    ini.push_str("session.save_handler=files\n");
    ini.push_str(&format!(
        "session.save_path={}\n",
        overrides.session_save_path
    ));
    ini.push_str(&format!("session.name={}\n", overrides.session_cookie_name));
    ini.push_str(&format!(
        "session.cookie_lifetime={}\n",
        overrides.session_cookie_lifetime
    ));
    ini.push_str(&format!(
        "session.cookie_httponly={}\n",
        if overrides.session_cookie_httponly {
            1
        } else {
            0
        }
    ));
    ini.push_str(&format!(
        "session.cookie_secure={}\n",
        if overrides.session_cookie_secure {
            1
        } else {
            0
        }
    ));
    ini.push_str(&format!(
        "session.cookie_samesite={}\n",
        overrides.session_cookie_samesite
    ));
    ini.push_str(&format!(
        "session.gc_maxlifetime={}\n",
        overrides.session_gc_maxlifetime
    ));
    ini.push_str("session.use_strict_mode=1\n");
    ini.push_str("session.use_only_cookies=1\n");

    // --- Camada 5: PHP INI Hardening (Fortress) ---
    if !overrides.open_basedir.is_empty() {
        ini.push_str(&format!("open_basedir={}\n", overrides.open_basedir));
        info!(open_basedir = %overrides.open_basedir, "PHP open_basedir restriction active");
    }
    if !overrides.disabled_functions.is_empty() {
        ini.push_str(&format!(
            "disable_functions={}\n",
            overrides.disabled_functions
        ));
        info!(functions = %overrides.disabled_functions, "PHP dangerous functions disabled");
    }
    if overrides.block_url_include {
        ini.push_str("allow_url_include=Off\n");
    }
    if overrides.block_url_fopen {
        ini.push_str("allow_url_fopen=Off\n");
    }

    // Load PHP extensions from [php] extensions list
    for ext in &overrides.extensions {
        ini.push_str(&format!("extension={}\n", ext));
        info!(extension = %ext, "PHP extension configured");
    }

    // Load Zend extensions from [php] zend_extensions list
    for ext in &overrides.zend_extensions {
        ini.push_str(&format!("zend_extension={}\n", ext));
        info!(zend_extension = %ext, "Zend extension configured");
    }

    // Custom php.ini directives from [php.ini] in turbine.toml.
    //
    // Reject `output_buffering` overrides: Turbine's SAPI captures output
    // via a `ub_write` callback that is only drained during
    // `php_request_shutdown` when `output_buffering > 0`. A non-zero value
    // causes large responses (> the buffer size) to be truncated on the
    // hot path — enforce 0 unconditionally.
    for (key, value) in &overrides.extra_ini {
        if key.eq_ignore_ascii_case("output_buffering") {
            warn!(
                directive = %key,
                value = %value,
                "Ignored php.ini override: output_buffering must be 0 under Turbine"
            );
            continue;
        }
        ini.push_str(&format!("{}={}\n", key, value));
        info!(directive = %key, value = %value, "Custom php.ini directive set");
    }

    let ini_path = std::env::temp_dir().join("turbine-php.ini");
    std::fs::write(&ini_path, ini).expect("Failed to write turbine php.ini");

    // Ensure OPcache file cache directory exists
    let _ = std::fs::create_dir_all("/tmp/turbine-opcache");

    info!(path = %ini_path.display(), "Wrote php.ini");
    ini_path
}

/// Safe wrapper around the PHP embed SAPI.
///
/// Only one instance can exist per process. The engine is initialized on
/// creation and shut down on drop.
pub struct PhpEngine {
    /// Whether a request is currently active (between request_startup/shutdown).
    request_active: bool,
}

impl PhpEngine {
    fn capture_headers_via_php(&self) -> Vec<(String, String)> {
        output::clear_output_buffer();

        if self
            .eval("foreach (headers_list() as $__turbine_header) { echo $__turbine_header, \"\\n\"; }")
            .is_err()
        {
            return Vec::new();
        }

        let raw = output::take_output();
        let mut headers = Vec::new();

        if let Ok(text) = String::from_utf8(raw) {
            for line in text.lines() {
                if let Some((name, value)) = line.split_once(':') {
                    headers.push((name.trim().to_string(), value.trim().to_string()));
                }
            }
        }

        headers
    }

    /// Initialize the PHP embed SAPI.
    ///
    /// `php_embed_init` internally calls `php_request_startup`, so the engine
    /// starts with an active request context.
    pub fn init() -> Result<Self, EngineError> {
        Self::init_with(PhpIniOverrides::default())
    }

    /// Initialize the PHP embed SAPI with custom INI overrides.
    pub fn init_with(overrides: PhpIniOverrides) -> Result<Self, EngineError> {
        if INITIALIZED.swap(true, Ordering::SeqCst) {
            return Err(EngineError::AlreadyInitialized);
        }

        info!(
            php_version = turbine_php_sys::PHP_VERSION,
            "Initializing PHP embed SAPI"
        );

        // Build and store INI entries (must outlive the PHP engine)
        // Write php.ini to disk and point PHP to it.
        // We use php_ini_path_override because sapi_startup() resets ini_entries to NULL.
        let ini_path = write_php_ini(&overrides);
        let ini_path_c = CString::new(ini_path.to_str().expect("ini path is valid UTF-8"))
            .expect("ini path has no null bytes");
        let ini_path_ptr = ini_path_c.as_ptr();
        INI_PATH.get_or_init(|| ini_path_c);

        unsafe {
            php_embed_module.php_ini_path_override = ini_path_ptr as *mut c_char;
            // Tell PHP to ignore default php.ini search paths
            php_embed_module.php_ini_ignore = 1;

            // Override SAPI name BEFORE php_embed_init.
            //
            // OPcache's accel_find_sapi() in PHP < 8.5 keeps a hard-coded
            // whitelist of supported SAPIs; if our name isn't on it,
            // OPcache silently disables itself. "cli-server" is on the
            // whitelist and is a long-running server SAPI, so it was the
            // compatible choice for PHP 8.4 and earlier.
            //
            // PHP >= 8.5.0 removed the whitelist (php-src#19351), so we
            // can (and should) expose our real identity. Using our own
            // name lets user code and extensions distinguish Turbine from
            // PHP's built-in dev server via `PHP_SAPI === 'turbine'`.
            let name = if turbine_php_sys::PHP_VERSION_ID >= 80500 {
                CString::new("turbine").unwrap()
            } else {
                CString::new("cli-server").unwrap()
            };
            let name_ptr = name.as_ptr();
            SAPI_NAME.get_or_init(|| name);
            php_embed_module.name = name_ptr as *mut c_char;
        }

        let result = unsafe { php_embed_init(0, ptr::null_mut()) };

        if result == FAILURE {
            INITIALIZED.store(false, Ordering::SeqCst);
            error!("php_embed_init returned FAILURE");
            return Err(EngineError::InitFailed);
        }

        debug!("PHP embed SAPI initialized successfully");

        // Install our custom output capture handler
        unsafe { output::install_output_capture() };
        debug!("SAPI output capture installed");

        Ok(PhpEngine {
            request_active: true, // php_embed_init starts a request
        })
    }

    /// Evaluate a PHP code string and return the result status.
    ///
    /// The code should NOT include `<?php` tags.
    pub fn eval(&self, code: &str) -> Result<(), EngineError> {
        let c_code = CString::new(code).map_err(|_| EngineError::NullByteInCode)?;
        let c_name = CString::new("turbine_eval").expect("static string has no null bytes");

        debug!(code = code, "Evaluating PHP code");

        let result = unsafe { zend_eval_string(c_code.as_ptr(), ptr::null_mut(), c_name.as_ptr()) };

        debug!(code = code, result = result, "zend_eval_string returned");

        if result == SUCCESS {
            Ok(())
        } else {
            error!(code = code, "PHP eval failed");
            Err(EngineError::EvalFailed {
                code: code.to_string(),
            })
        }
    }

    /// End the current request and start a new one.
    ///
    /// This performs the surgical state reset between requests:
    /// - Clears request heap
    /// - Resets superglobals ($_GET, $_POST, $_SERVER, etc.)
    /// - Resets output buffer
    /// - Clears pending exceptions
    /// - Does NOT touch: class_table, function_table, opcache, string_table
    pub fn reset_request(&mut self) -> Result<(), EngineError> {
        debug!("Resetting PHP request state");

        if self.request_active {
            unsafe {
                php_request_shutdown(ptr::null_mut());
            }
            self.request_active = false;
        }

        let result = unsafe { php_request_startup() };
        if result == FAILURE {
            error!("php_request_startup failed during reset");
            return Err(EngineError::RequestLifecycle);
        }

        // php_request_startup() re-initializes SAPI internals, which resets
        // ub_write and header_handler back to the embed SAPI defaults.
        // We must re-install our custom output capture after every restart.
        unsafe { output::install_output_capture() };

        self.request_active = true;
        debug!("PHP request state reset complete");
        Ok(())
    }

    /// Execute PHP code and capture all output (echo, print, etc.).
    ///
    /// Returns the captured output bytes. The output buffer is cleared
    /// before execution and drained after.
    pub fn eval_capture(&self, code: &str) -> Result<Vec<u8>, EngineError> {
        output::clear_output_buffer();
        self.eval(code)?;
        Ok(output::take_output())
    }

    /// Execute PHP code and capture output, HTTP headers, and status code.
    ///
    /// Returns (body, headers, status_code). Headers are pairs of (name, value).
    /// Status code defaults to 200 if PHP didn't set one explicitly.
    pub fn eval_capture_full(&self, code: &str) -> Result<PhpResponse, EngineError> {
        output::clear_output_buffer();
        self.eval(code)?;
        unsafe {
            let _ = sapi_send_headers();
        }
        let body = output::take_output();
        let mut headers = output::take_headers();
        if headers.is_empty() {
            headers = self.capture_headers_via_php();
        }
        Ok(PhpResponse {
            body,
            headers,
            status_code: output::take_response_code(),
        })
    }

    /// Get the PHP version string detected at build time.
    pub fn php_version(&self) -> &'static str {
        turbine_php_sys::PHP_VERSION
    }

    /// Check if a request context is currently active.
    pub fn is_request_active(&self) -> bool {
        self.request_active
    }
}

impl Drop for PhpEngine {
    fn drop(&mut self) {
        info!("Shutting down PHP embed SAPI");
        unsafe {
            php_embed_shutdown();
        }
        self.request_active = false;
        INITIALIZED.store(false, Ordering::SeqCst);
        debug!("PHP embed SAPI shut down");
    }
}
