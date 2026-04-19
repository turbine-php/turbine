//! `turbine` CLI subcommand handlers (everything except `serve`).
//!
//! Each function is a drop-in for the original top-level `fn cmd_*` in
//! `main.rs`.  `cmd_serve` stays where it is because it is deeply
//! entwined with runtime bootstrap and TLS wiring.

use crate::config::RuntimeConfig;
use turbine_engine::PhpEngine;

/// `turbine init` — generate a default `turbine.toml`.
pub fn cmd_init() {
    let path = std::env::current_dir()
        .unwrap_or_default()
        .join("turbine.toml");
    if path.exists() {
        eprintln!("turbine.toml already exists");
        std::process::exit(1);
    }
    std::fs::write(&path, RuntimeConfig::template()).expect("Failed to write turbine.toml");
    println!("Created {}", path.display());
}

/// `turbine check` — validate `turbine.toml` configuration.
pub fn cmd_check(config_path: Option<String>) {
    let path = config_path.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_default()
            .join("turbine.toml")
            .to_string_lossy()
            .to_string()
    });

    if !std::path::Path::new(&path).exists() {
        eprintln!("\x1b[31m✗\x1b[0m Config file not found: {path}");
        std::process::exit(1);
    }

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("\x1b[31m✗\x1b[0m Failed to read {path}: {e}");
            std::process::exit(1);
        }
    };

    let config: RuntimeConfig = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("\x1b[31m✗\x1b[0m TOML parse error in {path}:");
            eprintln!("  {e}");
            std::process::exit(1);
        }
    };

    let (errors, warnings) = config.check();

    println!("\x1b[1mTurbine Configuration Check\x1b[0m");
    println!("  File: {path}");
    println!();

    println!("\x1b[1mSettings:\x1b[0m");
    println!("  workers          = {}", config.server.workers);
    println!("  worker_mode      = {}", config.server.worker_mode);
    println!(
        "  persistent       = {}",
        config.server.persistent_workers.unwrap_or(false)
    );
    println!("  listen           = {}", config.server.listen);
    println!("  request_timeout  = {}s", config.server.request_timeout);
    println!("  max_requests     = {}", config.server.worker_max_requests);
    if let Some(t) = config.server.tokio_worker_threads {
        println!("  tokio_threads    = {t}");
    }
    println!("  security         = {}", config.security.enabled);
    println!("  compression      = {}", config.compression.enabled);
    println!("  cache            = {}", config.cache.enabled);
    println!("  tls              = {}", config.server.tls.enabled);
    if !config.virtual_hosts.is_empty() {
        println!("  virtual_hosts    = {}", config.virtual_hosts.len());
        for vhost in &config.virtual_hosts {
            let aliases = if vhost.aliases.is_empty() {
                String::new()
            } else {
                format!(" (+ {})", vhost.aliases.join(", "))
            };
            println!("    {} → {}{}", vhost.domain, vhost.root, aliases);
        }
    }
    println!();

    let mut has_issues = false;

    if !errors.is_empty() {
        has_issues = true;
        println!("\x1b[31m✗ {} error(s):\x1b[0m", errors.len());
        for e in &errors {
            println!("  \x1b[31m•\x1b[0m {e}");
        }
        println!();
    }

    if !warnings.is_empty() {
        has_issues = true;
        println!("\x1b[33m⚠ {} warning(s):\x1b[0m", warnings.len());
        for w in &warnings {
            println!("  \x1b[33m•\x1b[0m {w}");
        }
        println!();
    }

    if has_issues {
        if !errors.is_empty() {
            eprintln!("\x1b[31m✗ Configuration has errors that must be fixed.\x1b[0m");
            std::process::exit(1);
        } else {
            println!("\x1b[33m⚠ Configuration is valid but has warnings.\x1b[0m");
        }
    } else {
        println!("\x1b[32m✓ Configuration is valid. No errors or warnings.\x1b[0m");
    }
}

/// `turbine config` — display current configuration.
pub fn cmd_config() {
    let config = RuntimeConfig::load();
    println!("{config:#?}");
}

/// `turbine info` — show PHP engine information.
pub fn cmd_info() {
    let engine = match PhpEngine::init() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to init PHP: {e}");
            std::process::exit(1);
        }
    };
    println!("PHP version: {}", engine.php_version());
    println!("Embed SAPI:  active");
    println!("Turbine:     v{}", env!("CARGO_PKG_VERSION"));
}

/// `turbine status` — query a running server's status endpoint.
pub fn cmd_status(address: &str) {
    let url = format!("http://{address}/_/status");
    match std::net::TcpStream::connect(address) {
        Ok(mut stream) => {
            use std::io::{BufRead, BufReader, Write};
            let req =
                format!("GET /_/status HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
            let _ = stream.write_all(req.as_bytes());
            let mut response = String::new();
            let mut reader = BufReader::new(&stream);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => response.push_str(&line),
                    Err(_) => break,
                }
            }
            if let Some(body_start) = response.find("\r\n\r\n") {
                print!("{}", &response[body_start + 4..]);
            } else {
                eprintln!("Invalid response from {url}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Cannot connect to {address}: {e}");
            eprintln!("Is the server running? Start with: turbine serve");
            std::process::exit(1);
        }
    }
}

/// `turbine cache:clear` — send cache clear command to running server.
pub fn cmd_cache_clear(address: &str) {
    match std::net::TcpStream::connect(address) {
        Ok(mut stream) => {
            use std::io::{BufRead, BufReader, Write};
            let req = format!(
                "POST /_/cache/clear HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(req.as_bytes());
            let mut response = String::new();
            let mut reader = BufReader::new(&stream);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => response.push_str(&line),
                    Err(_) => break,
                }
            }
            if let Some(body_start) = response.find("\r\n\r\n") {
                print!("{}", &response[body_start + 4..]);
            } else {
                eprintln!("Invalid response");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Cannot connect to {address}: {e}");
            std::process::exit(1);
        }
    }
}
