//! TLS acceptor construction and low-level socket tuning.
//!
//! * [`build_tls_acceptor`] / [`build_tls_acceptor_with_sni`] — build a
//!   `tokio_rustls::TlsAcceptor` with ALPN set to `h2, http/1.1` and
//!   optional per-vhost SNI certificates.
//! * [`set_busy_poll`] — Linux-only `SO_BUSY_POLL` tuning (no-op elsewhere).
//! * [`bind_reuseport_linux`] — Linux-only `SO_REUSEPORT` listener bind
//!   used by the accept-per-core pattern.

use std::sync::Arc;

#[cfg(target_os = "linux")]
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::{error, info};

pub fn build_tls_acceptor(cert_path: &str, key_path: &str) -> TlsAcceptor {
    build_tls_acceptor_with_sni(cert_path, key_path, &[])
}

/// Build a TLS acceptor with optional SNI-based per-host certificates.
/// The default cert/key is used for connections that don't match any SNI name.
pub fn build_tls_acceptor_with_sni(
    cert_path: &str,
    key_path: &str,
    vhost_certs: &[(String, String, String)], // (domain, cert_path, key_path)
) -> TlsAcceptor {
    use rustls::ServerConfig as RustlsConfig;
    use std::io::BufReader;

    // Helper: load cert+key pair
    fn load_cert_key(
        cert_path: &str,
        key_path: &str,
    ) -> (
        Vec<rustls::pki_types::CertificateDer<'static>>,
        rustls::pki_types::PrivateKeyDer<'static>,
    ) {
        let cert_file = std::fs::File::open(cert_path).unwrap_or_else(|e| {
            error!(path = cert_path, "Failed to open certificate file: {e}");
            std::process::exit(1);
        });
        let key_file = std::fs::File::open(key_path).unwrap_or_else(|e| {
            error!(path = key_path, "Failed to open key file: {e}");
            std::process::exit(1);
        });

        let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(cert_file))
            .filter_map(|r| r.ok())
            .collect();
        if certs.is_empty() {
            error!(path = cert_path, "No certificates found in file");
            std::process::exit(1);
        }

        let key = rustls_pemfile::private_key(&mut BufReader::new(key_file))
            .unwrap_or_else(|e| {
                error!(path = key_path, "Failed to parse private key: {e}");
                std::process::exit(1);
            })
            .unwrap_or_else(|| {
                error!(path = key_path, "No private key found in file");
                std::process::exit(1);
            });

        (certs, key)
    }

    let tls_config = if vhost_certs.is_empty() {
        // Simple path: single certificate
        let (certs, key) = load_cert_key(cert_path, key_path);
        let mut cfg = RustlsConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap_or_else(|e| {
                error!("Invalid TLS certificate/key pair: {e}");
                std::process::exit(1);
            });
        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        info!("TLS configured (single cert, ALPN: h2, http/1.1)");
        cfg
    } else {
        // SNI path: multiple certificates per domain
        use rustls::server::ResolvesServerCertUsingSni;
        use rustls::sign::CertifiedKey;

        let mut resolver = ResolvesServerCertUsingSni::new();

        for (domain, vcert_path, vkey_path) in vhost_certs {
            let (certs, key) = load_cert_key(vcert_path, vkey_path);
            let signing_key = rustls::crypto::aws_lc_rs::sign::any_supported_type(&key)
                .unwrap_or_else(|e| {
                    error!(domain = %domain, "Failed to load signing key: {e}");
                    std::process::exit(1);
                });
            let certified = CertifiedKey::new(certs, signing_key);
            resolver.add(domain, certified).unwrap_or_else(|e| {
                error!(domain = %domain, "Failed to add SNI cert: {e}");
                std::process::exit(1);
            });
            info!(domain = %domain, "SNI certificate loaded");
        }

        // Also add the default cert to the SNI resolver as fallback
        let (default_certs, default_key) = load_cert_key(cert_path, key_path);
        let default_signing = rustls::crypto::aws_lc_rs::sign::any_supported_type(&default_key)
            .unwrap_or_else(|e| {
                error!("Failed to load default signing key: {e}");
                std::process::exit(1);
            });
        let default_certified = Arc::new(CertifiedKey::new(default_certs, default_signing));

        // Build config with SNI resolver + default fallback
        use rustls::server::ResolvesServerCert;
        use std::fmt;
        struct SniWithFallback {
            sni: ResolvesServerCertUsingSni,
            fallback: Arc<CertifiedKey>,
        }
        impl fmt::Debug for SniWithFallback {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct("SniWithFallback").finish()
            }
        }
        impl ResolvesServerCert for SniWithFallback {
            fn resolve(
                &self,
                client_hello: rustls::server::ClientHello<'_>,
            ) -> Option<Arc<CertifiedKey>> {
                // Try SNI lookup first — if the domain has a cert, use it.
                // ResolvesServerCertUsingSni already checks server_name() internally.
                if client_hello.server_name().is_some() {
                    // Clone the server name to avoid borrow conflict
                    let sni_result = self.sni.resolve(client_hello);
                    if sni_result.is_some() {
                        return sni_result;
                    }
                    // SNI name didn't match — fall through to default
                }
                Some(self.fallback.clone())
            }
        }

        let sni_fallback = Arc::new(SniWithFallback {
            sni: resolver,
            fallback: default_certified,
        });

        let mut cfg = RustlsConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(sni_fallback);
        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        info!(
            vhosts = vhost_certs.len(),
            "TLS configured with SNI (ALPN: h2, http/1.1)"
        );
        cfg
    };

    TlsAcceptor::from(Arc::new(tls_config))
}

