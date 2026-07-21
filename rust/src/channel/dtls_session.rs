// DTLS session — pure-Rust via webrtc-dtls.
//
// The channel's UDP socket carries RTP, DTLS and STUN multiplexed on one
// port. The tick/mixer classify demuxes by first byte: STUN (0-3), DTLS
// (20-63), RTP (128-191). DTLS datagrams are fed into a DtlsTransport
// (mpsc-backed Conn adapter) that the webrtc-dtls DTLSConn reads from.
// Outbound DTLS frames come back via a second mpsc and are sent on the
// real socket by the tick.
//
// After the handshake completes, keying material is exported and split
// (`split_keying_material` / `local_srtp_params` / `remote_srtp_params`) to
// build the SRTP/SRTCP encrypt/decrypt contexts — RTP contexts in the tick
// (`poll_dtls_handshake`), the inbound SRTCP context in `rtcp_loop`.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex as PLMutex;
use tokio::sync::mpsc;
use webrtc_dtls::config::Config as DtlsConfig;
use webrtc_dtls::config::{ClientAuthType, ExtendedMasterSecretType};
use webrtc_dtls::conn::DTLSConn;
use webrtc_dtls::crypto::Certificate;
use webrtc_srtp::protection_profile::ProtectionProfile;

use crate::channel::commands::DtlsSetup;

/// The peer's certificate fingerprint as promised out-of-band in SDP
/// (`a=fingerprint`). DTLS-SRTP has no CA: the peer's self-signed cert is
/// authenticated by matching the fingerprint of the cert seen in the handshake
/// against this value (RFC 5763 §5).
#[derive(Debug, Clone)]
pub struct PeerFingerprint {
    /// Hash algorithm, lowercased (e.g. `sha-256`). Only `sha-256` is
    /// supported for matching — any other value fails closed.
    pub algorithm: String,
    /// Colon-separated uppercase hex of the digest (e.g. `A1:B2:…`).
    pub hex_colon: String,
}

impl PeerFingerprint {
    /// Parse an SDP fingerprint value. Accepts either the full `a=fingerprint`
    /// form `"<algorithm> <hex>"` or a bare colon-hex string (algorithm assumed
    /// `sha-256`, matching what `crate::dtls::fingerprint` advertises and what
    /// the JS layer currently forwards). Returns `None` for empty/whitespace,
    /// which the caller treats as "no fingerprint supplied → skip verification".
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        match s.split_once(char::is_whitespace) {
            Some((algo, hex)) => Some(Self {
                algorithm: algo.trim().to_ascii_lowercase(),
                hex_colon: hex.trim().to_ascii_uppercase(),
            }),
            None => Some(Self {
                algorithm: "sha-256".to_string(),
                hex_colon: s.to_ascii_uppercase(),
            }),
        }
    }

    /// True iff `der` (a peer certificate in DER form) hashes to this
    /// fingerprint. Only sha-256 is honoured; any other algorithm returns
    /// false so an unknown/downgraded hash fails the handshake rather than
    /// silently passing.
    pub fn matches_der(&self, der: &[u8]) -> bool {
        if self.algorithm != "sha-256" {
            return false;
        }
        crate::dtls::sha256_fingerprint(der).eq_ignore_ascii_case(&self.hex_colon)
    }
}

/// Keying material extracted from a completed DTLS handshake.
#[derive(Debug, Clone)]
pub struct SrtpKeyingMaterial {
    pub profile: ProtectionProfile,
    pub client_write_key: Vec<u8>,
    pub client_write_salt: Vec<u8>,
    pub server_write_key: Vec<u8>,
    pub server_write_salt: Vec<u8>,
    pub local_is_server: bool,
}

/// Channel-fed Conn adapter. The recv_loop reads the socket continuously
/// and feeds DTLS packets via an mpsc. Outbound DTLS frames are sent
/// directly on the socket. No tick latency — the recv_loop fires
/// immediately on packet arrival.
///
/// `remote_addr` is **shared** with the channel's `state.remote_addr` (the
/// same arc the recv_loop populates on every inbound packet). Without this
/// sharing, the server role never learned the client's address and
/// fell back to `local_addr` — so DTLS replies were sent to the server's
/// own bind and Chromium saw no response to its ClientHello.
pub struct DtlsTransport {
    inbound_rx: tokio::sync::Mutex<mpsc::Receiver<Vec<u8>>>,
    sock: Arc<tokio::net::UdpSocket>,
    local_addr: SocketAddr,
    remote_addr: Arc<PLMutex<Option<SocketAddr>>>,
}

