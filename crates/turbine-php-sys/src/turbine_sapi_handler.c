/**
 * turbine_sapi_handler.c
 *
 * Native SAPI request handling for Turbine classic workers.
 *
 * Instead of sending PHP code strings via zend_eval_string() (which bypasses
 * OPcache and recompiles every request), this module provides php-fpm-style
 * execution:
 *
 *   1. Populate SG(request_info) with HTTP metadata
 *   2. Call php_request_startup() — PHP auto-populates $_SERVER, $_GET, etc.
 *   3. Call php_execute_script() — uses OPcache, standard Zend execution
 *   4. Call php_request_shutdown() — flushes output
 *
 * The SAPI callbacks (read_post, read_cookies, register_server_variables)
 * are overridden to read from Turbine's per-request buffers, not stdin.
 */

#pragma GCC diagnostic ignored "-Wunused-function"

#include <php.h>
#include <main/php_main.h>
#include <main/SAPI.h>
#include <main/php_variables.h>
#include <Zend/zend_stream.h>
#include <string.h>

/* ───────────────────────────────────────────────────────────────── */
/* Per-request state (set by turbine_sapi_set_request before        */
/* php_request_startup).                                            */
/*                                                                  */
/* These are _Thread_local so that thread-mode workers (ZTS PHP)    */
/* each get their own copy. In process mode (fork), each child has  */
/* a single thread so _Thread_local is equivalent to plain static.  */
/* ───────────────────────────────────────────────────────────────── */

/* POST body */
static _Thread_local const char *turbine_post_body      = NULL;
static _Thread_local size_t      turbine_post_body_len  = 0;
static _Thread_local size_t      turbine_post_body_read = 0;

/* Cookie header value */
static _Thread_local const char *turbine_cookie_data = NULL;

/* Headers as key-value pairs for $_SERVER['HTTP_*'] */
#define TURBINE_MAX_HEADERS 128

static _Thread_local const char *turbine_header_keys[TURBINE_MAX_HEADERS];
static _Thread_local size_t      turbine_header_key_lens[TURBINE_MAX_HEADERS];
static _Thread_local const char *turbine_header_vals[TURBINE_MAX_HEADERS];
static _Thread_local size_t      turbine_header_val_lens[TURBINE_MAX_HEADERS];
static _Thread_local int         turbine_header_count = 0;

/* Extra server variables */
static _Thread_local const char *turbine_script_filename = NULL;
static _Thread_local const char *turbine_document_root   = NULL;
static _Thread_local const char *turbine_remote_addr     = NULL;
static _Thread_local int         turbine_remote_port     = 0;
static _Thread_local int         turbine_server_port     = 0;
static _Thread_local int         turbine_is_https        = 0;
static _Thread_local const char *turbine_path_info       = NULL;
static _Thread_local const char *turbine_script_name     = NULL;

/* ───────────────────────────────────────────────────────────────── */
/* SAPI callbacks                                                   */
/* ───────────────────────────────────────────────────────────────── */

/**
 * read_post — provide POST body to PHP's input processing.
 * Called by php_request_startup → sapi_read_post_data.
 */
static size_t turbine_sapi_read_post(char *buffer, size_t count_bytes) {
    if (!turbine_post_body || turbine_post_body_read >= turbine_post_body_len) {
        return 0;
    }
    size_t remaining = turbine_post_body_len - turbine_post_body_read;
    size_t to_read   = (count_bytes < remaining) ? count_bytes : remaining;
    memcpy(buffer, turbine_post_body + turbine_post_body_read, to_read);
    turbine_post_body_read += to_read;
    return to_read;
}

/**
 * read_cookies — provide Cookie header to PHP.
 * Called by php_request_startup → sapi_activate → php_default_treat_data.
 */
static char *turbine_sapi_read_cookies(void) {
    return (char *)turbine_cookie_data;
}

/**
 * Convert a header name to HTTP_UPPER_CASE format.
 * "Content-Type" → "HTTP_CONTENT_TYPE"
 * Buffer must be at least 5 + name_len + 1 bytes. `name` does not need
 * to be null-terminated; `name_len` is used directly.
 */
static void header_to_server_key(const char *name, size_t name_len, char *buf, size_t buf_size) {
    size_t prefix_len = 5; /* "HTTP_" */
    if (prefix_len + name_len >= buf_size) {
        name_len = buf_size - prefix_len - 1;
    }
    memcpy(buf, "HTTP_", 5);
    for (size_t i = 0; i < name_len; i++) {
        char c = name[i];
        if (c == '-') {
            buf[prefix_len + i] = '_';
        } else if (c >= 'a' && c <= 'z') {
            buf[prefix_len + i] = c - 32; /* toupper */
        } else {
            buf[prefix_len + i] = c;
        }
    }
    buf[prefix_len + name_len] = '\0';
}

