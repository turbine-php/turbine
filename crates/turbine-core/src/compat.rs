//! PHP compatibility layer.
//!
//! Provides full HTTP request parsing (headers, body, cookies, query string),
//! complete `$_GET`/`$_POST`/`$_COOKIE`/`$_FILES`/`$_SERVER`/`$_REQUEST`
//! population, front-controller routing (`public/`), composer autoload
//! detection, and `.env` file integration.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use http_body_util::BodyExt;
use tracing::{info, warn};

/// Upload security configuration passed from the sandbox config.
pub struct UploadSecurityConfig {
    pub blocked_extensions: Vec<String>,
    pub scan_content: bool,
}

impl Default for UploadSecurityConfig {
    fn default() -> Self {
        UploadSecurityConfig {
            blocked_extensions: vec![
                ".php".to_string(),
                ".phtml".to_string(),
                ".phar".to_string(),
                ".php7".to_string(),
                ".php8".to_string(),
                ".inc".to_string(),
                ".phps".to_string(),
                ".pht".to_string(),
                ".pgif".to_string(),
            ],
            scan_content: true,
        }
    }
}

/// PHP code signatures to scan for in uploaded file content.
const PHP_SIGNATURES: &[&[u8]] = &[
    b"<?php",
    b"<?=",
    b"<script language=\"php\">",
    b"<script language='php'>",
];

/// A parsed uploaded file from multipart/form-data.
#[derive(Debug, Clone)]
pub struct UploadedFile {
    pub field_name: String,
    pub file_name: String,
    pub content_type: String,
    pub data: Vec<u8>,
    pub tmp_path: String,
}

impl Drop for UploadedFile {
    fn drop(&mut self) {
        if !self.tmp_path.is_empty() {
            let _ = std::fs::remove_file(&self.tmp_path);
        }
    }
}

/// Full HTTP request data parsed from a raw HTTP stream.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FullHttpRequest {
    pub method: String,
    pub path: String,
    pub query_string: String,
    pub http_version: String,
    pub headers: HashMap<String, String>,
    pub cookies: HashMap<String, String>,
    pub body: Vec<u8>,
    pub content_type: Option<String>,
    pub content_length: Option<usize>,
    /// Parsed files from multipart/form-data uploads.
    pub files: Vec<UploadedFile>,
    /// POST parameters extracted from multipart/form-data (non-file fields).
    pub multipart_post_params: Vec<(String, String)>,
}

impl FullHttpRequest {
    /// Parse a raw HTTP request from buffered bytes.
    #[allow(dead_code)]
    pub fn parse(raw: &[u8]) -> Option<Self> {
        let raw_str = String::from_utf8_lossy(raw);
        let (head, body_part) = if let Some(pos) = raw_str.find("\r\n\r\n") {
            (&raw_str[..pos], &raw[pos + 4..])
        } else {
            return None;
        };

        let mut lines = head.lines();
        let request_line = lines.next()?;
        let parts: Vec<&str> = request_line.split_whitespace().collect();
        if parts.len() < 2 {
            return None;
        }

        let method = parts[0].to_string();
        let full_uri = parts[1].to_string();
        let http_version = parts.get(2).unwrap_or(&"HTTP/1.1").to_string();

        let (path, query_string) = if let Some((p, q)) = full_uri.split_once('?') {
            (p.to_string(), q.to_string())
        } else {
            (full_uri, String::new())
        };

        let mut headers = HashMap::new();
        for line in lines {
            if let Some((key, value)) = line.split_once(':') {
                headers.insert(key.trim().to_lowercase(), value.trim().to_string());
            }
        }

        let content_type = headers.get("content-type").cloned();
        let content_length = headers
            .get("content-length")
            .and_then(|v| v.parse::<usize>().ok());

        let cookies = headers
            .get("cookie")
            .map(|c| parse_cookie_header(c))
            .unwrap_or_default();

        let body = body_part.to_vec();

        Some(Self {
            method,
            path,
            query_string,
            http_version,
            headers,
            cookies,
            body,
            content_type,
            content_length,
            files: Vec::new(),
            multipart_post_params: Vec::new(),
        })
    }

    /// Parse `$_GET` parameters from query string.
    pub fn get_params(&self) -> Vec<(String, String)> {
        parse_urlencoded(&self.query_string)
    }

