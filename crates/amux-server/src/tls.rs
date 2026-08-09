//! Self-signed TLS (RR-0022).
//!
//! Certificate + key persist under `~/.amux/tls/` so browsers that accepted
//! the cert once keep working across restarts. Regenerated automatically
//! when missing or unreadable.

use std::path::Path;

pub struct TlsMaterial {
    pub cert_pem: String,
    pub key_pem: String,
}

pub fn load_or_generate(dir: &Path) -> anyhow::Result<TlsMaterial> {
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    if let (Ok(cert_pem), Ok(key_pem)) = (
        std::fs::read_to_string(&cert_path),
        std::fs::read_to_string(&key_path),
    ) {
        if !cert_pem.is_empty() && !key_pem.is_empty() {
            return Ok(TlsMaterial { cert_pem, key_pem });
        }
    }
    let mut params = rcgen::CertificateParams::new(vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
    ])?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "amux");
    let key_pair = rcgen::KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;
    let material = TlsMaterial {
        cert_pem: cert.pem(),
        key_pem: key_pair.serialize_pem(),
    };
    std::fs::create_dir_all(dir)?;
    std::fs::write(&cert_path, &material.cert_pem)?;
    std::fs::write(&key_path, &material.key_pem)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(material)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_then_reuses() {
        let dir = std::env::temp_dir().join(format!("amux-tls-test-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let m1 = load_or_generate(&dir).unwrap();
        assert!(m1.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(m1.key_pem.contains("PRIVATE KEY"));
        let m2 = load_or_generate(&dir).unwrap();
        assert_eq!(m1.cert_pem, m2.cert_pem, "must reuse persisted cert");
        std::fs::remove_dir_all(&dir).ok();
    }
}

// ---------------------------------------------------------------------------
// SNI dual-cert serving (Tailscale parity with the Python server)
// ---------------------------------------------------------------------------

use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use std::sync::Arc;

/// SNI resolver: the REAL Tailscale Let's Encrypt cert for the tailnet
/// hostname, the self-signed fallback for localhost/IPs — byte-for-byte the
/// Python server's `_sni_cb` behavior (amux-server.py:77931), so
/// https://desktop.tail5ce8f5.ts.net:8824 carries a browser-trusted cert
/// and the service worker can register.
#[derive(Debug)]
pub struct SniCerts {
    pub fallback: Arc<CertifiedKey>,
    pub ts_hostname: Option<String>,
    pub ts_cert: Option<Arc<CertifiedKey>>,
}

impl ResolvesServerCert for SniCerts {
    fn resolve(&self, hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        if let (Some(name), Some(ts), Some(cert)) =
            (hello.server_name(), &self.ts_hostname, &self.ts_cert)
        {
            if name.eq_ignore_ascii_case(ts) {
                return Some(cert.clone());
            }
        }
        Some(self.fallback.clone())
    }
}

fn load_certified_key(cert_pem: &str, key_pem: &str) -> anyhow::Result<CertifiedKey> {
    let certs: Vec<_> = rustls_pemfile::certs(&mut cert_pem.as_bytes())
        .collect::<Result<_, _>>()?;
    let key = rustls_pemfile::private_key(&mut key_pem.as_bytes())?
        .ok_or_else(|| anyhow::anyhow!("no private key in PEM"))?;
    let signing_key = rustls::crypto::ring::sign::any_supported_type(&key)
        .map_err(|e| anyhow::anyhow!("unsupported key type: {e}"))?;
    Ok(CertifiedKey::new(certs, signing_key))
}

/// Build the full rustls ServerConfig: self-signed fallback always; the
/// Tailscale cert layered in when `<host>.ts.net.crt/.key` exist in the TLS
/// dir (the same files `tailscale cert` writes and the Python server loads).
pub fn build_server_config(dir: &std::path::Path) -> anyhow::Result<rustls::ServerConfig> {
    let material = load_or_generate(dir)?;
    let fallback = Arc::new(load_certified_key(&material.cert_pem, &material.key_pem)?);

    let mut ts_hostname = None;
    let mut ts_cert = None;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(host) = name.strip_suffix(".crt") {
                if host.contains(".ts.net") {
                    let key_path = dir.join(format!("{host}.key"));
                    if let (Ok(c), Ok(k)) = (
                        std::fs::read_to_string(e.path()),
                        std::fs::read_to_string(&key_path),
                    ) {
                        match load_certified_key(&c, &k) {
                            Ok(ck) => {
                                tracing::info!(host, "tailscale cert loaded for SNI");
                                ts_hostname = Some(host.to_string());
                                ts_cert = Some(Arc::new(ck));
                            }
                            Err(err) => {
                                tracing::warn!(host, error = %err, "tailscale cert unusable — fallback only");
                            }
                        }
                    }
                }
            }
        }
    }

    let mut cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(SniCerts {
            fallback,
            ts_hostname,
            ts_cert,
        }));
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(cfg)
}
