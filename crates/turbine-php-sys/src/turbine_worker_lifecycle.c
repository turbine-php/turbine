/**
 * turbine_worker_lifecycle.c
 *
 * Light-weight per-request startup/shutdown for persistent PHP workers.
 *
 * Unlike php_request_startup/shutdown, these functions DO NOT destroy the
 * PHP global variable table (EG(symbol_table)), so objects created during
 * bootstrap survive across requests.
 *
 * Call order per request:
 *   turbine_worker_request_startup()  → handle request → turbine_worker_request_shutdown()
 *
 * One-time startup per worker process:
 *   php_embed_init()  →  turbine_worker_boot()  →  [request loop]  →  php_embed_shutdown()
 */

/* Silence GCC unused-function warnings for PHP internal static helpers */
#pragma GCC diagnostic ignored "-Wunused-function"

#include <php.h>
#include <main/php_main.h>
#include <main/SAPI.h>
#include <php_ini.h>
#include <php_output.h>
#include <zend_exceptions.h>
#include <zend_hash.h>

#ifdef HAVE_PHP_SESSION
#include <ext/session/php_session.h>
#endif

/* ───────────────────────────────────────────────────────────────── */
/* Worker Boot — called once in the forked child before the loop.   */
/* Sets up the persistent PHP environment (no request cycle yet).   */
/* ───────────────────────────────────────────────────────────────── */
int turbine_worker_boot(void) {
    /* php_embed_init already called by master before fork.            */
    /* php_request_startup() initialises per-request state; call once  */
    /* for the bootstrap phase (require autoload, create kernel, etc). */
    /* The first real request will call turbine_worker_request_startup. */
    return php_request_startup();
}

/* ───────────────────────────────────────────────────────────────── */
/* Light request shutdown — flush output and reset request state    */
/* WITHOUT destroying PHP global variables (symbol table).          */
/* ───────────────────────────────────────────────────────────────── */
void turbine_worker_request_shutdown(void) {
    /* Flush and close output buffers (send any remaining echo output). */
    zend_try { php_output_end_all(); }
    zend_end_try();

    zend_try { php_output_deactivate(); }
    zend_end_try();

#ifdef HAVE_PHP_SESSION
    /* Close session properly so data is written and not leaked. */
    if (PS(session_status) == php_session_active) {
        zend_try { php_session_flush(1); }
        zend_end_try();
    }
#endif

    /* Reset SAPI request context (cookies, headers, response code). */
    zend_try { sapi_deactivate(); }
    zend_end_try();

    /* Reset memory limit for next request. */
    zend_set_memory_limit(PG(memory_limit));

    /* NOTE: We intentionally do NOT call zend_hash_clean(&EG(symbol_table))
     * or php_module_shutdown(). This preserves PHP variables created during
     * bootstrap ($app, $kernel, etc.) across requests. */
}

/* ───────────────────────────────────────────────────────────────── */
/* Light request startup — re-activate SAPI and output subsystem.   */
/* Called at the beginning of each request in the worker loop.      */
/* ───────────────────────────────────────────────────────────────── */
int turbine_worker_request_startup(void) {
    /* Update SG(server_context) and SG(request_info) for the new request. */
    SG(server_context) = (void *)1;
    SG(sapi_headers).http_response_code = 200;

    zend_try {
        /* Re-enable output buffering for this request. */
        php_output_activate();

        PG(header_is_being_sent) = 0;
        PG(connection_status)    = PHP_CONNECTION_NORMAL;

        /* Re-activate SAPI (sets up cookies, auth, etc.). */
        sapi_activate();

        /* Optionally expose PHP version header. */
        if (PG(expose_php)) {
            sapi_add_header(SAPI_PHP_VERSION_HEADER,
                            sizeof(SAPI_PHP_VERSION_HEADER) - 1, 1);
        }

        /* Re-arm auto-globals ($_GET, $_POST, $_SERVER, $_COOKIE).
         * We skip $_ENV (it doesn't change between requests). */
        zend_auto_global *ag;
        zend_string *env_key = ZSTR_KNOWN(ZEND_STR_AUTOGLOBAL_ENV);
        ZEND_HASH_MAP_FOREACH_PTR(CG(auto_globals), ag) {
            if (ag->name != env_key && ag->auto_global_callback) {
                ag->armed = ag->auto_global_callback(ag->name);
            }
        }
        ZEND_HASH_FOREACH_END();

        /* Delete $_SESSION from symbol table so it doesn't leak across requests. */
        zend_hash_str_del(&EG(symbol_table), "_SESSION", sizeof("_SESSION") - 1);
    }
    zend_catch { return FAILURE; }
    zend_end_try();

    SG(sapi_started) = 1;
    return SUCCESS;
}

/* ───────────────────────────────────────────────────────────────── */
/* Worker shutdown — called once when the worker process exits.     */
/* ───────────────────────────────────────────────────────────────── */
void turbine_worker_shutdown(void) {
    /* Full request shutdown to flush everything before process exit. */
    php_request_shutdown(NULL);
}