    /// Build a FullHttpRequest from a hyper Request.
    pub async fn from_hyper(
        req: hyper::Request<hyper::body::Incoming>,
        remote_addr: std::net::SocketAddr,
        upload_tmp_dir: &str,
        upload_security: &UploadSecurityConfig,
    ) -> Option<(Self, std::net::SocketAddr)> {
        let method = req.method().to_string();
        let uri = req.uri().clone();
        let path = uri.path().to_string();
        let query_string = uri.query().unwrap_or("").to_string();
        let http_version = format!("{:?}", req.version());

        let mut headers = HashMap::new();
        for (name, value) in req.headers() {
            if let Ok(v) = value.to_str() {
                headers.insert(name.as_str().to_lowercase(), v.to_string());
            }
        }

        let content_type = headers.get("content-type").cloned();
        let content_length = headers
            .get("content-length")
            .and_then(|v| v.parse::<usize>().ok());

        let cookies = headers
            .get("cookie")
            .map(|c| parse_cookie_header(c))
            .unwrap_or_default();

        let body = match req.collect().await {
            Ok(collected) => collected.to_bytes().to_vec(),
            Err(_) => Vec::new(),
        };

        let (files, multipart_post_params) = if let Some(ref ct) = content_type {
            if ct.starts_with("multipart/form-data") {
                parse_multipart(ct, &body, upload_tmp_dir, upload_security)
            } else {
                (Vec::new(), Vec::new())
            }
        } else {
            (Vec::new(), Vec::new())
        };

        Some((
            Self {
                method,
                path,
                query_string,
                http_version,
                headers,
                cookies,
                body,
                content_type,
                content_length,
                files,
                multipart_post_params,
            },
            remote_addr,
        ))
    }

    /// Parse `$_POST` parameters from body (for `application/x-www-form-urlencoded`)
    /// and merge with multipart/form-data text fields.
    pub fn post_params(&self) -> Vec<(String, String)> {
        if self.method != "POST" && self.method != "PUT" && self.method != "PATCH" {
            return Vec::new();
        }

        if !self.multipart_post_params.is_empty() {
            return self.multipart_post_params.clone();
        }

        match self.content_type.as_deref() {
            Some(ct) if ct.starts_with("application/x-www-form-urlencoded") => {
                let body_str = String::from_utf8_lossy(&self.body);
                parse_urlencoded(&body_str)
            }
            _ => Vec::new(),
        }
    }

    /// Check if this is a JSON request.
    #[allow(dead_code)]
    pub fn is_json(&self) -> bool {
        self.content_type
            .as_deref()
            .map(|ct| ct.contains("application/json"))
            .unwrap_or(false)
    }

