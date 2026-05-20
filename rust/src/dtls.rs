// DTLS / SRTP — pure-Rust cert + fingerprint.
//
// The DTLS-SRTP peer authentication contract is: the server advertises a
// certificate fingerprint in its SDP, the peer computes the fingerprint of
// the cert received in the DTLS handshake, and they must match.
//
// Previously the JS wrapper shelled out to `openssl genrsa` on first
// startup, wrote a PEM to disk, and Rust re-parsed it. That had two
// problems: (1) `webrtc-dtls` only accepts ECDSA P-256 or Ed25519 private
// keys, so RSA broke the handshake; (2) the openssl binary was an extra
// container dependency. Both go away by generating the cert in Rust once
// per process and keeping it in memory — the container's filesystem is
// ephemeral anyway.

use std::sync::OnceLock;

use sha2::{Digest, Sha256};
use webrtc_dtls::crypto::Certificate;

fn hex_colon(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 { s.push(':'); }
        s.push_str(&format!("{:02X}", b));
    }
    s
}

static CERT: OnceLock<Certificate> = OnceLock::new();
static FINGERPRINT: OnceLock<String> = OnceLock::new();

fn init_cert() -> Certificate {
    Certificate::generate_self_signed(vec!["projectrtp".into()])
        .expect("generate self-signed ECDSA P-256 cert")
}

fn compute_fingerprint(cert: &Certificate) -> String {
    let der = cert.certificate.first()
        .expect("cert has at least one DER entry")
        .as_ref();
    hex_colon(&Sha256::digest(der))
}

/// Process-lifetime certificate used for every DTLS handshake on this
/// server. Generated on first access. See module docs for rationale.
pub fn get_certificate() -> Certificate {
    CERT.get_or_init(init_cert).clone()
}

/// SHA-256 fingerprint of the process-lifetime certificate, formatted as
/// colon-separated uppercase hex (e.g. `A1:B2:…`). Exposed to JS as
/// `projectrtp.dtls.fingerprint` — babble-sip embeds this in SDP.
pub fn fingerprint() -> &'static str {
    FINGERPRINT.get_or_init(|| compute_fingerprint(CERT.get_or_init(init_cert)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_sha256_colon_hex_of_cert_der() {
        let fp = fingerprint();
        // Format: 32 bytes * 2 hex + 31 colons = 95 chars.
        assert_eq!(fp.len(), 95);
        assert!(fp.chars().all(|c| c == ':' || c.is_ascii_hexdigit()));
    }

    #[test]
    fn fingerprint_is_stable_within_process() {
        let a = fingerprint().to_string();
        let b = fingerprint().to_string();
        assert_eq!(a, b);
    }

    #[test]
    fn get_certificate_returns_clones_of_same_cert() {
        let c1 = get_certificate();
        let c2 = get_certificate();
        assert_eq!(c1.certificate, c2.certificate);
    }
}
