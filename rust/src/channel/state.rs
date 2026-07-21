// ChannelState — the single owner of per-channel state.

use std::net::SocketAddr;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use parking_lot::Mutex as PLMutex;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::actor::Event;
use super::commands::{ChannelId, Direction, RemoteConfig};
use super::dtls_session::SrtpKeyingMaterial;
use super::jitter::JitterBuffer;
use super::rtcp_stats::{RemoteReport, RxStats, DEFAULT_CLOCK_RATE};
use super::rtp::RtpPacket;

/// Close bookkeeping. Set once, observed on task exit.
#[derive(Debug, Clone, Default)]
pub struct CloseInfo {
    #[allow(dead_code)]
    pub reason: String,
}

pub struct ChannelState {
    pub id: ChannelId,
    pub local_addr: SocketAddr,
    pub remote_addr: Arc<PLMutex<Option<SocketAddr>>>,
    pub remote: Option<RemoteConfig>,
    pub direction: Direction,

    pub rtp_sock: Arc<UdpSocket>,
    pub rtcp_sock: Arc<UdpSocket>,

    pub jitter: Arc<PLMutex<JitterBuffer>>,

    // --- RTCP (RFC 3550) ---
    /// Receiver accounting for the remote source (loss/jitter, LSR/DLSR).
    pub rx_stats: Arc<PLMutex<RxStats>>,
    /// The peer's view of the stream we send, from its SR/RR about us.
    pub remote_report: Arc<PLMutex<RemoteReport>>,
    /// Canonical name emitted in SDES; stable for the channel's lifetime.
    pub cname: String,

    #[allow(dead_code)]
    pub out_pool: Vec<RtpPacket>,

    pub out_sn: u16,
    pub out_ts: u32,
    pub ssrc: u32,

    pub echo: bool,
    pub remote_confirmed: bool,

    pub codecx: crate::codec::CodecBundle,
    pub remote_pt: u8,

    pub tick_count: u64,
    pub ticks_without_rtp: u64,

    pub in_count: Arc<AtomicU64>,
    pub in_dropped: u64,
    pub out_count: u64,
    /// Payload octets sent — the RTCP SR sender-info octet count.
    pub out_octets: u64,

    pub rfc2833_pt: u8,
    pub pending_events: Vec<Event>,
    pub close_info: Option<CloseInfo>,
    pub port_reservation: Option<crate::portpool::PortReservation>,

    pub local_icepwd: Arc<PLMutex<String>>,
    pub remote_icepwd: String,

    // Recv loop — runs for the channel's lifetime, reads socket continuously.
    pub recv_cancel: Option<CancellationToken>,

    // DTLS
    pub dtls_inbound_tx: Arc<PLMutex<Option<mpsc::Sender<Vec<u8>>>>>,
    pub dtls_result_rx:
        Option<tokio::sync::oneshot::Receiver<Option<super::dtls_session::HandshakeResult>>>,
    /// AbortHandle for the spawned DTLS handshake task. Aborted on
    /// channel close so an orphan handshake can't keep polling a dead
    /// transport (which would busy-spin a tokio worker).
    pub dtls_handshake_abort: Option<tokio::task::AbortHandle>,
    pub srtp_keys: Option<SrtpKeyingMaterial>,
    pub srtp_encrypt: Option<webrtc_srtp::context::Context>,
    pub srtp_decrypt: Option<webrtc_srtp::context::Context>,
}

impl ChannelState {
    pub fn new(
        id: ChannelId,
        local_addr: SocketAddr,
        rtp_sock: UdpSocket,
        rtcp_sock: UdpSocket,
        ssrc: u32,
    ) -> Self {
        Self {
            id,
            local_addr,
            remote_addr: Arc::new(PLMutex::new(None)),
            remote: None,
            direction: Direction::default(),
            rtp_sock: Arc::new(rtp_sock),
            rtcp_sock: Arc::new(rtcp_sock),
            jitter: Arc::new(PLMutex::new(JitterBuffer::new(32, 10))),
            rx_stats: Arc::new(PLMutex::new(RxStats::new(DEFAULT_CLOCK_RATE))),
            remote_report: Arc::new(PLMutex::new(RemoteReport::default())),
            cname: format!("{ssrc:08x}@{}", local_addr.ip()),
            out_pool: Vec::new(),
            out_sn: 0,
            out_ts: 0,
            ssrc,
            echo: false,
            remote_confirmed: false,
            codecx: crate::codec::CodecBundle::new(),
            remote_pt: 0,
            tick_count: 0,
            ticks_without_rtp: 0,
            in_count: Arc::new(AtomicU64::new(0)),
            in_dropped: 0,
            out_count: 0,
            out_octets: 0,
            rfc2833_pt: 101,
            pending_events: Vec::new(),
            close_info: None,
            port_reservation: None,
            local_icepwd: Arc::new(PLMutex::new(String::new())),
            remote_icepwd: String::new(),
            recv_cancel: None,
            dtls_inbound_tx: Arc::new(PLMutex::new(None)),
            dtls_result_rx: None,
            dtls_handshake_abort: None,
            srtp_keys: None,
            srtp_encrypt: None,
            srtp_decrypt: None,
        }
    }

    pub fn get_remote_addr(&self) -> Option<SocketAddr> {
        *self.remote_addr.lock()
    }

    pub fn set_remote_addr(&self, addr: SocketAddr) {
        *self.remote_addr.lock() = Some(addr);
    }
}