/**
 * register_server_variables — populate $_SERVER from request metadata.
 * Called by php_request_startup after SAPI activation.
 */
static void turbine_sapi_register_variables(zval *track_vars_array) {
    /* Import standard environment variables ($_ENV-derived entries in $_SERVER) */
    php_import_environment_variables(track_vars_array);

    /* Core request variables */
    if (SG(request_info).request_method) {
        php_register_variable_safe("REQUEST_METHOD",
            (char *)SG(request_info).request_method,
            strlen(SG(request_info).request_method), track_vars_array);
    }
    if (SG(request_info).request_uri) {
        php_register_variable_safe("REQUEST_URI",
            (char *)SG(request_info).request_uri,
            strlen(SG(request_info).request_uri), track_vars_array);
    }
    if (SG(request_info).query_string) {
        php_register_variable_safe("QUERY_STRING",
            (char *)SG(request_info).query_string,
            strlen(SG(request_info).query_string), track_vars_array);
    }
    if (SG(request_info).content_type) {
        php_register_variable_safe("CONTENT_TYPE",
            (char *)SG(request_info).content_type,
            strlen(SG(request_info).content_type), track_vars_array);
    }
    if (SG(request_info).content_length >= 0) {
        char cl_buf[32];
        snprintf(cl_buf, sizeof(cl_buf), "%ld", (long)SG(request_info).content_length);
        php_register_variable_safe("CONTENT_LENGTH", cl_buf, strlen(cl_buf), track_vars_array);
    }

    /* Turbine-specific server variables */
    if (turbine_script_filename) {
        php_register_variable_safe("SCRIPT_FILENAME",
            (char *)turbine_script_filename,
            strlen(turbine_script_filename), track_vars_array);
    }
    if (turbine_document_root) {
        php_register_variable_safe("DOCUMENT_ROOT",
            (char *)turbine_document_root,
            strlen(turbine_document_root), track_vars_array);
    }
    if (turbine_remote_addr) {
        php_register_variable_safe("REMOTE_ADDR",
            (char *)turbine_remote_addr,
            strlen(turbine_remote_addr), track_vars_array);
    }
    if (turbine_script_name) {
        php_register_variable_safe("SCRIPT_NAME",
            (char *)turbine_script_name,
            strlen(turbine_script_name), track_vars_array);
    }
    if (turbine_path_info) {
        php_register_variable_safe("PATH_INFO",
            (char *)turbine_path_info,
            strlen(turbine_path_info), track_vars_array);
        php_register_variable_safe("PHP_SELF",
            (char *)turbine_path_info,
            strlen(turbine_path_info), track_vars_array);
    }

    {
        char port_buf[16];
        snprintf(port_buf, sizeof(port_buf), "%d", turbine_remote_port);
        php_register_variable_safe("REMOTE_PORT", port_buf, strlen(port_buf), track_vars_array);
        snprintf(port_buf, sizeof(port_buf), "%d", turbine_server_port);
        php_register_variable_safe("SERVER_PORT", port_buf, strlen(port_buf), track_vars_array);
    }

    php_register_variable_safe("SERVER_SOFTWARE", "Turbine", 7, track_vars_array);
    php_register_variable_safe("SERVER_NAME", "localhost", 9, track_vars_array);
    php_register_variable_safe("SERVER_PROTOCOL", "HTTP/1.1", 8, track_vars_array);
    php_register_variable_safe("GATEWAY_INTERFACE", "CGI/1.1", 7, track_vars_array);

    if (turbine_is_https) {
        php_register_variable_safe("HTTPS", "on", 2, track_vars_array);
        php_register_variable_safe("REQUEST_SCHEME", "https", 5, track_vars_array);
    } else {
        php_register_variable_safe("REQUEST_SCHEME", "http", 4, track_vars_array);
    }

    /* HTTP headers → HTTP_UPPER_CASE */
    char key_buf[256];
    for (int i = 0; i < turbine_header_count; i++) {
        const char *name_ptr = turbine_header_keys[i];
        const char *val_ptr  = turbine_header_vals[i];
        size_t name_len      = turbine_header_key_lens[i];
        size_t val_len       = turbine_header_val_lens[i];
        if (!name_ptr || !val_ptr || name_len == 0) continue;

        /* Content-Type and Content-Length are already set above (without HTTP_ prefix) */
        if (name_len == 12 && strncasecmp(name_ptr, "Content-Type", 12) == 0) continue;
        if (name_len == 14 && strncasecmp(name_ptr, "Content-Length", 14) == 0) continue;

        header_to_server_key(name_ptr, name_len, key_buf, sizeof(key_buf));
        php_register_variable_safe(key_buf, (char *)val_ptr, val_len, track_vars_array);
    }
}

/* ───────────────────────────────────────────────────────────────── */
/* Public API                                                       */
/* ───────────────────────────────────────────────────────────────── */

