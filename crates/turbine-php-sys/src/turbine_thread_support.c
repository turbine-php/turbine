/**
 * turbine_thread_support.c
 *
 * Thread-mode support for Turbine's dual worker backend.
 *
 * When PHP is compiled with ZTS (Zend Thread Safety), multiple threads can
 * each run their own PHP interpreter context via TSRM. This file provides:
 *
 *   - turbine_php_is_thread_safe() — runtime ZTS detection
 *   - turbine_thread_init()        — per-thread TSRM context creation
 *   - turbine_thread_cleanup()     — per-thread TSRM context destruction
 *
 * When PHP is NTS (non-thread-safe), these functions are safe to call but
 * turbine_php_is_thread_safe() returns 0, and the init/cleanup are no-ops.
 *
 * The TSRM API:
 *   - ts_resource(0) to allocate thread-local storage for PHP globals
 *   - ts_free_thread() to release it
 */

#pragma GCC diagnostic ignored "-Wunused-function"

#include <php.h>
#include <main/php_main.h>
#include <main/SAPI.h>

#ifdef ZTS
#include <TSRM/TSRM.h>
/* For ZEND_TSRMLS_CACHE_UPDATE — updates the thread-local cache of
 * executor_globals, compiler_globals, etc. */
ZEND_TSRMLS_CACHE_DEFINE()
#endif

/**
 * Check whether PHP was compiled with Zend Thread Safety (ZTS).
 *
 * Returns 1 if ZTS is enabled, 0 if NTS.
 * Thread mode workers REQUIRE ZTS — starting thread mode with NTS PHP
 * will cause undefined behavior (data races on PHP globals).
 */
int turbine_php_is_thread_safe(void) {
#ifdef ZTS
    return 1;
#else
    return 0;
#endif
}

/**
 * Initialize a TSRM interpreter context for the calling thread.
 *
 * Must be called from each worker thread BEFORE any PHP operations.
 * In ZTS mode, this allocates thread-local storage for all PHP globals
 * (SG, EG, PG, CG) so they are independent per thread.
 *
 * In NTS mode, this is a no-op (returns 0 = success).
 *
 * Returns 0 on success, -1 on failure.
 */
int turbine_thread_init(void) {
#ifdef ZTS
    /* Allocate TSRM resources for this thread.
     * ts_resource(0) triggers TSRM to create thread-local copies of all
     * registered resource IDs (EG, SG, PG, CG, etc.) for the calling thread.
     */
    (void)ts_resource(0);

    /* Update the thread-local cache macros so EG(), SG(), PG() etc.
     * resolve to this thread's storage. */
    ZEND_TSRMLS_CACHE_UPDATE();

    return 0;
#else
    return 0;
#endif
}

/**
 * Clean up the TSRM interpreter context for the calling thread.
 *
 * Must be called from each worker thread AFTER all PHP operations are done,
 * just before the thread exits.
 *
 * In NTS mode, this is a no-op.
 */
void turbine_thread_cleanup(void) {
#ifdef ZTS
    /* Free all thread-local TSRM resources for the calling thread. */
    ts_free_thread();
#endif
}
