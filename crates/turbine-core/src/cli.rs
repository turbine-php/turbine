//! CLI argument parsing — `turbine serve`, `turbine status`, `turbine cache:clear`, etc.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "turbine",
    version,
    about = "Turbine Runtime — high-performance PHP runtime built in Rust",
    long_about = "Runtime PHP de alta performance com segurança nativa, construído em Rust.\n\n\
                  Executa aplicações PHP com worker pool persistente, OPcode cache,\n\
                  sandbox de segurança (heurístico, não-WAF) e response cache automático."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Start the HTTP server (default if no subcommand given)
    Serve {
        /// Override listen address (e.g. 127.0.0.1:8080)
        #[arg(short, long)]
        listen: Option<String>,

        /// Override number of worker processes
        #[arg(short, long)]
        workers: Option<usize>,

        /// Path to turbine.toml configuration file
        #[arg(short, long)]
        config: Option<String>,

        /// Application root directory (default: current directory)
        #[arg(short, long)]
        root: Option<String>,

        /// Path to PEM certificate chain file (enables TLS)
        #[arg(long)]
        tls_cert: Option<String>,

        /// Path to PEM private key file (enables TLS)
        #[arg(long)]
        tls_key: Option<String>,

        /// Request execution timeout in seconds (0 = no timeout)
        #[arg(long)]
        request_timeout: Option<u64>,

        /// Path to access log file
        #[arg(long)]
        access_log: Option<String>,
    },

    /// Show runtime status (connects to running server)
    Status {
        /// Server address to query
        #[arg(short, long, default_value = "127.0.0.1:8080")]
        address: String,
    },

    /// Clear the response cache
    #[command(name = "cache:clear")]
    CacheClear {
        /// Server address to query
        #[arg(short, long, default_value = "127.0.0.1:8080")]
        address: String,
    },

    /// Show current configuration
    Config,

    /// Show PHP engine information
    Info,

    /// Initialize a new turbine.toml in the current directory
    Init,

    /// Validate turbine.toml configuration (check for errors and warnings)
    Check {
        /// Path to turbine.toml configuration file
        #[arg(short, long)]
        config: Option<String>,
    },
}
