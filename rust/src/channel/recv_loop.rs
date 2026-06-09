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

use parking_lot::Mutex as PLMutex;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::jitter::JitterBuffer;
use super::rtp::{self, RtpPacket};
use crate::stun;

pub struct RecvLoopConfig {
    pub sock: Arc<UdpSocket>,
    pub jitter: Arc<PLMutex<JitterBuffer>>,
    pub remote_addr: Arc<PLMutex<Option<SocketAddr>>>,
    pub in_count: Arc<AtomicU64>,
    pub local_icepwd: Arc<PLMutex<String>>,
    pub dtls_tx: Arc<PLMutex<Option<mpsc::Sender<Vec<u8>>>>>,
    pub cancel: CancellationToken,
}

pub fn spawn(cfg: RecvLoopConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run(cfg))
}

async fn run(cfg: RecvLoopConfig) {
    let mut buf = [0u8; rtp::RTP_MAX_LENGTH];
    loop {
        tokio::select! {
            biased;
            _ = cfg.cancel.cancelled() => break,
            result = cfg.sock.recv_from(&mut buf) => {
                match result {
                    Ok((n, peer)) => {
                        *cfg.remote_addr.lock() = Some(peer);
                        handle_packet(&cfg, &buf[..n], peer).await;
                    }
                    Err(_) => break,
                }
            }
        }
    }
}

async fn handle_packet(cfg: &RecvLoopConfig, pkt: &[u8], peer: SocketAddr) {
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

    // RTP / DTMF — push to jitter. DTMF (rfc2833) classification happens
    // at pop time in the tick, since it needs access to Subsystems.
    if pkt.len() >= rtp::RTP_FIXED_HEADER_LEN {
        cfg.in_count.fetch_add(1, Ordering::Relaxed);
        let mut rp = RtpPacket::new();
        rp.as_mut_slice_for_fill(pkt.len()).copy_from_slice(pkt);
        cfg.jitter.lock().push(rp);
    }
}
