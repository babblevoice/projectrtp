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
use tokio_util::sync::CancellationToken;

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
    pub cancel: CancellationToken,
}

pub fn spawn(cfg: RtcpLoopConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run(cfg))
}

async fn run(cfg: RtcpLoopConfig) {
    let mut buf = [0u8; rtp::RTP_MAX_LENGTH];
    loop {
        tokio::select! {
            biased;
            _ = cfg.cancel.cancelled() => break,
            result = cfg.sock.recv_from(&mut buf) => {
                match result {
                    Ok((n, _peer)) => handle_packet(&cfg, &buf[..n]),
                    Err(_) => break,
                }
            }
        }
    }
}

fn handle_packet(cfg: &RtcpLoopConfig, pkt: &[u8]) {
    // Malformed / non-RTCP compound — ignore (Tier 1 is best-effort).
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
                cfg.rx_stats
                    .lock()
                    .note_sender_report(info.ntp_sec, info.ntp_frac, now);
                fold_reports(cfg, &reports, now_ntp_mid);
            }
            RtcpItem::ReceiverReport { reports, .. } => {
                fold_reports(cfg, &reports, now_ntp_mid);
            }
            RtcpItem::Bye { .. } => {
                // Peer signalled end of stream; teardown is driven elsewhere.
            }
        }
    }
}

fn fold_reports(cfg: &RtcpLoopConfig, reports: &[ReportBlock], now_ntp_mid: u32) {
    for rb in reports {
        if rb.ssrc == cfg.local_ssrc {
            cfg.remote_report.lock().update_from(rb, now_ntp_mid);
        }
    }
}
