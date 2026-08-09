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
