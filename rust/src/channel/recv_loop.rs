// Per-channel receive loop — reads the UDP socket continuously and
// classifies packets by first byte, matching the C++ IOCP model:
//
//   STUN (0-3)     → respond immediately (no tick latency)
//   DTLS (20-63)   → feed to DTLSConn via mpsc (fast handshake)
//   RTP  (128-191) → push to jitter buffer under lock (tick pops later)
//
// Spawned once per channel lifetime. Survives Local↔Mixed transitions
// because it holds Arc references to the socket and jitter buffer.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex as PLMutex;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use super::dtls_session::SrtpKeyingMaterial;
use super::jitter::JitterBuffer;
use super::rtcp_loop;
use super::rtcp_stats::{RemoteReport, RxStats};
use super::rtp::{self, RtpPacket};
use crate::stun;

pub struct RecvLoopConfig {
    pub sock: Arc<UdpSocket>,
    pub jitter: Arc<PLMutex<JitterBuffer>>,
    pub remote_addr: Arc<PLMutex<Option<SocketAddr>>>,
    pub in_count: Arc<AtomicU64>,
    /// RFC 3550 receiver accounting — fed on every inbound RTP packet.
    pub rx_stats: Arc<PLMutex<RxStats>>,
    /// Peer's view of the stream we send — folded from rtcp-mux'd RTCP
    /// (RFC 5761) that arrives on the RTP port. Shared with `rtcp_loop`.
    pub remote_report: Arc<PLMutex<RemoteReport>>,
    /// Our SSRC — selects the muxed report block that is about our stream.
    pub local_ssrc: u32,
    pub local_icepwd: Arc<PLMutex<String>>,
    pub dtls_tx: Arc<PLMutex<Option<mpsc::Sender<Vec<u8>>>>>,
    /// DTLS keying material for decrypting muxed SRTCP; `None` until the
    /// handshake completes (and always, for non-secure channels).
    pub key_rx: watch::Receiver<Option<SrtpKeyingMaterial>>,
    pub cancel: CancellationToken,
}

pub fn spawn(cfg: RecvLoopConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run(cfg))
}

async fn run(cfg: RecvLoopConfig) {
    let mut buf = [0u8; rtp::RTP_MAX_LENGTH];
    // Built lazily on the first muxed SRTCP packet after keys arrive; a plain
    // (non-secure) channel never populates it and muxed RTCP stays cleartext.
    let mut srtcp_decrypt: Option<webrtc_srtp::context::Context> = None;
    loop {
        tokio::select! {
            biased;
            _ = cfg.cancel.cancelled() => break,
            result = cfg.sock.recv_from(&mut buf) => {
                match result {
                    Ok((n, peer)) => {
                        *cfg.remote_addr.lock() = Some(peer);
                        handle_packet(&cfg, &buf[..n], peer, &mut srtcp_decrypt).await;
                    }
                    Err(_) => break,
                }
            }
        }
    }
}