/// Enable `SO_BUSY_POLL` on a socket fd.
///
/// Linux only. The kernel will spin on the NIC RX queue for up to
/// `us` microseconds before yielding to the scheduler, shaving 20-50µs
/// off p99 latency at the cost of CPU usage.
///
/// Silently no-op on non-Linux and when the setsockopt fails (e.g.
/// insufficient privileges on kernels < 5.7). Returns `true` if the
/// option was applied.
#[cfg(target_os = "linux")]
pub fn set_busy_poll(fd: std::os::unix::io::RawFd, us: u32) -> bool {
    // SO_BUSY_POLL = 46 on Linux (kernel 3.11+).
    const SO_BUSY_POLL: libc::c_int = 46;
    let val: libc::c_int = us as libc::c_int;
    // SAFETY: fd is a valid socket fd owned by the caller; we only
    // borrow it for the duration of the setsockopt call.
    let rc = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            SO_BUSY_POLL,
            &val as *const _ as *const libc::c_void,
            std::mem::size_of_val(&val) as libc::socklen_t,
        )
    };
    rc == 0
}

#[cfg(not(target_os = "linux"))]
#[inline]
#[allow(dead_code)]
pub fn set_busy_poll(_fd: i32, _us: u32) -> bool {
    false
}

/// Bind a single TCP listener with `SO_REUSEPORT` set, so multiple
/// listeners can share the same (addr, port). Linux distributes
/// incoming connections across all such listeners with a flow hash,
/// which is the basis of the accept-per-core pattern.
///
/// Also sets `SO_REUSEADDR`, `SOCK_NONBLOCK`, `SOCK_CLOEXEC`, and a
/// generous listen backlog (1024).
///
/// Returns a `tokio::net::TcpListener` ready for async accept.
#[cfg(target_os = "linux")]
pub fn bind_reuseport_linux(addr: std::net::SocketAddr) -> std::io::Result<TcpListener> {
    use std::os::unix::io::FromRawFd;

    let domain = if addr.is_ipv6() {
        libc::AF_INET6
    } else {
        libc::AF_INET
    };
    // SAFETY: libc::socket returns -1 on error; we check below. All
    // other raw-fd ops are standard BSD sockets API usage.
    unsafe {
        let fd = libc::socket(
            domain,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        );
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // Wrap fd so it gets closed on any early return.
        let owned: std::os::unix::io::OwnedFd = std::os::unix::io::FromRawFd::from_raw_fd(fd);

        let one: libc::c_int = 1;
        let set = |opt: libc::c_int| -> std::io::Result<()> {
            let rc = libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                opt,
                &one as *const _ as *const libc::c_void,
                std::mem::size_of_val(&one) as libc::socklen_t,
            );
            if rc != 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        };
        set(libc::SO_REUSEADDR)?;
        set(libc::SO_REUSEPORT)?;

        // Bind
        match addr {
            std::net::SocketAddr::V4(v4) => {
                let sin = libc::sockaddr_in {
                    sin_family: libc::AF_INET as libc::sa_family_t,
                    sin_port: v4.port().to_be(),
                    sin_addr: libc::in_addr {
                        s_addr: u32::from_ne_bytes(v4.ip().octets()),
                    },
                    sin_zero: [0; 8],
                };
                let rc = libc::bind(
                    fd,
                    &sin as *const _ as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
                );
                if rc != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            std::net::SocketAddr::V6(v6) => {
                let mut sin6: libc::sockaddr_in6 = std::mem::zeroed();
                sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
                sin6.sin6_port = v6.port().to_be();
                sin6.sin6_addr.s6_addr = v6.ip().octets();
                sin6.sin6_flowinfo = v6.flowinfo();
                sin6.sin6_scope_id = v6.scope_id();
                let rc = libc::bind(
                    fd,
                    &sin6 as *const _ as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
                );
                if rc != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
        }

        let rc = libc::listen(fd, 1024);
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }

        // Convert to std listener (taking ownership), then to tokio.
        let raw = std::os::unix::io::IntoRawFd::into_raw_fd(owned);
        let std_listener = std::net::TcpListener::from_raw_fd(raw);
        TcpListener::from_std(std_listener)
    }
}