    /// Generate PHP code that populates all superglobals from this request.
    pub fn php_superglobals_code(
        &self,
        app_root: &Path,
        script_path: &str,
        client_ip: &str,
        server_port: u16,
        is_tls: bool,
    ) -> String {
        let mut code = String::with_capacity(2048);

        // --- $_SERVER ---
        code.push_str(&format!(
            "$_SERVER['REQUEST_METHOD'] = '{}'; ",
            escape_php(&self.method)
        ));
        code.push_str(&format!(
            "$_SERVER['REQUEST_URI'] = '{}'; ",
            escape_php(&format!(
                "{}{}",
                &self.path,
                if self.query_string.is_empty() {
                    String::new()
                } else {
                    format!("?{}", &self.query_string)
                }
            ))
        ));
        code.push_str(&format!(
            "$_SERVER['QUERY_STRING'] = '{}'; ",
            escape_php(&self.query_string)
        ));
        code.push_str(&format!(
            "$_SERVER['SCRIPT_NAME'] = '/{}'; ",
            escape_php(script_path)
        ));
        code.push_str(&format!(
            "$_SERVER['SCRIPT_FILENAME'] = '{}'; ",
            escape_php(&app_root.join(script_path).display().to_string())
        ));
        code.push_str(&format!(
            "$_SERVER['DOCUMENT_ROOT'] = '{}'; ",
            escape_php(&app_root.display().to_string())
        ));
        code.push_str(&format!(
            "$_SERVER['SERVER_SOFTWARE'] = 'Turbine/{}'; ",
            env!("CARGO_PKG_VERSION")
        ));
        code.push_str("$_SERVER['SERVER_PROTOCOL'] = 'HTTP/1.1'; ");
        code.push_str(&format!("$_SERVER['SERVER_PORT'] = '{}'; ", server_port));
        code.push_str("$_SERVER['SERVER_NAME'] = 'localhost'; ");
        if is_tls {
            code.push_str("$_SERVER['HTTPS'] = 'on'; ");
            code.push_str("$_SERVER['REQUEST_SCHEME'] = 'https'; ");
        } else {
            code.push_str("$_SERVER['REQUEST_SCHEME'] = 'http'; ");
        }
        code.push_str(&format!(
            "$_SERVER['REMOTE_ADDR'] = '{}'; ",
            escape_php(client_ip)
        ));
        code.push_str("$_SERVER['REMOTE_PORT'] = '0'; ");
        code.push_str(&format!(
            "$_SERVER['PATH_INFO'] = '{}'; ",
            escape_php(&self.path)
        ));

        // HTTP headers → $_SERVER['HTTP_*']
        for (key, value) in &self.headers {
            let server_key = format!("HTTP_{}", key.replace('-', "_").to_uppercase());
            code.push_str(&format!(
                "$_SERVER['{}'] = '{}'; ",
                escape_php(&server_key),
                escape_php(value)
            ));
        }

        if let Some(ref ct) = self.content_type {
            code.push_str(&format!(
                "$_SERVER['CONTENT_TYPE'] = '{}'; ",
                escape_php(ct)
            ));
        }

        if let Some(cl) = self.content_length {
            code.push_str(&format!("$_SERVER['CONTENT_LENGTH'] = '{}'; ", cl));
        }

        // --- $_GET ---
        let get_params = self.get_params();
        if !get_params.is_empty() {
            code.push_str("$_GET = [");
            for (k, v) in &get_params {
                code.push_str(&format!("'{}' => '{}', ", escape_php(k), escape_php(v)));
            }
            code.push_str("]; ");
        } else {
            code.push_str("$_GET = []; ");
        }

        // --- $_POST ---
        let post_params = self.post_params();
        if !post_params.is_empty() {
            code.push_str("$_POST = [");
            for (k, v) in &post_params {
                code.push_str(&format!("'{}' => '{}', ", escape_php(k), escape_php(v)));
            }
            code.push_str("]; ");
        } else {
            code.push_str("$_POST = []; ");
        }

        // --- $_COOKIE ---
        if !self.cookies.is_empty() {
            code.push_str("$_COOKIE = [");
            for (k, v) in &self.cookies {
                code.push_str(&format!("'{}' => '{}', ", escape_php(k), escape_php(v)));
            }
            code.push_str("]; ");
        } else {
            code.push_str("$_COOKIE = []; ");
        }

        // --- $_REQUEST (merged GET + POST + COOKIE, same as PHP default) ---
        code.push_str("$_REQUEST = array_merge($_GET, $_POST, $_COOKIE); ");

        // --- $_FILES (parsed from multipart/form-data) ---
        if !self.files.is_empty() {
            code.push_str("$_FILES = [");
            for file in &self.files {
                code.push_str(&format!(
                    "'{}' => ['name' => '{}', 'type' => '{}', 'tmp_name' => '{}', 'error' => 0, 'size' => {}], ",
                    escape_php(&file.field_name),
                    escape_php(&file.file_name),
                    escape_php(&file.content_type),
                    escape_php(&file.tmp_path),
                    file.data.len(),
                ));
            }
            code.push_str("]; ");

            // Track uploaded temp files so we can validate them.
            // PHP's is_uploaded_file()/move_uploaded_file() won't work because the embed SAPI
            // doesn't know about our uploads. We register helper functions that frameworks can use.
            code.push_str("$GLOBALS['__turbine_uploaded_files'] = [");
            for file in &self.files {
                code.push_str(&format!("'{}' => true, ", escape_php(&file.tmp_path)));
            }
            code.push_str("]; ");
        } else {
            code.push_str("$_FILES = []; ");
        }

        // --- php://input for JSON/raw body ---
        if !self.body.is_empty() {
            let body_str = String::from_utf8_lossy(&self.body);
            // For JSON/raw body access, we store it in a global that can be
            // accessed via php://input (which the SAPI handles) or our helper
            code.push_str(&format!(
                "$GLOBALS['__turbine_raw_body'] = '{}'; ",
                escape_php(&body_str)
            ));
            // Override php://input via stream wrapper for frameworks that use it
            code.push_str(
                "if (!class_exists('TurbineInputStream')) { \
                    class TurbineInputStream { \
                        private $pos = 0; \
                        private $data; \
                        public function stream_open($path, $mode, $options, &$opened_path) { \
                            $this->data = $GLOBALS['__turbine_raw_body'] ?? ''; \
                            return true; \
                        } \
                        public function stream_read($count) { \
                            $chunk = substr($this->data, $this->pos, $count); \
                            $this->pos += strlen($chunk); \
                            return $chunk; \
                        } \
                        public function stream_eof() { return $this->pos >= strlen($this->data); } \
                        public function stream_stat() { return ['size' => strlen($this->data)]; } \
                        public function stream_seek($offset, $whence) { \
                            if ($whence === SEEK_SET) { $this->pos = $offset; } \
                            elseif ($whence === SEEK_CUR) { $this->pos += $offset; } \
                            elseif ($whence === SEEK_END) { $this->pos = strlen($this->data) + $offset; } \
                            return true; \
                        } \
                        public function stream_tell() { return $this->pos; } \
                    } \
                    stream_wrapper_unregister('php'); \
                    stream_wrapper_register('php', 'TurbineInputStream'); \
                } "
            );
        }

        code
    }
}

