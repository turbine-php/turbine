use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecurityError {
    #[error("SQL injection detected: {pattern}")]
    SqlInjection { pattern: String },

    #[error("XSS detected in output")]
    Xss,

    #[error("Code injection detected: {pattern}")]
    CodeInjection { pattern: String },

    #[error("Rate limit exceeded for {ip}")]
    RateLimited { ip: String },

    #[error("Scanning behaviour detected from {ip}")]
    ScanningDetected { ip: String },
}
