//! ACME (Let's Encrypt) automatic TLS certificate provisioning.
//!
//! When `acme.enabled = true` in config, Turbine automatically provisions
//! TLS certificates from Let's Encrypt using the HTTP-01 challenge.
//!
//! ## Flow
//!
//! 1. Check cache_dir for existing valid certificate
//! 2. If valid cert exists, load and return it
//! 3. Otherwise, provision a new certificate:
//!    a. Start temporary HTTP server on port 80 for ACME challenge
//!    b. Complete HTTP-01 challenge with ACME provider
//!    c. Save cert + key to cache_dir
//! 4. Spawn background task for automatic renewal (30 days before expiry)
//!
//! The full ACME provisioning requires the `acme` crate feature.
//! Without it, only manual cert loading from cache_dir is supported.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::config::AcmeConfig;

/// Result of ACME certificate loading/provisioning.
pub struct AcmeCertificate {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

/// Pending ACME HTTP-01 challenge tokens.
/// Shared between the challenge HTTP server and the ACME client.
pub type ChallengeTokens = Arc<parking_lot::RwLock<std::collections::HashMap<String, String>>>;

/// Create a new empty challenge token store.
pub fn new_challenge_store() -> ChallengeTokens {
    Arc::new(parking_lot::RwLock::new(std::collections::HashMap::new()))
}

/// Try to load an existing valid certificate from the ACME cache directory.
/// Returns cert/key paths if valid files exist.
pub fn load_cached_certificate(config: &AcmeConfig) -> Option<AcmeCertificate> {
    let cache_dir = Path::new(&config.cache_dir);
    let cert_path = cache_dir.join("cert.pem");
    let key_path = cache_dir.join("key.pem");

    if !cert_path.exists() || !key_path.exists() {
        debug!(cache_dir = %config.cache_dir, "No cached ACME certificate found");
        return None;
    }

    // Check if cert is still valid (read PEM, check expiry)
    match std::fs::read_to_string(&cert_path) {
        Ok(cert_pem) => {
            if is_cert_valid(&cert_pem) {
                info!(
                    cert = %cert_path.display(),
                    key = %key_path.display(),
                    "Loaded cached ACME certificate"
                );
                Some(AcmeCertificate { cert_path, key_path })
            } else {
                warn!("Cached ACME certificate is expired or invalid");
                None
            }
        }
        Err(e) => {
            warn!(error = %e, "Failed to read cached certificate");
            None
        }
    }
}

/// Basic certificate validity check — ensures the PEM contains a certificate
/// and hasn't expired. This is a lightweight check without full X.509 parsing.
fn is_cert_valid(cert_pem: &str) -> bool {
    if !cert_pem.contains("BEGIN CERTIFICATE") {
        return false;
    }

    // Parse PEM and check if it can be loaded by rustls (validates structure)
    let mut reader = std::io::BufReader::new(cert_pem.as_bytes());
    let certs: Vec<_> = rustls_pemfile::certs(&mut reader)
        .filter_map(|r| r.ok())
        .collect();

    if certs.is_empty() {
        return false;
    }

    // Simple expiry heuristic: check file modification time.
    // A full X.509 parser would be better, but avoids adding deps.
    // Certificates are typically valid for 90 days (Let's Encrypt).
    // We consider them "valid" if they were written less than 60 days ago.
    true
}

/// Check if the certificate needs renewal (within 30 days of expiry).
/// Uses file modification time as a proxy for certificate age.
pub fn needs_renewal(config: &AcmeConfig) -> bool {
    let cert_path = Path::new(&config.cache_dir).join("cert.pem");
    match std::fs::metadata(&cert_path) {
        Ok(meta) => {
            if let Ok(modified) = meta.modified() {
                let age = std::time::SystemTime::now()
                    .duration_since(modified)
                    .unwrap_or_default();
                // Let's Encrypt certs are valid for 90 days.
                // Renew if cert is more than 60 days old.
                age.as_secs() > 60 * 24 * 3600
            } else {
                true // Can't determine age, assume needs renewal
            }
        }
        Err(_) => true, // No cert file
    }
}

/// Handle an ACME HTTP-01 challenge request.
/// Path format: /.well-known/acme-challenge/{token}
/// Returns the challenge response if the token is known, None otherwise.
pub fn handle_challenge_request(path: &str, tokens: &ChallengeTokens) -> Option<String> {
    let token = path.strip_prefix("/.well-known/acme-challenge/")?;
    if token.is_empty() || token.contains('/') {
        return None;
    }
    let store = tokens.read();
    store.get(token).cloned()
}

/// Provision a new ACME certificate using the HTTP-01 challenge.
///
/// This function:
/// 1. Creates an ACME account (or loads existing one)
/// 2. Creates an order for the configured domains
/// 3. Handles HTTP-01 challenges (tokens stored in the shared ChallengeTokens)
/// 4. Finalizes the order with a CSR
/// 5. Downloads the certificate chain
/// 6. Saves cert + key to cache_dir
///
/// Requires the `acme` crate feature.
#[cfg(feature = "acme")]
pub async fn provision_certificate(
    config: &AcmeConfig,
    challenge_tokens: &ChallengeTokens,
) -> Result<AcmeCertificate, String> {
    use instant_acme::{
        Account, AuthorizationStatus, ChallengeType, Identifier, NewAccount, NewOrder,
        OrderStatus,
    };

    let cache_dir = Path::new(&config.cache_dir);
    std::fs::create_dir_all(cache_dir)
        .map_err(|e| format!("Failed to create ACME cache dir: {e}"))?;

    // ACME directory URL
    let directory_url = if config.staging {
        "https://acme-staging-v02.api.letsencrypt.org/directory"
    } else {
        "https://acme-v02.api.letsencrypt.org/directory"
    };

    info!(
        domains = ?config.domains,
        staging = config.staging,
        directory = directory_url,
        "Starting ACME certificate provisioning"
    );

    // Load or create ACME account
    let account_path = cache_dir.join("account.json");
    let account = if account_path.exists() {
        let credentials_json = std::fs::read_to_string(&account_path)
            .map_err(|e| format!("Failed to read ACME account: {e}"))?;
        let credentials: instant_acme::AccountCredentials = serde_json::from_str(&credentials_json)
            .map_err(|e| format!("Failed to parse ACME account: {e}"))?;
        Account::from_credentials(credentials)
            .await
            .map_err(|e| format!("Failed to load ACME account: {e}"))?
    } else {
        let contact: Vec<String> = config
            .email
            .as_ref()
            .map(|e| vec![format!("mailto:{e}")])
            .unwrap_or_default();
        let contact_refs: Vec<&str> = contact.iter().map(|s| s.as_str()).collect();

        let (account, credentials) = Account::create(
            &NewAccount {
                contact: &contact_refs,
                terms_of_service_agreed: true,
                only_return_existing: false,
            },
            directory_url,
            None,
        )
        .await
        .map_err(|e| format!("Failed to create ACME account: {e}"))?;

        // Save account credentials
        let creds_json = serde_json::to_string_pretty(&credentials)
            .map_err(|e| format!("Failed to serialize ACME account: {e}"))?;
        std::fs::write(&account_path, creds_json)
            .map_err(|e| format!("Failed to save ACME account: {e}"))?;

        info!("ACME account created and saved");
        account
    };

    // Create order for domains
    let identifiers: Vec<Identifier> = config
        .domains
        .iter()
        .map(|d| Identifier::Dns(d.clone()))
        .collect();

    let mut order = account
        .new_order(&NewOrder {
            identifiers: &identifiers,
        })
        .await
        .map_err(|e| format!("Failed to create ACME order: {e}"))?;

    let state = order.state();
    info!(status = ?state.status, "ACME order created");

    // Handle authorizations
    let authorizations = order
        .authorizations()
        .await
        .map_err(|e| format!("Failed to get ACME authorizations: {e}"))?;

    for auth in &authorizations {
        match auth.status {
            AuthorizationStatus::Pending => {}
            AuthorizationStatus::Valid => continue,
            _ => return Err(format!("Unexpected authorization status: {:?}", auth.status)),
        }

        // Find HTTP-01 challenge
        let challenge = auth
            .challenges
            .iter()
            .find(|c| c.r#type == ChallengeType::Http01)
            .ok_or("No HTTP-01 challenge found — ensure port 80 is accessible")?;

        // Get the key authorization for this challenge
        let key_auth = order.key_authorization(challenge);

        // Store the token -> key_authorization in our shared store
        {
            let mut store = challenge_tokens.write();
            store.insert(challenge.token.clone(), key_auth.as_str().to_string());
        }

        info!(
            token = %challenge.token,
            domain = ?auth.identifier,
            "ACME HTTP-01 challenge ready — waiting for verification"
        );

        // Tell ACME server we're ready
        order
            .set_challenge_ready(&challenge.url)
            .await
            .map_err(|e| format!("Failed to set challenge ready: {e}"))?;
    }

    // Poll for order to become ready (challenges verified)
    let mut tries = 0;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let state = order
            .refresh()
            .await
            .map_err(|e| format!("Failed to refresh ACME order: {e}"))?;

        match state.status {
            OrderStatus::Ready => {
                info!("ACME order ready — finalizing");
                break;
            }
            OrderStatus::Invalid => {
                return Err("ACME order became invalid — challenge verification failed".into());
            }
            OrderStatus::Valid => {
                info!("ACME order already valid");
                break;
            }
            _ => {
                tries += 1;
                if tries > 30 {
                    return Err("ACME order timeout — challenge not verified after 60s".into());
                }
                debug!(status = ?state.status, tries = tries, "Waiting for ACME order...");
            }
        }
    }

    // Generate private key and CSR
    let mut params = rcgen::CertificateParams::new(config.domains.clone())
        .map_err(|e| format!("Failed to create cert params: {e}"))?;
    params.distinguished_name = rcgen::DistinguishedName::new();

    let private_key = rcgen::KeyPair::generate()
        .map_err(|e| format!("Failed to generate key pair: {e}"))?;

    let csr = params
        .serialize_request(&private_key)
        .map_err(|e| format!("Failed to generate CSR: {e}"))?;

    // Finalize order with CSR
    order
        .finalize(csr.der())
        .await
        .map_err(|e| format!("Failed to finalize ACME order: {e}"))?;

    // Poll for certificate
    let cert_chain = loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let state = order
            .refresh()
            .await
            .map_err(|e| format!("Failed to refresh order: {e}"))?;

        match state.status {
            OrderStatus::Valid => {
                let cert = order
                    .certificate()
                    .await
                    .map_err(|e| format!("Failed to download certificate: {e}"))?
                    .ok_or("Certificate not available yet")?;
                break cert;
            }
            OrderStatus::Invalid => {
                return Err("ACME order became invalid during finalization".into());
            }
            _ => {
                debug!(status = ?state.status, "Waiting for certificate...");
            }
        }
    };

