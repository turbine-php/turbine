use std::env;
use std::process::Command;

fn main() {
    // Try to find php-config to get compilation flags
    let php_config = env::var("PHP_CONFIG").unwrap_or_else(|_| "php-config".to_string());

    let includes = run_php_config(&php_config, "--includes");
    let ldflags = run_php_config(&php_config, "--ldflags");
    let libs = run_php_config(&php_config, "--libs");
    let prefix = run_php_config(&php_config, "--prefix");
    let extension_dir = run_php_config(&php_config, "--extension-dir");
    let version = run_php_config(&php_config, "--version");

    println!("cargo:rustc-env=PHP_VERSION={version}");
    println!("cargo:rustc-env=PHP_PREFIX={prefix}");
    println!("cargo:rustc-env=PHP_EXTENSION_DIR={extension_dir}");

    // Link against libphp (embed SAPI)
    let lib_dir = format!("{prefix}/lib");
    println!("cargo:rustc-link-search=native={lib_dir}");

    // Static linking: produces a single binary with no PHP runtime dependency.
    // Requires PHP built with --enable-static (produces libphp.a).
    // Enable via: TURBINE_STATIC_PHP=1 cargo build --release
    if env::var("TURBINE_STATIC_PHP").unwrap_or_default() == "1" {
        println!("cargo:rustc-link-lib=static=php");
        eprintln!("cargo:warning=Static linking PHP — single binary mode");
    } else {
        println!("cargo:rustc-link-lib=php");
        // Set rpath so the binary can find libphp at runtime (macOS + Linux)
        println!("cargo:rustc-link-arg=-Wl,-rpath,{lib_dir}");
    }

    // Parse and emit additional linker flags
    for flag in ldflags.split_whitespace() {
        if let Some(path) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={path}");
        }
    }

    for flag in libs.split_whitespace() {
        if let Some(lib) = flag.strip_prefix("-l") {
            // On macOS, skip libstdc++ (uses libc++ instead)
            if lib == "stdc++" {
                println!("cargo:rustc-link-lib=c++");
                continue;
            }
            println!("cargo:rustc-link-lib={lib}");
        }
    }

    // Pass include paths for any C shim compilation
    println!("cargo:includes={includes}");

    // Compile the worker lifecycle C shim (turbine_worker_request_startup/shutdown).
    // These provide lightweight per-request state management that preserves
    // the PHP global variable table ($kernel, $app) across requests.
    let include_flags: Vec<&str> = includes.split_whitespace().collect();
    let mut c_build = cc::Build::new();
    c_build.file("src/turbine_worker_lifecycle.c");
    c_build.file("src/turbine_sapi_handler.c");
    c_build.file("src/turbine_thread_support.c");
    for flag in &include_flags {
        c_build.flag(flag);
    }
    // Suppress PHP internal header warnings that are not our concern
    c_build.flag("-Wno-unused-function")
           .flag("-Wno-deprecated-declarations");
    c_build.compile("turbine_worker_lifecycle");
    println!("cargo:rerun-if-changed=src/turbine_worker_lifecycle.c");
    println!("cargo:rerun-if-changed=src/turbine_sapi_handler.c");
    println!("cargo:rerun-if-changed=src/turbine_thread_support.c");

    // Rebuild if PHP installation changes
    println!("cargo:rerun-if-env-changed=PHP_CONFIG");
    println!("cargo:rerun-if-changed=build.rs");
}

fn run_php_config(php_config: &str, arg: &str) -> String {
    let output = Command::new(php_config)
        .arg(arg)
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "Failed to run `{php_config} {arg}`. \
                 Is PHP installed with the embed SAPI? \
                 Install with: brew install php (macOS) or \
                 build PHP from source with --enable-embed. \
                 Error: {e}"
            );
        });

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("`{php_config} {arg}` failed: {stderr}");
    }

    String::from_utf8(output.stdout)
        .expect("php-config output is not valid UTF-8")
        .trim()
        .to_string()
}
