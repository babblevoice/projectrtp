// ChannelState — the single owner of per-channel state.
//
// The actor task holds `Ownership` (Local / Mixed / Closing); when Local, it
// holds a `Box<ChannelState>` that `tick::run()` mutates directly. When a
// channel joins a mix, the state moves into the MixGroup actor. See the
// channel-port design note for the rationale.

use std::net::SocketAddr;

use tokio::net::UdpSocket;

use super::commands::{ChannelId, Direction, RemoteConfig};
use super::jitter::JitterBuffer;
use super::rtp::RtpPacket;

/// Close bookkeeping. Set once, observed on task exit.
#[derive(Debug, Clone, Default)]
pub struct CloseInfo {
    pub reason: String,
}

/// Minimal, flesh out as tick.rs / player.rs / recorder.rs land.
pub struct ChannelState {
    pub id: ChannelId,
    pub local_addr: SocketAddr,
    pub remote_addr: Option<SocketAddr>,
    pub remote: Option<RemoteConfig>,
    pub direction: Direction,

    // Sockets owned for the channel's lifetime.
    pub rtp_sock: UdpSocket,
    pub rtcp_sock: UdpSocket,

    // Inbound path.
    pub jitter: JitterBuffer,

    // Outbound path — packet pool (each BytesMut == RTP_MAX_LENGTH capacity).
    // Pool wiring lands alongside tick.rs; for now just a fixed Vec of reusable
    // buffers so the shape is committed.
    pub out_pool: Vec<RtpPacket>,

    // RTP send-side state.
    pub out_sn: u16,
    pub out_ts: u32,
    pub ssrc: u32,

    // Echo / recv-confirmed / remote-confirmed flags — filled as features land.
    pub echo: bool,
    pub remote_confirmed: bool,

    // Tick counters — for idle timeout, stats.
    pub tick_count: u64,

    // Close bookkeeping (populated by Command::Close).
    pub close_info: Option<CloseInfo>,
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
            remote_addr: None,
            remote: None,
            direction: Direction::default(),
            rtp_sock,
            rtcp_sock,
            jitter: JitterBuffer::new(32, 10),
            out_pool: Vec::new(),
            out_sn: 0,
            out_ts: 0,
            ssrc,
            echo: false,
            remote_confirmed: false,
            tick_count: 0,
            close_info: None,
        }
    }
}