/**
 * Install Turbine's SAPI hooks into the embed module.
 *
 * Must be called once per worker process (after fork, before first request).
 * Overrides read_post, read_cookies, and register_server_variables.
 */
void turbine_sapi_install_hooks(void) {
    sapi_module.read_post                = turbine_sapi_read_post;
    sapi_module.read_cookies             = turbine_sapi_read_cookies;
    sapi_module.register_server_variables = turbine_sapi_register_variables;
}

/**
 * Populate SG(request_info) with HTTP request metadata.
 *
 * Must be called BEFORE php_request_startup() so that PHP's standard
 * mechanisms populate $_SERVER, $_GET, $_POST, $_COOKIE automatically.
 */
void turbine_sapi_set_request(
    const char *method,
    const char *uri,
    const char *query_string,
    const char *content_type,
    long        content_length,
    const char *cookie_data,
    const char *script_filename,
    const char *document_root,
    const char *remote_addr,
    int         remote_port,
    int         server_port,
    int         is_https,
    const char *path_info,
    const char *script_name,
    /* POST body */
    const char *post_body,
    size_t      post_body_len,
    /* Headers — values carry explicit lengths so Rust does not need to
       allocate null-terminated CStrings per header. */
    int         header_count,
    const char **header_keys,
    const size_t *header_key_lens,
    const char **header_vals,
    const size_t *header_val_lens
) {
    /* Populate SG(request_info) — PHP reads this during request_startup */
    SG(request_info).request_method  = method;
    SG(request_info).request_uri     = (char *)uri;
    SG(request_info).query_string    = (char *)query_string;
    SG(request_info).content_type    = content_type;
    SG(request_info).content_length  = content_length;
    SG(request_info).path_translated = (char *)script_filename;

    /* Store extra data for register_server_variables callback */
    turbine_script_filename = script_filename;
    turbine_document_root   = document_root;
    turbine_remote_addr     = remote_addr;
    turbine_remote_port     = remote_port;
    turbine_server_port     = server_port;
    turbine_is_https        = is_https;
    turbine_path_info       = path_info;
    turbine_script_name     = script_name;

    /* Store POST body for read_post callback */
    turbine_post_body      = post_body;
    turbine_post_body_len  = post_body_len;
    turbine_post_body_read = 0;

    /* Store cookie data for read_cookies callback */
    turbine_cookie_data = cookie_data;

    /* Store headers for register_server_variables */
    turbine_header_count = (header_count > TURBINE_MAX_HEADERS)
                           ? TURBINE_MAX_HEADERS : header_count;
    for (int i = 0; i < turbine_header_count; i++) {
        turbine_header_keys[i]     = header_keys[i];
        turbine_header_key_lens[i] = header_key_lens ? header_key_lens[i] : 0;
        turbine_header_vals[i]     = header_vals[i];
        turbine_header_val_lens[i] = header_val_lens ? header_val_lens[i] : 0;
    }

    /* Reset SAPI state for new request */
    SG(server_context) = (void *)1;
    SG(sapi_headers).http_response_code = 200;
}

/**
 * Execute a PHP script using the standard Zend Engine execution path.
 *
 * This uses php_execute_script() which:
 *   - Resolves through OPcache (cached bytecode)
 *   - Uses the standard Zend VM
 *   - Is the same path php-fpm uses
 *
 * Returns SUCCESS (0) or FAILURE (-1).
 */
int turbine_execute_script(const char *filename) {
    zend_file_handle file_handle;
    zend_stream_init_filename(&file_handle, filename);

    int ret = php_execute_script(&file_handle);

    zend_destroy_file_handle(&file_handle);
    return ret ? SUCCESS : FAILURE;
}

/* ───────────────────────────────────────────────────────────────── */
/* ZTS-safe sapi_globals accessors                                  */
/*                                                                  */
/* In NTS builds, sapi_globals is a plain C global. In ZTS builds   */
/* it lives in TSRM thread-local storage and must be accessed via   */
/* the SG() macro. These wrappers let Rust call into sapi_globals   */
/* without needing to resolve the symbol directly.                  */
/* ───────────────────────────────────────────────────────────────── */

/**
 * Read the HTTP response code from SG(sapi_headers).http_response_code.
 */
int turbine_read_response_code(void) {
    return SG(sapi_headers).http_response_code;
}

/**
 * Register a file path as an uploaded file in SG(rfc1867_uploaded_files).
 *
 * This makes is_uploaded_file() and move_uploaded_file() recognize the file.
 */
void turbine_register_uploaded_file_c(const char *path, size_t path_len) {
    if (!SG(rfc1867_uploaded_files)) {
        HashTable *ht = (HashTable *)malloc(sizeof(HashTable));
        if (!ht) return;
        zend_hash_init(ht, 8, NULL, NULL, 0);
        SG(rfc1867_uploaded_files) = ht;
    }
    zend_hash_str_add_empty_element(SG(rfc1867_uploaded_files), path, path_len);
}