#[async_trait]
impl webrtc_util::Conn for DtlsTransport {
    async fn connect(&self, addr: SocketAddr) -> webrtc_util::Result<()> {
        *self.remote_addr.lock() = Some(addr);
        Ok(())
    }

    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        let mut guard = self.inbound_rx.lock().await;
        match guard.recv().await {
            Some(data) => {
                let n = data.len().min(buf.len());
                buf[..n].copy_from_slice(&data[..n]);
                Ok(n)
            }
            None => {
                // All senders dropped — channel is tearing down. Without a
                // delay here, webrtc-dtls's handshake driver re-polls recv
                // immediately on every Err and pegs a CPU until the spawned
                // task is finally aborted. The 50 ms sleep caps that worst
                // case at 20 Hz.
                drop(guard);
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                Err(webrtc_util::Error::Other("dtls channel closed".into()))
            }
        }
    }

    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        let n = self.recv(buf).await?;
        let addr = (*self.remote_addr.lock()).unwrap_or(self.local_addr);
        Ok((n, addr))
    }

    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        let remote = (*self.remote_addr.lock()).unwrap_or(self.local_addr);
        self.sock
            .send_to(buf, remote)
            .await
            .map_err(|e| webrtc_util::Error::Other(e.to_string()))
    }

    async fn send_to(&self, buf: &[u8], target: SocketAddr) -> webrtc_util::Result<usize> {
        self.sock
            .send_to(buf, target)
            .await
            .map_err(|e| webrtc_util::Error::Other(e.to_string()))
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        Ok(self.local_addr)
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        *self.remote_addr.lock()
    }

    async fn close(&self) -> webrtc_util::Result<()> {
        Ok(())
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

/// Result of a completed DTLS handshake, sent back to the channel actor.
pub struct HandshakeResult {
    pub keying_material: Vec<u8>,
    pub profile: ProtectionProfile,
    pub is_client: bool,
}

/// Handle returned by `spawn_handshake`. The actor stores `abort` on
/// `ChannelState` and calls `abort.abort()` on close so the spawned
/// handshake task can't outlive the channel and busy-spin on a dead
/// transport — the original cause of a 99% CPU per orphan handshake
/// reported in production.
pub struct HandshakeHandle {
    pub result_rx: tokio::sync::oneshot::Receiver<Option<HandshakeResult>>,
    pub abort: tokio::task::AbortHandle,
}

/// Hard cap on the handshake. WebRTC peers complete in well under a
/// second when the network is healthy; 10 s allows for retransmits but
/// guarantees the spawned task exits even if the peer disappears
/// mid-handshake. Mirrors the C++ side's idle teardown of stalled remotes.
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Spawn the DTLS handshake as a background task. The transport reads the
/// UDP socket directly for fast handshake round-trips. Non-DTLS packets
/// (RTP, STUN) are forwarded via the returned `forwarded_rx` channel so
/// the tick still processes them during the handshake phase.
///
/// After the handshake completes, the task exits and the tick resumes
/// direct socket reads (by clearing `forwarded_rx`).
pub fn spawn_handshake(
    setup: DtlsSetup,
    local_addr: SocketAddr,
    sock: Arc<tokio::net::UdpSocket>,
    inbound_rx: mpsc::Receiver<Vec<u8>>,
    certificate: Certificate,
    remote_addr: Arc<PLMutex<Option<SocketAddr>>>,
    expected_fingerprint: Option<PeerFingerprint>,
) -> HandshakeHandle {
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();

    let transport = Arc::new(DtlsTransport {
        inbound_rx: tokio::sync::Mutex::new(inbound_rx),
        sock,
        local_addr,
        remote_addr,
    });

    let is_client = setup == DtlsSetup::Active;
    let srtp_profiles = vec![
        webrtc_dtls::extension::extension_use_srtp::SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80,
    ];

    let join = tokio::spawn(async move {
        let mut config = DtlsConfig {
            certificates: vec![certificate],
            srtp_protection_profiles: srtp_profiles,
            // DTLS-SRTP uses self-signed certs with no CA, so skip the built-in
            // chain/name verification; the peer is instead authenticated by its
            // SDP fingerprint via `verify_peer_certificate` below.
            insecure_skip_verify: true,
            // Make the *server* (passive) role send a CertificateRequest and
            // require the peer to present a cert — otherwise it never receives
            // one to fingerprint. Ignored for the client role. We do the actual
            // authentication in `verify_peer_certificate`, not via a CA, so no
            // client_cert_verifier is needed (RequireAnyClientCert < the
            // VerifyClientCertIfGiven threshold that would demand one).
            client_auth: ClientAuthType::RequireAnyClientCert,
            extended_master_secret: ExtendedMasterSecretType::Require,
            ..Default::default()
        };

        // When SDP supplied a fingerprint, enforce it: the handshake fails
        // (BadCertificate alert) unless the peer's leaf cert hashes to it. With
        // no fingerprint we fall back to the prior unauthenticated behaviour.
        if let Some(fp) = expected_fingerprint {
            config.verify_peer_certificate =
                Some(Arc::new(move |certs, _chains| match certs.first() {
                    Some(der) if fp.matches_der(der) => Ok(()),
                    _ => Err(webrtc_dtls::Error::Other(
                        "dtls peer certificate fingerprint mismatch".to_owned(),
                    )),
                }));
        }

        let outcome = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            DTLSConn::new(transport, config, is_client, None),
        )
        .await;

        let result = match outcome {
            Ok(Ok(conn)) => {
                use webrtc_util::KeyingMaterialExporter;
                let state = conn.connection_state().await;
                let label = "EXTRACTOR-dtls_srtp";
                let profile = ProtectionProfile::Aes128CmHmacSha1_80;
                let km_len = 2 * (profile.key_len() + profile.salt_len());
                match state.export_keying_material(label, &[], km_len).await {
                    Ok(km) => Some(HandshakeResult {
                        keying_material: km,
                        profile,
                        is_client,
                    }),
                    Err(_) => None,
                }
            }
            // Ok(Err(_)) — DTLSConn::new returned an error.
            // Err(_)     — outer timeout fired; peer never finished.
            _ => None,
        };
        let _ = result_tx.send(result);
    });

    HandshakeHandle {
        result_rx,
        abort: join.abort_handle(),
    }
}