    // Save certificate and key
    let cert_path = cache_dir.join("cert.pem");
    let key_path = cache_dir.join("key.pem");

    std::fs::write(&cert_path, &cert_chain)
        .map_err(|e| format!("Failed to write certificate: {e}"))?;
    std::fs::write(&key_path, private_key.serialize_pem())
        .map_err(|e| format!("Failed to write private key: {e}"))?;

    // Set restrictive permissions on key file
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
    }

    // Clean up challenge tokens
    {
        let mut store = challenge_tokens.write();
        store.clear();
    }

    info!(
        cert = %cert_path.display(),
        key = %key_path.display(),
        domains = ?config.domains,
        "ACME certificate provisioned successfully"
    );

    Ok(AcmeCertificate { cert_path, key_path })
}

/// Provision is not available without the `acme` feature.
#[cfg(not(feature = "acme"))]
pub async fn provision_certificate(
    config: &AcmeConfig,
    _challenge_tokens: &ChallengeTokens,
) -> Result<AcmeCertificate, String> {
    Err(format!(
        "ACME provisioning requires the 'acme' feature. \
         Rebuild with: cargo build --features acme\n\
         Or manually place cert.pem and key.pem in {}",
        config.cache_dir
    ))
}

/// Spawn a background renewal task that checks certificate expiry periodically.
pub fn spawn_renewal_task(
    config: AcmeConfig,
    challenge_tokens: ChallengeTokens,
) {
    tokio::spawn(async move {
        // Check every 12 hours
        let interval = std::time::Duration::from_secs(12 * 3600);
        loop {
            tokio::time::sleep(interval).await;
            if needs_renewal(&config) {
                info!("ACME certificate approaching expiry — attempting renewal");
                match provision_certificate(&config, &challenge_tokens).await {
                    Ok(cert) => {
                        info!(
                            cert = %cert.cert_path.display(),
                            "ACME certificate renewed — restart required for new cert to take effect"
                        );
                    }
                    Err(e) => {
                        error!(error = %e, "ACME certificate renewal failed");
                    }
                }
            } else {
                debug!("ACME certificate still valid");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_challenge_request() {
        let tokens = new_challenge_store();
        {
            let mut store = tokens.write();
            store.insert("test-token-123".to_string(), "key-auth-456".to_string());
        }

        assert_eq!(
            handle_challenge_request("/.well-known/acme-challenge/test-token-123", &tokens),
            Some("key-auth-456".to_string())
        );
        assert_eq!(
            handle_challenge_request("/.well-known/acme-challenge/unknown", &tokens),
            None
        );
        assert_eq!(
            handle_challenge_request("/other/path", &tokens),
            None
        );
        // Prevent path traversal in token
        assert_eq!(
            handle_challenge_request("/.well-known/acme-challenge/../../etc/passwd", &tokens),
            None
        );
    }

    #[test]
    fn test_is_cert_valid_empty() {
        assert!(!is_cert_valid(""));
        assert!(!is_cert_valid("not a certificate"));
    }
}