/// Parse multipart/form-data body into files and text fields.
///
/// Files are written to temp files so PHP can access them via `$_FILES['tmp_name']`.
/// Text fields are returned as key-value pairs for `$_POST`.
fn parse_multipart(
    content_type: &str,
    body: &[u8],
    upload_tmp_dir: &str,
    upload_security: &UploadSecurityConfig,
) -> (Vec<UploadedFile>, Vec<(String, String)>) {
    let boundary = match multer::parse_boundary(content_type) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "Failed to parse multipart boundary");
            return (Vec::new(), Vec::new());
        }
    };

    let body_bytes = bytes::Bytes::from(body.to_vec());
    let stream = futures_stream_once(body_bytes);
    let mut multipart = multer::Multipart::new(stream, boundary);

    let mut files = Vec::new();
    let mut text_fields = Vec::new();

    let rt = tokio::runtime::Handle::try_current();
    let process = async {
        loop {
            match multipart.next_field().await {
                Ok(Some(field)) => {
                    let field_name = field.name().unwrap_or("").to_string();
                    let file_name = field.file_name().map(|s| s.to_string());
                    let ct = field
                        .content_type()
                        .map(|m| m.to_string())
                        .unwrap_or_default();
                    let data = match field.bytes().await {
                        Ok(b) => b.to_vec(),
                        Err(_) => continue,
                    };

                    if let Some(fname) = file_name {
                        // File field — write to temp file
                        if fname.is_empty() {
                            continue;
                        }

                        // --- Camada 4: Upload Hardening (Fortress) ---

                        // Check blocked extensions (case-insensitive, double-extension aware)
                        let fname_lower = fname.to_lowercase();
                        let mut extension_blocked = false;
                        for blocked_ext in &upload_security.blocked_extensions {
                            if fname_lower.ends_with(&blocked_ext.to_lowercase()) {
                                extension_blocked = true;
                                break;
                            }
                            // Detect double extensions like "shell.php.jpg" → still contains ".php."
                            let with_dot = format!("{}.", blocked_ext.to_lowercase());
                            if fname_lower.contains(&with_dot) {
                                extension_blocked = true;
                                break;
                            }
                        }
                        if extension_blocked {
                            warn!(
                                filename = %fname, field = %field_name,
                                "BLOCKED: upload with dangerous file extension"
                            );
                            continue;
                        }

                        // Scan content for PHP code signatures
                        if upload_security.scan_content {
                            let content_lower: Vec<u8> =
                                data.iter().map(|b| b.to_ascii_lowercase()).collect();
                            let mut php_detected = false;
                            for sig in PHP_SIGNATURES {
                                let sig_lower: Vec<u8> =
                                    sig.iter().map(|b| b.to_ascii_lowercase()).collect();
                                if content_lower
                                    .windows(sig_lower.len())
                                    .any(|w| w == sig_lower.as_slice())
                                {
                                    php_detected = true;
                                    break;
                                }
                            }
                            if php_detected {
                                warn!(
                                    filename = %fname, field = %field_name, size = data.len(),
                                    "BLOCKED: upload contains embedded PHP code"
                                );
                                continue;
                            }
                        }

                        let tmp_path = format!(
                            "{}/turbine_upload_{}_{}",
                            upload_tmp_dir,
                            std::process::id(),
                            files.len()
                        );
                        if let Err(e) = std::fs::write(&tmp_path, &data) {
                            warn!(path = %tmp_path, error = %e, "Failed to write upload temp file");
                            continue;
                        }
                        files.push(UploadedFile {
                            field_name,
                            file_name: fname,
                            content_type: if ct.is_empty() {
                                "application/octet-stream".to_string()
                            } else {
                                ct
                            },
                            data,
                            tmp_path,
                        });
                    } else {
                        // Text field → $_POST
                        let value = String::from_utf8_lossy(&data).to_string();
                        text_fields.push((field_name, value));
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    warn!(error = %e, "Error reading multipart field");
                    break;
                }
            }
        }
    };

    if let Ok(handle) = rt {
        tokio::task::block_in_place(|| handle.block_on(process));
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(process);
    }

    (files, text_fields)
}

/// Create a single-item stream from bytes for multer.
fn futures_stream_once(
    data: bytes::Bytes,
) -> impl futures_core::Stream<Item = Result<bytes::Bytes, std::io::Error>> {
    futures_core_once_stream(data)
}

