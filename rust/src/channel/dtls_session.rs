// DTLS session — pure-Rust via webrtc-dtls.
//
// The channel's UDP socket carries RTP, DTLS and STUN multiplexed on one
// port. The tick/mixer classify demuxes by first byte: STUN (0-3), DTLS
// (20-63), RTP (128-191). DTLS datagrams are fed into a DtlsTransport
// (mpsc-backed Conn adapter) that the webrtc-dtls DTLSConn reads from.
// Outbound DTLS frames come back via a second mpsc and are sent on the
// real socket by the tick.
//
// After the handshake completes, keying material is exported and used to
// create SRTP encrypt/decrypt contexts (see srtp_ctx.rs).

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;
use webrtc_dtls::config::Config as DtlsConfig;
use webrtc_dtls::config::ExtendedMasterSecretType;
use webrtc_dtls::conn::DTLSConn;
use webrtc_dtls::crypto::Certificate;
use webrtc_srtp::protection_profile::ProtectionProfile;

use crate::channel::commands::DtlsSetup;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PeerFingerprint {
    pub algorithm: String,
    pub hex_colon: String,
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
pub struct DtlsTransport {
    inbound_rx: tokio::sync::Mutex<mpsc::Receiver<Vec<u8>>>,
    sock: Arc<tokio::net::UdpSocket>,
    local_addr: SocketAddr,
    remote_addr: Mutex<Option<SocketAddr>>,
}

#[async_trait]
impl webrtc_util::Conn for DtlsTransport {
    async fn connect(&self, addr: SocketAddr) -> webrtc_util::Result<()> {
        *self.remote_addr.lock().unwrap() = Some(addr);
        Ok(())
    }

    async fn recv(&self, buf: &mut [u8]) -> webrtc_util::Result<usize> {
        let data = self.inbound_rx.lock().await.recv().await
            .ok_or_else(|| webrtc_util::Error::Other("dtls channel closed".into()))?;
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        Ok(n)
    }

    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        let n = self.recv(buf).await?;
        let addr = self.remote_addr.lock().unwrap().unwrap_or(self.local_addr);
        Ok((n, addr))
    }

    async fn send(&self, buf: &[u8]) -> webrtc_util::Result<usize> {
        let remote = self.remote_addr.lock().unwrap().unwrap_or(self.local_addr);
        self.sock.send_to(buf, remote).await
            .map_err(|e| webrtc_util::Error::Other(e.to_string()))
    }

    async fn send_to(&self, buf: &[u8], target: SocketAddr) -> webrtc_util::Result<usize> {
        self.sock.send_to(buf, target).await
            .map_err(|e| webrtc_util::Error::Other(e.to_string()))
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        Ok(self.local_addr)
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        *self.remote_addr.lock().unwrap()
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
) -> tokio::sync::oneshot::Receiver<Option<HandshakeResult>> {
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();

    let transport = Arc::new(DtlsTransport {
        inbound_rx: tokio::sync::Mutex::new(inbound_rx),
        sock,
        local_addr,
        remote_addr: Mutex::new(None),
    });

    let is_client = setup == DtlsSetup::Active;
    let srtp_profiles = vec![
        webrtc_dtls::extension::extension_use_srtp::SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80,
    ];

    tokio::spawn(async move {
        let config = DtlsConfig {
            certificates: vec![certificate],
            srtp_protection_profiles: srtp_profiles,
            insecure_skip_verify: true,
            extended_master_secret: ExtendedMasterSecretType::Require,
            ..Default::default()
        };

        match DTLSConn::new(transport, config, is_client, None).await {
            Ok(conn) => {
                use webrtc_util::KeyingMaterialExporter;
                let state = conn.connection_state().await;
                let label = "EXTRACTOR-dtls_srtp";
                let profile = ProtectionProfile::Aes128CmHmacSha1_80;
                let km_len = 2 * (profile.key_len() + profile.salt_len());
                match state.export_keying_material(label, &[], km_len).await {
                    Ok(km) => {
                        let _ = result_tx.send(Some(HandshakeResult {
                            keying_material: km,
                            profile,
                            is_client,
                        }));
                    }
                    Err(_) => {
                        let _ = result_tx.send(None);
                    }
                }
            }
            Err(_) => {
                let _ = result_tx.send(None);
            }
        }
    });

    result_rx
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
    let client_write_key = km[off..off + key_len].to_vec(); off += key_len;
    let server_write_key = km[off..off + key_len].to_vec(); off += key_len;
    let client_write_salt = km[off..off + salt_len].to_vec(); off += salt_len;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::net::UdpSocket;

    #[tokio::test]
    async fn dtls_handshake_between_two_peers() {
        let server_cert = Certificate::generate_self_signed(vec!["server".into()]).unwrap();
        let client_cert = Certificate::generate_self_signed(vec!["client".into()]).unwrap();

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

        // Relay: read server_sock → client_dtls_tx, read client_sock → server_dtls_tx.
        let srv_sock2 = server_sock.clone();
        let cli_sock2 = client_sock.clone();
        let relay = tokio::spawn(async move {
            let mut sbuf = [0u8; 2048];
            let mut cbuf = [0u8; 2048];
            loop {
                tokio::select! {
                    Ok((n, _)) = srv_sock2.recv_from(&mut sbuf) => {
                        if client_dtls_tx.send(sbuf[..n].to_vec()).await.is_err() { break; }
                    }
                    Ok((n, _)) = cli_sock2.recv_from(&mut cbuf) => {
                        if server_dtls_tx.send(cbuf[..n].to_vec()).await.is_err() { break; }
                    }
                }
            }
        });

        let server_result_rx = spawn_handshake(
            DtlsSetup::Passive, server_addr, server_sock, server_dtls_rx, server_cert,
        );
        let client_result_rx = spawn_handshake(
            DtlsSetup::Active, client_addr, client_sock, client_dtls_rx, client_cert,
        );

        let server_result = tokio::time::timeout(Duration::from_secs(5), server_result_rx)
            .await
            .expect("server handshake timeout")
            .expect("server oneshot dropped");
        let client_result = tokio::time::timeout(Duration::from_secs(5), client_result_rx)
            .await
            .expect("client handshake timeout")
            .expect("client oneshot dropped");

        assert!(server_result.is_some(), "server handshake failed");
        assert!(client_result.is_some(), "client handshake failed");

        let server_km = server_result.unwrap();
        let client_km = client_result.unwrap();

        assert_eq!(server_km.keying_material, client_km.keying_material);
        assert!(!server_km.is_client);
        assert!(client_km.is_client);

        let server_keys = split_keying_material(
            &server_km.keying_material, server_km.profile, server_km.is_client,
        );
        let client_keys = split_keying_material(
            &client_km.keying_material, client_km.profile, client_km.is_client,
        );
        assert_eq!(server_keys.client_write_key, client_keys.client_write_key);
        assert_eq!(server_keys.server_write_key, client_keys.server_write_key);

        // Verify SRTP encrypt/decrypt round-trip.
        let mut encrypt_ctx = webrtc_srtp::context::Context::new(
            &client_keys.client_write_key,
            &client_keys.client_write_salt,
            client_keys.profile, None, None,
        ).expect("srtp encrypt ctx");

        let mut decrypt_ctx = webrtc_srtp::context::Context::new(
            &server_keys.client_write_key,
            &server_keys.client_write_salt,
            server_keys.profile, None, None,
        ).expect("srtp decrypt ctx");

        let mut rtp_pkt = vec![0x80, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0xA0, 0x00, 0x00, 0x00, 0x01];
        rtp_pkt.extend_from_slice(&[0x55u8; 160]);

        let encrypted = encrypt_ctx.encrypt_rtp(&rtp_pkt).expect("encrypt");
        assert_ne!(&encrypted[12..], &rtp_pkt[12..], "payload should be encrypted");

        let decrypted = decrypt_ctx.decrypt_rtp(&encrypted).expect("decrypt");
        assert_eq!(&decrypted[..], &rtp_pkt[..], "round-trip should match");

        relay.abort();
    }
}
