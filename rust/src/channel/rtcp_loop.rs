// Per-channel inbound RTCP reader — the mirror of `recv_loop.rs`, but on the
// separate P+1 control socket instead of the RTP socket. It reads RTCP
// compound packets, parses SR/RR/BYE (RFC 3550 §6), and folds them into the
// channel's shared accounting:
//
//   SR  → record the sender NTP for our next report's LSR/DLSR
//         (`note_sender_report`), then fold any report block *about our SSRC*
//         into `remote_report` (peer-reported loss/jitter + RTT via LSR/DLSR).
//   RR  → fold the report block about our SSRC into `remote_report`.
//   BYE → noted only; channel teardown is driven by the JS layer / RTP timeout.
//
// Spawned once per channel lifetime and cancelled in the actor close path via
// the same `CancellationToken` that stops `recv_loop`.

use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex as PLMutex;
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::dtls_session::{remote_srtp_params, SrtpKeyingMaterial};
use super::rtcp::{self, ReportBlock, RtcpItem};
use super::rtcp_stats::{RemoteReport, RxStats};
use super::rtp;

pub struct RtcpLoopConfig {
    pub sock: Arc<UdpSocket>,
    pub rx_stats: Arc<PLMutex<RxStats>>,
    pub remote_report: Arc<PLMutex<RemoteReport>>,
    /// Our SSRC — a report block whose SSRC matches describes the stream *we*
    /// send, so it is the one the peer's loss/jitter/RTT figures are about.
    pub local_ssrc: u32,
    /// DTLS keying material, published by the tick once the handshake
    /// completes. `None` until then (and for non-secure channels), in which
    /// case inbound RTCP is treated as cleartext.
    pub key_rx: watch::Receiver<Option<SrtpKeyingMaterial>>,
    pub cancel: CancellationToken,
}

pub fn spawn(cfg: RtcpLoopConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run(cfg))
}

async fn run(cfg: RtcpLoopConfig) {
    let mut buf = [0u8; rtp::RTP_MAX_LENGTH];
    // Built lazily on the first packet after keying material arrives. Once
    // present, all inbound RTCP on a secure channel is SRTCP.
    let mut srtcp_decrypt: Option<webrtc_srtp::context::Context> = None;
    loop {
        tokio::select! {
            biased;
            _ = cfg.cancel.cancelled() => break,
            result = cfg.sock.recv_from(&mut buf) => {
                match result {
                    Ok((n, _peer)) => {
                        maybe_build_decrypt(&cfg.key_rx, &mut srtcp_decrypt);
                        handle_rtcp(
                            &buf[..n],
                            srtcp_decrypt.as_mut(),
                            &cfg.rx_stats,
                            &cfg.remote_report,
                            cfg.local_ssrc,
                        );
                    }
                    Err(_) => break,
                }
            }
        }
    }
}

/// Build the SRTCP decrypt context once the handshake has published keys.
/// No-op if already built or no keys yet. Shared with `recv_loop`, which
/// builds its own context for rtcp-mux'd inbound RTCP.
pub fn maybe_build_decrypt(
    key_rx: &watch::Receiver<Option<SrtpKeyingMaterial>>,
    slot: &mut Option<webrtc_srtp::context::Context>,
) {
    if slot.is_some() {
        return;
    }
    if let Some(keys) = key_rx.borrow().as_ref() {
        let (key, salt, profile) = remote_srtp_params(keys);
        if let Ok(ctx) = webrtc_srtp::context::Context::new(key, salt, profile, None, None) {
            *slot = Some(ctx);
        }
    }
}

/// Decrypt (if a context is present) then parse and fold an inbound RTCP
/// datagram into the shared accounting. Shared by this dedicated P+1 loop and,
/// under rtcp-mux (RFC 5761), by `recv_loop` reading off the RTP port.
pub fn handle_rtcp(
    pkt: &[u8],
    decrypt: Option<&mut webrtc_srtp::context::Context>,
    rx_stats: &PLMutex<RxStats>,
    remote_report: &PLMutex<RemoteReport>,
    local_ssrc: u32,
) {
    match decrypt {
        // Auth failure / malformed SRTCP → decrypt errors, packet dropped.
        Some(ctx) => {
            if let Ok(plain) = ctx.decrypt_rtcp(pkt) {
                parse_and_fold(&plain, rx_stats, remote_report, local_ssrc);
            }
        }
        None => parse_and_fold(pkt, rx_stats, remote_report, local_ssrc),
    }
}