/// Split exported keying material into client/server key + salt pairs.
/// Layout per RFC 5764 §4.2:
///   client_write_key || server_write_key || client_write_salt || server_write_salt
pub fn split_keying_material(
    km: &[u8],
    profile: ProtectionProfile,
    is_client: bool,
) -> SrtpKeyingMaterial {
    let key_len = profile.key_len();
    let salt_len = profile.salt_len();
    let mut off = 0;
    let client_write_key = km[off..off + key_len].to_vec();
    off += key_len;
    let server_write_key = km[off..off + key_len].to_vec();
    off += key_len;
    let client_write_salt = km[off..off + salt_len].to_vec();
    off += salt_len;
    let server_write_salt = km[off..off + salt_len].to_vec();
    let _ = off;
    SrtpKeyingMaterial {
        profile,
        client_write_key,
        client_write_salt,
        server_write_key,
        server_write_salt,
        local_is_server: !is_client,
    }
}

/// Key + salt + profile for the **local** (outbound) SRTP/SRTCP direction.
/// The local side writes with the server key when we are the DTLS server,
/// otherwise the client key.
pub fn local_srtp_params(km: &SrtpKeyingMaterial) -> (&[u8], &[u8], ProtectionProfile) {
    if km.local_is_server {
        (&km.server_write_key, &km.server_write_salt, km.profile)
    } else {
        (&km.client_write_key, &km.client_write_salt, km.profile)
    }
}

