//! PHP bootstrap assembly.
//!
//! Prepends the opt-in `turbine_*()` helper functions (defined in
//! [`crate::features`]) to the application's own `php_bootstrap_code()`
//! based on which primitives are enabled in [`config::RuntimeConfig`].
//!
//! Kept in its own module so the `cmd_serve` path stays readable —
//! adding a new primitive now means: write the helpers in `features`,
//! add one branch here, done.

use tracing::info;

use crate::config::RuntimeConfig;
use crate::features;

/// Build the final PHP bootstrap string by prepending each enabled
/// feature's helper block (in reverse order of desired precedence —
/// each `format!` *prepends* so the most recently added block runs
/// first once we're done).
pub fn build_php_bootstrap(config: &RuntimeConfig, app_bootstrap: String) -> String {
    let mut out = app_bootstrap;

    if config.structured_logging.enabled {
        out = format!("{}{}", features::php_turbine_log_function(), out);
        info!("PHP turbine_log() function injected into bootstrap");
    }

    if config.shared_table.enabled {
        out = format!("{}{}", features::php_turbine_table_functions(), out);
        info!("PHP turbine_table_*() helpers injected into bootstrap");
    }

    if config.task_queue.enabled {
        out = format!("{}{}", features::php_turbine_task_functions(), out);
        info!("PHP turbine_task_*() helpers injected into bootstrap");
    }

    if config.websocket.enabled {
        out = format!("{}{}", features::php_turbine_ws_functions(), out);
        info!("PHP turbine_ws_*() helpers injected into bootstrap");
    }

    if config.async_io.enabled {
        out = format!("{}{}", features::php_turbine_async_functions(), out);
        info!("PHP turbine_async_*() helpers injected into bootstrap");
    }

    out
}
