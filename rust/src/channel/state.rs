// ChannelState — the single owner of per-channel state.
//
// The actor task holds `Ownership` (Local / Mixed / Closing); when Local, it
// holds a `Box<ChannelState>` that `tick::run()` mutates directly. When a
// channel joins a mix, the state moves into the MixGroup actor. See the
// channel-port design note for the rationale.

use std::net::SocketAddr;

use tokio::net::UdpSocket;

use super::actor::Event;
use super::commands::{ChannelId, Command, Direction, RemoteConfig};
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

    // 2-channel mix relay target — when Some, every inbound packet is
    // forwarded straight to this address (the *other* channel's remote) and
    // the normal echo/silence outbound is skipped. n>2 mixing requires real
    // sample-level mix math and lands in mixer.rs.
    pub mix_peer_remote: Option<SocketAddr>,
    /// Outbound payload type for the peer when forwarding via mix relay. If
    /// it differs from the inbound PT we transcode (G.711 only).
    pub mix_peer_pt: u8,
    /// RFC 2833 PT to stamp onto DTMF packets we relay to the peer. Peer
    /// may have negotiated a different rfc2833pt to us.
    pub mix_peer_rfc2833_pt: u8,
    /// Handle to the mix peer's actor — set by `BindMixPeer`. When set, this
    /// channel is in a 2-chan mix; any change to *our* remote gets pushed
    /// over as a `SetPeerRemote` so the peer's outbound targets stay in sync
    /// when JS calls `.remote()` after `.mix()`. None when unbound.
    pub mix_peer_handle: Option<tokio::sync::mpsc::Sender<Command>>,
    /// N-way mix: shared group state. When set, tick.rs deposits this
    /// channel's decoded inbound frame into the group and reads
    /// (`summed - own`) back to encode + send. Mutually exclusive with
    /// `mix_peer_*` (2-chan fast path) — the facade promotes 2-chan to
    /// N-way on the second `mix()` call.
    pub mix_group: Option<std::sync::Arc<parking_lot::Mutex<super::mixer::MixGroupShared>>>,
    /// This channel's slot in the group.
    pub mix_group_idx: usize,
    /// Group version we last emitted an output packet at. Compare against
    /// `group.max_other_deposit(idx)` to decide whether to emit this tick.
    pub mix_last_emit: u64,
    /// Stateful transcoder (G.722 needs filter history) for the mix relay.
    pub transcoder: crate::codec::Transcoder,


    /// Outbound payload type for our own remote (set from params.remote.codec).
    pub remote_pt: u8,

    // Tick counters — for idle timeout, stats.
    pub tick_count: u64,

    // Stats reported in the close event.
    pub in_count: u64,
    pub in_dropped: u64,
    pub in_skip: u64,
    pub out_count: u64,

    // RFC 2833 payload type — defaults to 101 unless the remote config
    // negotiates a different one.
    pub rfc2833_pt: u8,

    // Events the tick wants the actor to forward to the JS callback. Drained
    // after each tick.run().
    pub pending_events: Vec<Event>,

    // Close bookkeeping (populated by Command::Close).
    pub close_info: Option<CloseInfo>,

    /// Port-pool reservation. `Some` when the channel's RTP/RTCP pair came
    /// from the managed pool; drops back into the pool when the actor ends
    /// and state falls out of scope. `None` when ephemeral ports were used
    /// (pool uninitialized — primarily tests that don't call `run({ports})`).
    pub port_reservation: Option<crate::portpool::PortReservation>,

    /// ICE password for this side of the channel (our own). Used to verify
    /// MESSAGE-INTEGRITY on inbound STUN Binding Requests and to sign
    /// Binding Responses we emit. Empty string disables STUN handling.
    pub local_icepwd: String,
    /// Peer's ICE password — informational only today (see `RemoteConfig`).
    pub remote_icepwd: String,
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
            mix_peer_remote: None,
            mix_peer_pt: 0,
            mix_peer_rfc2833_pt: 101,
            mix_peer_handle: None,
            mix_group: None,
            mix_group_idx: 0,
            mix_last_emit: 0,
            transcoder: crate::codec::Transcoder::new(),
            remote_pt: 0,
            tick_count: 0,
            in_count: 0,
            in_dropped: 0,
            in_skip: 0,
            out_count: 0,
            rfc2833_pt: 101,
            pending_events: Vec::new(),
            close_info: None,
            port_reservation: None,
            local_icepwd: String::new(),
            remote_icepwd: String::new(),
        }
    }
}
