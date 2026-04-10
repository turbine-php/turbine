//! Embedded HTML dashboard for `/_/dashboard`.
//!
//! The HTML/CSS/JS lives in `templates/dashboard.html` and is compiled into
//! the binary via `include_str!`. Two placeholders are substituted at runtime:
//!
//! * `%%LISTEN%%`        — server listen address (e.g. `127.0.0.1:8080`)
//! * `%%AUTH_REQUIRED%%` — `true` or `false` (JS boolean)

const TEMPLATE: &str = include_str!("templates/dashboard.html");

/// Returns the complete dashboard HTML page with runtime values substituted.
pub fn dashboard_html(listen: &str, auth_required: bool) -> String {
    TEMPLATE.replace("%%LISTEN%%", listen).replace(
        "%%AUTH_REQUIRED%%",
        if auth_required { "true" } else { "false" },
    )
}