/// Key + salt + profile for the **remote** (inbound) SRTP/SRTCP direction —
/// the peer writes with the opposite key to us. Used to build the SRTCP
/// decrypt context in `rtcp_loop`, mirroring the RTP decrypt context the tick
/// builds in `poll_dtls_handshake`.
pub fn remote_srtp_params(km: &SrtpKeyingMaterial) -> (&[u8], &[u8], ProtectionProfile) {
    if km.local_is_server {
        (&km.client_write_key, &km.client_write_salt, km.profile)
    } else {
        (&km.server_write_key, &km.server_write_salt, km.profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::net::UdpSocket;

    /// The sha-256 `PeerFingerprint` of a certificate's leaf DER — what the
    /// peer would advertise in SDP.
    fn fp_of(cert: &Certificate) -> PeerFingerprint {
        let der = cert.certificate.first().unwrap().as_ref();
        PeerFingerprint {
            algorithm: "sha-256".to_string(),
            hex_colon: crate::dtls::sha256_fingerprint(der),
        }
    }

    /// Drive a full DTLS handshake between an active (client) and passive
    /// (server) peer over loopback, each optionally verifying the other's
    /// certificate fingerprint. Returns the two handshake outcomes (`None` =
    /// the handshake failed — e.g. a fingerprint mismatch).
    ///
    /// Multi-thread flavor is required by callers: DTLSConn spawns internal
    /// tasks and the relay below is a `tokio::select!` loop, so a
    /// single-threaded runtime would deadlock.
    async fn handshake_pair(
        server_cert: Certificate,
        client_cert: Certificate,
        server_expects: Option<PeerFingerprint>,
        client_expects: Option<PeerFingerprint>,
    ) -> (Option<HandshakeResult>, Option<HandshakeResult>) {
        let server_sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let client_sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let server_addr = server_sock.local_addr().unwrap();
        let client_addr = client_sock.local_addr().unwrap();

        // Connect sockets so DtlsTransport.send() reaches the peer.
        server_sock.connect(client_addr).await.unwrap();
        client_sock.connect(server_addr).await.unwrap();

        // Each side gets an mpsc for DTLS inbound — a relay task reads
        // each socket and feeds the peer's mpsc (simulating recv_loop).
        let (server_dtls_tx, server_dtls_rx) = mpsc::channel::<Vec<u8>>(64);
        let (client_dtls_tx, client_dtls_rx) = mpsc::channel::<Vec<u8>>(64);

        // Relay simulates recv_loop: it picks data off each side's socket
        // (which the OS filled with the peer's outbound) and forwards it
        // onto that side's own DTLS inbound queue. I.e. packets arriving
        // on server_sock are from the client, so they belong on the
        // server's mpsc (server_dtls_tx), not the client's.
        let srv_sock2 = server_sock.clone();
        let cli_sock2 = client_sock.clone();
        let relay = tokio::spawn(async move {
            let mut sbuf = [0u8; 2048];
            let mut cbuf = [0u8; 2048];
            loop {
                tokio::select! {
                    Ok((n, _)) = srv_sock2.recv_from(&mut sbuf) => {
                        if server_dtls_tx.send(sbuf[..n].to_vec()).await.is_err() { break; }
                    }
                    Ok((n, _)) = cli_sock2.recv_from(&mut cbuf) => {
                        if client_dtls_tx.send(cbuf[..n].to_vec()).await.is_err() { break; }
                    }
                }
            }
        });

        let server_remote = Arc::new(PLMutex::new(Some(client_addr)));
        let client_remote = Arc::new(PLMutex::new(Some(server_addr)));
        let server_h = spawn_handshake(
            DtlsSetup::Passive,
            server_addr,
            server_sock,
            server_dtls_rx,
            server_cert,
            server_remote,
            server_expects,
        );
        let client_h = spawn_handshake(
            DtlsSetup::Active,
            client_addr,
            client_sock,
            client_dtls_rx,
            client_cert,
            client_remote,
            client_expects,
        );

        // A failed handshake still resolves the oneshot (with `None`); only a
        // dropped sender or true timeout should panic here.
        let server_result = tokio::time::timeout(Duration::from_secs(5), server_h.result_rx)
            .await
            .expect("server handshake timeout")
            .expect("server oneshot dropped");
        let client_result = tokio::time::timeout(Duration::from_secs(5), client_h.result_rx)
            .await
            .expect("client handshake timeout")
            .expect("client oneshot dropped");

        relay.abort();
        (server_result, client_result)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dtls_handshake_between_two_peers() {
        let server_cert = Certificate::generate_self_signed(vec!["server".into()]).unwrap();
        let client_cert = Certificate::generate_self_signed(vec!["client".into()]).unwrap();

        let (server_result, client_result) =
            handshake_pair(server_cert, client_cert, None, None).await;

        assert!(server_result.is_some(), "server handshake failed");
        assert!(client_result.is_some(), "client handshake failed");

        let server_km = server_result.unwrap();
        let client_km = client_result.unwrap();

        assert_eq!(server_km.keying_material, client_km.keying_material);
        assert!(!server_km.is_client);
        assert!(client_km.is_client);

        let server_keys = split_keying_material(
            &server_km.keying_material,
            server_km.profile,
            server_km.is_client,
        );
        let client_keys = split_keying_material(
            &client_km.keying_material,
            client_km.profile,
            client_km.is_client,
        );
        assert_eq!(server_keys.client_write_key, client_keys.client_write_key);
        assert_eq!(server_keys.server_write_key, client_keys.server_write_key);

        // Verify SRTP encrypt/decrypt round-trip.
        let mut encrypt_ctx = webrtc_srtp::context::Context::new(
            &client_keys.client_write_key,
            &client_keys.client_write_salt,
            client_keys.profile,
            None,
            None,
        )
        .expect("srtp encrypt ctx");

        let mut decrypt_ctx = webrtc_srtp::context::Context::new(
            &server_keys.client_write_key,
            &server_keys.client_write_salt,
            server_keys.profile,
            None,
            None,
        )
        .expect("srtp decrypt ctx");

        let mut rtp_pkt = vec![
            0x80, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0xA0, 0x00, 0x00, 0x00, 0x01,
        ];
        rtp_pkt.extend_from_slice(&[0x55u8; 160]);

        let encrypted = encrypt_ctx.encrypt_rtp(&rtp_pkt).expect("encrypt");
        assert_ne!(
            &encrypted[12..],
            &rtp_pkt[12..],
            "payload should be encrypted"
        );

        let decrypted = decrypt_ctx.decrypt_rtp(&encrypted).expect("decrypt");
        assert_eq!(&decrypted[..], &rtp_pkt[..], "round-trip should match");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dtls_handshake_succeeds_when_fingerprints_match() {
        let server_cert = Certificate::generate_self_signed(vec!["server".into()]).unwrap();
        let client_cert = Certificate::generate_self_signed(vec!["client".into()]).unwrap();
        // Each side is given the *other's* real fingerprint, as SDP would carry.
        let server_expects = fp_of(&client_cert);
        let client_expects = fp_of(&server_cert);

        let (server_result, client_result) = handshake_pair(
            server_cert,
            client_cert,
            Some(server_expects),
            Some(client_expects),
        )
        .await;

        assert!(
            server_result.is_some() && client_result.is_some(),
            "matching fingerprints must complete the handshake"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dtls_handshake_rejects_a_mismatched_peer_fingerprint() {
        let server_cert = Certificate::generate_self_signed(vec!["server".into()]).unwrap();
        let client_cert = Certificate::generate_self_signed(vec!["client".into()]).unwrap();
        // The server is told to expect a fingerprint that is NOT the client's —
        // a stand-in for a MITM presenting a different cert. The server must
        // abort, so no keying material is exported on either side.
        let wrong = PeerFingerprint {
            algorithm: "sha-256".to_string(),
            hex_colon: "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:\
                        00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF"
                .to_string(),
        };

        let (server_result, client_result) =
            handshake_pair(server_cert, client_cert, Some(wrong), None).await;

        assert!(
            server_result.is_none(),
            "server must reject a mismatched client fingerprint"
        );
        assert!(
            client_result.is_none(),
            "the aborted handshake must also fail the peer"
        );
    }

    #[test]
    fn fingerprint_parse_bare_hex_assumes_sha256() {
        let fp = PeerFingerprint::parse("a1:b2:c3").unwrap();
        assert_eq!(fp.algorithm, "sha-256");
        assert_eq!(fp.hex_colon, "A1:B2:C3");
    }

    #[test]
    fn fingerprint_parse_algorithm_prefixed() {
        let fp = PeerFingerprint::parse("SHA-256 a1:b2:c3").unwrap();
        assert_eq!(fp.algorithm, "sha-256");
        assert_eq!(fp.hex_colon, "A1:B2:C3");
    }

    #[test]
    fn fingerprint_parse_empty_is_none() {
        assert!(PeerFingerprint::parse("").is_none());
        assert!(PeerFingerprint::parse("   ").is_none());
    }

    #[test]
    fn fingerprint_matches_der_of_own_cert() {
        let cert = Certificate::generate_self_signed(vec!["peer".into()]).unwrap();
        let der = cert.certificate.first().unwrap().as_ref();
        let fp = fp_of(&cert);
        assert!(fp.matches_der(der), "a cert must match its own fingerprint");

        // Case-insensitive on the hex.
        let lower = PeerFingerprint {
            algorithm: "sha-256".to_string(),
            hex_colon: fp.hex_colon.to_ascii_lowercase(),
        };
        assert!(lower.matches_der(der));
    }

    #[test]
    fn fingerprint_rejects_wrong_hash_and_unknown_algorithm() {
        let cert = Certificate::generate_self_signed(vec!["peer".into()]).unwrap();
        let der = cert.certificate.first().unwrap().as_ref();

        let mut wrong = fp_of(&cert);
        // Flip the first hex nibble.
        let first = if wrong.hex_colon.starts_with('0') {
            '1'
        } else {
            '0'
        };
        wrong.hex_colon.replace_range(0..1, &first.to_string());
        assert!(!wrong.matches_der(der), "a flipped digit must not match");

        // A correct digest under an unsupported algorithm must fail closed.
        let unknown = PeerFingerprint {
            algorithm: "sha-1".to_string(),
            hex_colon: crate::dtls::sha256_fingerprint(der),
        };
        assert!(
            !unknown.matches_der(der),
            "unsupported algorithm must fail closed"
        );
    }
}