fn parse_and_fold(
    pkt: &[u8],
    rx_stats: &PLMutex<RxStats>,
    remote_report: &PLMutex<RemoteReport>,
    local_ssrc: u32,
) {
    // Malformed / non-RTCP compound — ignore (best-effort).
    let items = match rtcp::parse(pkt) {
        Ok(items) => items,
        Err(_) => return,
    };

    let now = Instant::now();
    let (ntp_sec, ntp_frac) = rtcp::ntp_now();
    let now_ntp_mid = rtcp::ntp_middle_32(ntp_sec, ntp_frac);

    for item in items {
        match item {
            RtcpItem::SenderReport { info, reports, .. } => {
                rx_stats
                    .lock()
                    .note_sender_report(info.ntp_sec, info.ntp_frac, now);
                fold_reports(remote_report, local_ssrc, &reports, now_ntp_mid);
            }
            RtcpItem::ReceiverReport { reports, .. } => {
                fold_reports(remote_report, local_ssrc, &reports, now_ntp_mid);
            }
            RtcpItem::Bye { .. } => {
                // Peer signalled end of stream; teardown is driven elsewhere.
            }
        }
    }
}

fn fold_reports(
    remote_report: &PLMutex<RemoteReport>,
    local_ssrc: u32,
    reports: &[ReportBlock],
    now_ntp_mid: u32,
) {
    for rb in reports {
        if rb.ssrc == local_ssrc {
            remote_report.lock().update_from(rb, now_ntp_mid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use webrtc_srtp::context::Context;
    use webrtc_srtp::protection_profile::ProtectionProfile;

    // A paired encrypt/decrypt context sharing one key — the decrypt side
    // recovers what the encrypt side protected. Mirrors the SRTP round-trip
    // in `dtls_session::tests`, but for the RTCP (SRTCP) methods.
    fn srtcp_pair() -> (Context, Context) {
        let key = [0x11u8; 16];
        let salt = [0x22u8; 14];
        let profile = ProtectionProfile::Aes128CmHmacSha1_80;
        let enc = Context::new(&key, &salt, profile, None, None).unwrap();
        let dec = Context::new(&key, &salt, profile, None, None).unwrap();
        (enc, dec)
    }

    #[test]
    fn srtcp_roundtrip_preserves_compound() {
        // A plain RR + SDES compound (what `maybe_send_rtcp` emits pre-audio).
        let compound =
            rtcp::build_compound(0xDEAD_BEEF, None, &[], Some("abcd1234@127.0.0.1"), false);
        let (mut enc, mut dec) = srtcp_pair();

        let protected = enc.encrypt_rtcp(&compound).expect("encrypt_rtcp");
        assert_ne!(&protected[..], &compound[..], "compound must be encrypted");
        assert!(
            protected.len() > compound.len(),
            "SRTCP appends a 4-byte index + auth tag"
        );

        let recovered = dec.decrypt_rtcp(&protected).expect("decrypt_rtcp");
        assert_eq!(
            &recovered[..],
            &compound[..],
            "round-trip must match byte-for-byte"
        );

        // ...and the recovered bytes still parse as the original RR.
        let items = rtcp::parse(&recovered).expect("parse");
        assert!(matches!(
            items.first(),
            Some(RtcpItem::ReceiverReport { .. })
        ));
    }

    #[test]
    fn srtcp_rejects_tampered_packet() {
        let compound = rtcp::build_compound(0x1234_5678, None, &[], Some("x@127.0.0.1"), false);
        let (mut enc, mut dec) = srtcp_pair();

        let mut protected = enc.encrypt_rtcp(&compound).expect("encrypt_rtcp").to_vec();
        let last = protected.len() - 1;
        protected[last] ^= 0xFF; // corrupt the trailing auth tag

        assert!(
            dec.decrypt_rtcp(&protected).is_err(),
            "a tampered SRTCP packet must fail authentication"
        );
    }
}