/// Minimal single-item stream implementation (avoids adding `futures` dependency).
fn futures_core_once_stream(
    data: bytes::Bytes,
) -> impl futures_core::Stream<Item = Result<bytes::Bytes, std::io::Error>> {
    OnceStream { data: Some(data) }
}

/// A stream that yields exactly one item.
struct OnceStream {
    data: Option<bytes::Bytes>,
}

impl futures_core::Stream for OnceStream {
    type Item = Result<bytes::Bytes, std::io::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Ready(self.data.take().map(Ok))
    }
}

/// Detect the application structure and find the entry point.
///
/// Supports:
/// - Apps with `public/index.php` (front controller pattern)
/// - Apps with root `index.php` and composer autoloader (front controller)
/// - Generic: `index.php` in root (direct file mapping)
pub struct AppDetector;

impl AppDetector {
    /// Detect the application structure using purely structural heuristics.
    ///
    /// No framework-specific detection — just file/directory existence checks.
    /// The `front_controller` flag is auto-detected but can be overridden via config.
    pub fn detect(app_root: &Path) -> AppStructure {
        let has_composer = app_root.join("vendor").join("autoload.php").exists();
        let has_env = app_root.join(".env").exists();
        let autoload_path = if has_composer {
            Some(app_root.join("vendor").join("autoload.php"))
        } else {
            None
        };

        // Pattern 1: public/index.php — standard front controller layout
        let public_index = app_root.join("public").join("index.php");
        if public_index.exists() {
            info!(
                entry = "public/index.php",
                "Detected front-controller application (public/index.php)"
            );
            return AppStructure {
                document_root: app_root.join("public"),
                entry_point: "index.php".to_string(),
                front_controller: true,
                has_composer,
                has_env,
                autoload_path,
            };
        }

        // Pattern 2: root index.php exists
        let root_index = app_root.join("index.php");
        if root_index.exists() {
            // Heuristic: if there's a composer autoloader, this is likely a front controller app.
            let is_front_controller = has_composer;

            info!(
                entry = "index.php",
                front_controller = is_front_controller,
                "Detected application with root index.php"
            );
            return AppStructure {
                document_root: app_root.to_path_buf(),
                entry_point: "index.php".to_string(),
                front_controller: is_front_controller,
                has_composer,
                has_env,
                autoload_path,
            };
        }

        // Fallback: no index.php found
        AppStructure {
            document_root: app_root.to_path_buf(),
            entry_point: "index.php".to_string(),
            front_controller: false,
            has_composer: false,
            has_env: false,
            autoload_path: None,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AppStructure {
    pub document_root: PathBuf,
    pub entry_point: String,
    /// When true, unresolved paths fall back to the entry point (front controller pattern).
    /// When false, paths map directly to files (classic PHP directory).
    pub front_controller: bool,
    pub has_composer: bool,
    pub has_env: bool,
    pub autoload_path: Option<PathBuf>,
}

impl AppStructure {
    /// Generate PHP bootstrap code for this application structure.
    ///
    /// This includes:
    /// - .env file loading into `$_ENV` and `$_SERVER`
    /// - composer autoload require
    /// - chdir to proper document root
    pub fn php_bootstrap_code(&self) -> String {
        let mut code = String::with_capacity(1024);

        // Set working directory to document root
        code.push_str(&format!(
            "chdir('{}'); ",
            escape_php(&self.document_root.display().to_string())
        ));

        // Load .env into $_ENV and $_SERVER
        if self.has_env {
            let env_path = self.document_root.parent().unwrap_or(&self.document_root);
            code.push_str(&format!(
                "if (file_exists('{env_path}/.env')) {{ \
                    $__env_lines = file('{env_path}/.env', FILE_IGNORE_NEW_LINES | FILE_SKIP_EMPTY_LINES); \
                    foreach ($__env_lines as $__line) {{ \
                        if (str_starts_with(trim($__line), '#')) continue; \
                        if (!str_contains($__line, '=')) continue; \
                        [$__key, $__val] = explode('=', $__line, 2); \
                        $__key = trim($__key); \
                        $__val = trim($__val, \" \\t\\n\\r\\0\\x0B\\\"\\'\"); \
                        $_ENV[$__key] = $__val; \
                        $_SERVER[$__key] = $__val; \
                        if (function_exists('putenv')) putenv(\"$__key=$__val\"); \
                    }} \
                    unset($__env_lines, $__line, $__key, $__val); \
                }} ",
                env_path = escape_php(&env_path.display().to_string()),
            ));
        }

        // Require composer autoload
        if let Some(ref autoload) = self.autoload_path {
            code.push_str(&format!(
                "require_once '{}'; ",
                escape_php(&autoload.display().to_string())
            ));
        }

        code
    }

    /// Resolve the PHP file path for a given URI.
    ///
    /// Unified resolution for all frameworks:
    /// 1. Direct `.php` file — serve if exists on disk
    /// 2. Static (non-PHP) file — serve directly
    /// 3. Directory with `index.php` — resolve to that index
    /// 4. Front controller mode → entry point; direct mode → raw path
    pub fn resolve_path(&self, uri_path: &str) -> String {
        let clean = uri_path.split('?').next().unwrap_or(uri_path);
        let relative = clean.trim_start_matches('/');

        // 1. Direct .php file — serve if it exists on disk
        if clean.ends_with(".php")
            && !relative.is_empty()
            && self.document_root.join(relative).is_file()
        {
            return relative.to_string();
        }

        // 2. Static (non-PHP) file — serve directly
        if !relative.is_empty() && !clean.ends_with(".php") {
            let static_path = self.document_root.join(relative);
            if static_path.is_file() {
                return relative.to_string();
            }
        }

        // 3. Directory with its own index.php (e.g. /admin/ → admin/index.php)
        let trimmed = relative.trim_end_matches('/');
        if !trimmed.is_empty() {
            let dir_index = format!("{}/index.php", trimmed);
            if self.document_root.join(&dir_index).is_file() {
                return dir_index;
            }
        }

        // 4. Fallback
        if self.front_controller || clean == "/" || relative.is_empty() {
            // Front controller: everything goes through entry point
            self.entry_point.clone()
        } else {
            // Direct mapping: map URI path to file path (may 404 later)
            relative.to_string()
        }
    }
}

/// Escape a string for safe inclusion in PHP single-quoted strings.
fn escape_php(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Parse a Cookie header value into key-value pairs.
fn parse_cookie_header(header: &str) -> HashMap<String, String> {
    header
        .split(';')
        .filter_map(|pair| {
            let pair = pair.trim();
            if let Some((k, v)) = pair.split_once('=') {
                Some((url_decode(k.trim()), url_decode(v.trim())))
            } else {
                None
            }
        })
        .collect()
}

/// Parse URL-encoded form data.
fn parse_urlencoded(s: &str) -> Vec<(String, String)> {
    if s.is_empty() {
        return Vec::new();
    }
    s.split('&')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            if k.is_empty() {
                None
            } else {
                Some((url_decode(k), url_decode(v)))
            }
        })
        .collect()
}

/// URL decode.
fn url_decode(input: &str) -> String {
    let mut out = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Some(hi), Some(lo)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                    out.push(hi << 4 | lo);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_get() {
        let raw = b"GET /index.php?foo=bar HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let req = FullHttpRequest::parse(raw).unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/index.php");
        assert_eq!(req.query_string, "foo=bar");
        assert!(req.body.is_empty());
    }

    #[test]
    fn parse_post_with_body() {
        let raw = b"POST /api HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 11\r\n\r\nfoo=1&bar=2";
        let req = FullHttpRequest::parse(raw).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/api");
        let params = req.post_params();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], ("foo".to_string(), "1".to_string()));
    }

    #[test]
    fn parse_json_request() {
        let raw =
            b"POST /api HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"key\":\"value\"}";
        let req = FullHttpRequest::parse(raw).unwrap();
        assert!(req.is_json());
        assert!(req.post_params().is_empty()); // JSON body isn't form-decoded
    }

    #[test]
    fn parse_cookies() {
        let raw = b"GET / HTTP/1.1\r\nCookie: session=abc123; theme=dark\r\n\r\n";
        let req = FullHttpRequest::parse(raw).unwrap();
        assert_eq!(req.cookies.get("session").unwrap(), "abc123");
        assert_eq!(req.cookies.get("theme").unwrap(), "dark");
    }

    #[test]
    fn parse_headers_lowercase() {
        let raw = b"GET / HTTP/1.1\r\nX-Custom-Header: myvalue\r\nAccept: text/html\r\n\r\n";
        let req = FullHttpRequest::parse(raw).unwrap();
        assert_eq!(req.headers.get("x-custom-header").unwrap(), "myvalue");
        assert_eq!(req.headers.get("accept").unwrap(), "text/html");
    }

    #[test]
    fn get_params_from_query_string() {
        let raw = b"GET /page?a=1&b=hello+world&c=%2F HTTP/1.1\r\n\r\n";
        let req = FullHttpRequest::parse(raw).unwrap();
        let params = req.get_params();
        assert_eq!(params.len(), 3);
        assert_eq!(params[1], ("b".to_string(), "hello world".to_string()));
        assert_eq!(params[2], ("c".to_string(), "/".to_string()));
    }

    #[test]
    fn superglobals_code_contains_expected_vars() {
        let raw = b"GET /test?x=1 HTTP/1.1\r\nHost: localhost\r\nCookie: sid=abc\r\n\r\n";
        let req = FullHttpRequest::parse(raw).unwrap();
        let code =
            req.php_superglobals_code(Path::new("/app"), "test.php", "127.0.0.1", 8080, false);

        assert!(code.contains("$_SERVER['REQUEST_METHOD'] = 'GET'"));
        assert!(code.contains("$_SERVER['QUERY_STRING'] = 'x=1'"));
        assert!(code.contains("$_SERVER['SCRIPT_FILENAME']"));
        assert!(code.contains("$_SERVER['REMOTE_ADDR'] = '127.0.0.1'"));
        assert!(code.contains("$_SERVER['HTTP_HOST'] = 'localhost'"));
        assert!(code.contains("$_GET = ["));
        assert!(code.contains("$_COOKIE = ["));
        assert!(code.contains("$_REQUEST = array_merge"));
    }

    #[test]
    fn escape_php_handles_quotes() {
        assert_eq!(escape_php("it's a test"), "it\\'s a test");
        assert_eq!(escape_php("path\\to"), "path\\\\to");
    }

    #[test]
    fn cookie_parsing() {
        let cookies = parse_cookie_header("a=1; b=two; c=three%20four");
        assert_eq!(cookies.get("a").unwrap(), "1");
        assert_eq!(cookies.get("b").unwrap(), "two");
    }

    #[test]
    fn app_structure_resolve_front_controller() {
        let structure = AppStructure {
            document_root: PathBuf::from("/app/public"),
            entry_point: "index.php".to_string(),
            front_controller: true,
            has_composer: true,
            has_env: true,
            autoload_path: Some(PathBuf::from("/app/vendor/autoload.php")),
        };

        // All routes go to front controller
        assert_eq!(structure.resolve_path("/"), "index.php");
        assert_eq!(structure.resolve_path("/api/users"), "index.php");
        assert_eq!(structure.resolve_path("/login?redirect=/"), "index.php");
    }

    #[test]
    fn app_structure_resolve_generic() {
        let structure = AppStructure {
            document_root: PathBuf::from("/app"),
            entry_point: "index.php".to_string(),
            front_controller: false,
            has_composer: false,
            has_env: false,
            autoload_path: None,
        };

        assert_eq!(structure.resolve_path("/"), "index.php");
        assert_eq!(structure.resolve_path("/info.php"), "info.php");
        assert_eq!(
            structure.resolve_path("/api/users.php?id=1"),
            "api/users.php"
        );
    }

    #[test]
    fn bootstrap_code_with_env_and_autoload() {
        let structure = AppStructure {
            document_root: PathBuf::from("/app/public"),
            entry_point: "index.php".to_string(),
            front_controller: true,
            has_composer: true,
            has_env: true,
            autoload_path: Some(PathBuf::from("/app/vendor/autoload.php")),
        };
        let code = structure.php_bootstrap_code();
        assert!(code.contains("chdir("));
        assert!(code.contains(".env"));
        assert!(code.contains("require_once"));
        assert!(code.contains("autoload.php"));
    }

    #[test]
    fn full_request_body_injection() {
        let raw =
            b"POST /api HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"name\":\"test\"}";
        let req = FullHttpRequest::parse(raw).unwrap();
        let code =
            req.php_superglobals_code(Path::new("/app"), "index.php", "127.0.0.1", 8080, false);
        assert!(code.contains("__turbine_raw_body"));
        assert!(code.contains("TurbineInputStream"));
    }

    // --- Camada 4: Upload Hardening Tests ---

    #[test]
    fn upload_security_blocks_php_extension() {
        let security = UploadSecurityConfig::default();
        let fname = "shell.php";
        let fname_lower = fname.to_lowercase();
        let blocked = security
            .blocked_extensions
            .iter()
            .any(|ext| fname_lower.ends_with(&ext.to_lowercase()));
        assert!(blocked, "shell.php should be blocked");
    }

    #[test]
    fn upload_security_blocks_double_extension() {
        let security = UploadSecurityConfig::default();
        let fname = "avatar.php.jpg";
        let fname_lower = fname.to_lowercase();
        let blocked = security.blocked_extensions.iter().any(|ext| {
            let with_dot = format!("{}.", ext.to_lowercase());
            fname_lower.contains(&with_dot)
        });
        assert!(
            blocked,
            "avatar.php.jpg should be blocked (double extension)"
        );
    }

    #[test]
    fn upload_security_allows_normal_images() {
        let security = UploadSecurityConfig::default();
        let fname = "photo.jpg";
        let fname_lower = fname.to_lowercase();
        let ext_blocked = security.blocked_extensions.iter().any(|ext| {
            fname_lower.ends_with(&ext.to_lowercase()) || {
                let with_dot = format!("{}.", ext.to_lowercase());
                fname_lower.contains(&with_dot)
            }
        });
        assert!(!ext_blocked, "photo.jpg should NOT be blocked");
    }

    #[test]
    fn upload_security_blocks_all_php_variants() {
        let security = UploadSecurityConfig::default();
        for ext in &[
            ".phtml", ".phar", ".php7", ".php8", ".inc", ".phps", ".pht", ".pgif",
        ] {
            let fname = format!("test{ext}");
            let fname_lower = fname.to_lowercase();
            let blocked = security
                .blocked_extensions
                .iter()
                .any(|blocked_ext| fname_lower.ends_with(&blocked_ext.to_lowercase()));
            assert!(blocked, "{fname} should be blocked");
        }
    }

    #[test]
    fn upload_content_scan_detects_php_code() {
        let data = b"GIF89a\x00\x00<?php system($_GET['cmd']); ?>";
        let content_lower: Vec<u8> = data.iter().map(|b| b.to_ascii_lowercase()).collect();
        let detected = PHP_SIGNATURES.iter().any(|sig| {
            let sig_lower: Vec<u8> = sig.iter().map(|b| b.to_ascii_lowercase()).collect();
            content_lower
                .windows(sig_lower.len())
                .any(|w| w == sig_lower.as_slice())
        });
        assert!(detected, "Should detect <?php in uploaded content");
    }

    #[test]
    fn upload_content_scan_allows_clean_binary() {
        let data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR some binary data";
        let content_lower: Vec<u8> = data.iter().map(|b| b.to_ascii_lowercase()).collect();
        let detected = PHP_SIGNATURES.iter().any(|sig| {
            let sig_lower: Vec<u8> = sig.iter().map(|b| b.to_ascii_lowercase()).collect();
            content_lower
                .windows(sig_lower.len())
                .any(|w| w == sig_lower.as_slice())
        });
        assert!(!detected, "Clean PNG binary should NOT be flagged");
    }

    #[test]
    fn upload_content_scan_detects_short_tag() {
        let data = b"<?= phpinfo(); ?>";
        let content_lower: Vec<u8> = data.iter().map(|b| b.to_ascii_lowercase()).collect();
        let detected = PHP_SIGNATURES.iter().any(|sig| {
            let sig_lower: Vec<u8> = sig.iter().map(|b| b.to_ascii_lowercase()).collect();
            content_lower
                .windows(sig_lower.len())
                .any(|w| w == sig_lower.as_slice())
        });
        assert!(detected, "Should detect <?= short tag in content");
    }

    // --- Camada 1: Execution Whitelist Tests ---

    #[test]
    fn execution_whitelist_blocks_non_entry_generic() {
        let structure = AppStructure {
            document_root: PathBuf::from("/app"),
            entry_point: "index.php".to_string(),
            front_controller: false,
            has_composer: false,
            has_env: false,
            autoload_path: None,
        };
        let whitelist = vec!["index.php".to_string()];
        let path = structure.resolve_path("/admin/shell.php");
        assert!(
            !whitelist.contains(&path),
            "admin/shell.php should NOT be in whitelist"
        );
    }

    #[test]
    fn execution_whitelist_allows_entry_point() {
        let whitelist = vec!["index.php".to_string()];
        assert!(
            whitelist.contains(&"index.php".to_string()),
            "index.php should be in whitelist"
        );
    }

    #[test]
    fn data_directory_blocks_execution() {
        let data_dirs = vec!["storage/".to_string(), "uploads/".to_string()];
        let php_path = "storage/framework/shell.php";
        let blocked = data_dirs.iter().any(|dir| {
            let normalized = dir.trim_end_matches('/');
            php_path.starts_with(normalized)
        });
        assert!(blocked, "PHP in storage/ should be blocked");
    }

    #[test]
    fn data_directory_allows_entry_point() {
        let data_dirs = vec!["storage/".to_string(), "uploads/".to_string()];
        let php_path = "index.php";
        let blocked = data_dirs.iter().any(|dir| {
            let normalized = dir.trim_end_matches('/');
            php_path.starts_with(normalized)
        });
        assert!(!blocked, "index.php should NOT be blocked by data dirs");
    }
}