async fn handle_packet(
    cfg: &RecvLoopConfig,
    pkt: &[u8],
    peer: SocketAddr,
    srtcp_decrypt: &mut Option<webrtc_srtp::context::Context>,
) {
    if pkt.is_empty() {
        return;
    }
    let first = pkt[0];

    // STUN — respond immediately.
    if stun::is_stun(pkt) {
        let icepwd = cfg.local_icepwd.lock().clone();
        if icepwd.is_empty() {
            return;
        }
        let key = icepwd.as_bytes().to_vec();
        let mut req = pkt.to_vec();
        let mut resp = [0u8; rtp::RTP_MAX_LENGTH];
        let n = stun::handle(&mut req, &mut resp, peer, &key, &key);
        if n > 0 {
            let _ = cfg.sock.send_to(&resp[..n], peer).await;
        }
        return;
    }

    // DTLS — feed to DTLSConn if active.
    if (20..=63).contains(&first) {
        if let Some(tx) = cfg.dtls_tx.lock().clone() {
            let _ = tx.try_send(pkt.to_vec());
        }
        return;
    }

    // rtcp-mux (RFC 5761): RTCP carried on the RTP port. See `is_muxed_rtcp`
    // for the demux rule. SRTP/SRTCP leave the header in cleartext, so this
    // classifies before any decrypt. On a non-mux channel the peer targets P+1
    // and `rtcp_loop` handles it, so this branch simply never fires.
    if is_muxed_rtcp(pkt) {
        rtcp_loop::maybe_build_decrypt(&cfg.key_rx, srtcp_decrypt);
        rtcp_loop::handle_rtcp(
            pkt,
            srtcp_decrypt.as_mut(),
            &cfg.rx_stats,
            &cfg.remote_report,
            cfg.local_ssrc,
        );
        return;
    }

    // RTP / DTMF — push to jitter. DTMF (rfc2833) classification happens
    // at pop time in the tick, since it needs access to Subsystems.
    if pkt.len() >= rtp::RTP_FIXED_HEADER_LEN {
        cfg.in_count.fetch_add(1, Ordering::Relaxed);
        // RFC 3550 receiver accounting: sequence/loss (A.1/A.3) and
        // interarrival jitter (A.8). DTMF (rfc2833) shares the audio stream's
        // SSRC and sequence space, so it is counted here too.
        cfg.rx_stats.lock().on_packet_at(
            rtp::ssrc(pkt),
            rtp::sequence_number(pkt),
            rtp::timestamp(pkt),
            Instant::now(),
        );
        let mut rp = RtpPacket::new();
        rp.as_mut_slice_for_fill(pkt.len()).copy_from_slice(pkt);
        cfg.jitter.lock().push(rp);
    }
}

/// RFC 5761 §4 demux: on the shared RTP port, is this datagram RTCP rather
/// than RTP? True when the first byte is in the RTP/RTCP version range
/// (128..=191) and the second byte is an RTCP packet type — SR/RR/SDES/BYE/APP
/// (200..=204). Those values map to an RTP marker+payload-type of 72..=76,
/// which RTP deliberately never assigns, so the classification is unambiguous
/// and works on SRTP/SRTCP too (the header stays in cleartext).
fn is_muxed_rtcp(pkt: &[u8]) -> bool {
    pkt.len() >= 2 && (128..=191).contains(&pkt[0]) && (200..=204).contains(&pkt[1])
}

#[cfg(test)]
mod tests {
    use super::is_muxed_rtcp;

    #[test]
    fn classifies_rtcp_packet_types() {
        // Version-2 RTCP header (0x80) + each RTCP packet type.
        for pt in 200u8..=204 {
            assert!(is_muxed_rtcp(&[0x80, pt]), "PT {pt} should demux as RTCP");
        }
    }

    #[test]
    fn rtp_audio_is_not_mistaken_for_rtcp() {
        // PCMU (pt 0), PCMA (8), G722 (9), rfc2833 (101) — with and without
        // the marker bit. None of the second bytes fall in 200..=204.
        for pt in [0u8, 8, 9, 101] {
            assert!(!is_muxed_rtcp(&[0x80, pt]), "RTP pt {pt} misread as RTCP");
            let marked = 0x80 | pt; // marker bit set
            assert!(
                !is_muxed_rtcp(&[0x80, marked]),
                "marked RTP pt {pt} misread as RTCP"
            );
        }
    }

    #[test]
    fn rejects_out_of_range_and_short() {
        assert!(!is_muxed_rtcp(&[0x80, 199]), "205- boundary below");
        assert!(!is_muxed_rtcp(&[0x80, 205]), "205 is above BYE/APP window");
        assert!(!is_muxed_rtcp(&[0x00, 200]), "STUN-range first byte");
        assert!(!is_muxed_rtcp(&[0x30, 200]), "DTLS-range first byte");
        assert!(!is_muxed_rtcp(&[0x80]), "too short");
        assert!(!is_muxed_rtcp(&[]), "empty");
    }
}
