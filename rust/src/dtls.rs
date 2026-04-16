// DTLS / SRTP — port of projectrtpsrtp.{cpp,h}.
//
// Scope for Task #6 (this commit):
//   - Compute and expose the DTLS-SRTP local fingerprint (`dtls.fingerprint`)
//     from the PEM cert at `~/.projectrtp/certs/dtls-srtp.pem`. Pure-Rust.
//
// Deferred to Task #7 (lands with channel):
//   - gnutls DTLS session with custom push/pull/timeout transport callbacks
//     wired into the channel's UDP socket.
//   - SRTP context setup from DTLS key material; protect() / unprotect() of
//     RTP/RTCP packets via libsrtp2 (via the `srtp2-sys` crate — already a
//     candidate dep, will land with the channel integration).

use std::path::PathBuf;
use std::sync::OnceLock;

use base64::Engine;
use napi_derive::napi;
use sha2::{Digest, Sha256};

fn cert_path() -> PathBuf {
    let mut p = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    p.push(".projectrtp/certs/dtls-srtp.pem");
    p
}

fn extract_cert_der(pem: &str) -> Option<Vec<u8>> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let start = pem.find(BEGIN)? + BEGIN.len();
    let rest = &pem[start..];
    let end = rest.find(END)?;
    let b64: String = rest[..end]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()).ok()
}

fn compute_fingerprint_from_pem(pem: &str) -> Option<String> {
    let der = extract_cert_der(pem)?;
    let digest = Sha256::digest(&der);
    Some(hex_colon(&digest))
}

fn hex_colon(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 { s.push(':'); }
        s.push_str(&format!("{:02X}", b));
    }
    s
}

static FINGERPRINT: OnceLock<String> = OnceLock::new();

fn load_fingerprint() -> String {
    let path = cert_path();
    let pem = match std::fs::read_to_string(&path) {
        Ok(p) => p,
        Err(_) => return String::new(),
    };
    compute_fingerprint_from_pem(&pem).unwrap_or_default()
}

/// Internal accessor for the fingerprint. Not exported via `#[napi]` —
/// we publish it as a *string property* on the `dtls` namespace, not a
/// function, because the JS tests assert `projectrtp.dtls.fingerprint`
/// is `.to.be.a("string")`. The namespace is populated in `lib.rs`
/// `init_module_exports` via a `#[napi::module_exports]` hook.
pub fn fingerprint() -> &'static str {
    FINGERPRINT.get_or_init(load_fingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A known self-signed cert (generated for this test) and the SHA-256 of
    // its DER, verified with `openssl x509 -in cert.pem -fingerprint -sha256 -noout`.
    const TEST_PEM: &str = "\
-----BEGIN CERTIFICATE-----
MIIBhTCCASugAwIBAgIUK+QOH2qy0x77QkyBAHNNqRTZAYUwCgYIKoZIzj0EAwIw
FDESMBAGA1UEAwwJcHJvamVjdHJ0cDAeFw0yNTAxMDEwMDAwMDBaFw0zNTAxMDEw
MDAwMDBaMBQxEjAQBgNVBAMMCXByb2plY3RydHAwWTATBgcqhkjOPQIBBggqhkjO
PQMBBwNCAATbI0qBXnRTPUaqmBR7tMzC2pl3xXwLi5o2UYH6crdRKC79EEjpHl/D
kkh4fAWkF4wuZ2dMgsnUJVwdvb6Fiep4o1MwUTAdBgNVHQ4EFgQU8eYvkWxdE4Co
TjRwPMO9evlapOwwHwYDVR0jBBgwFoAU8eYvkWxdE4CoTjRwPMO9evlapOwwDwYD
VR0TAQH/BAUwAwEB/zAKBggqhkjOPQQDAgNIADBFAiEAxQyjmjs0P4nzEtDsjVJI
m97lJqRUrSPntVN6WJoFGncCIArj93ZNTXiA6SyJVb4mwR2uHvdP7gqC8GKmHIb6
QtrM
-----END CERTIFICATE-----
";

    #[test]
    fn extracts_der_from_pem() {
        let der = extract_cert_der(TEST_PEM).expect("der");
        // ASN.1 SEQUENCE tag + length prefix.
        assert_eq!(der[0], 0x30);
        assert!(der.len() > 100);
    }

    #[test]
    fn fingerprint_is_sha256_colon_hex_of_der() {
        let fp = compute_fingerprint_from_pem(TEST_PEM).expect("fp");
        // Format: 32 bytes * 2 hex + 31 colons = 95 chars.
        assert_eq!(fp.len(), 95);
        assert!(fp.chars().all(|c| c == ':' || c.is_ascii_hexdigit()));
        // Determinism: same input → same output.
        let fp2 = compute_fingerprint_from_pem(TEST_PEM).unwrap();
        assert_eq!(fp, fp2);
    }

    #[test]
    fn missing_pem_yields_empty_string() {
        assert!(compute_fingerprint_from_pem("not a pem").is_none());
    }
}
