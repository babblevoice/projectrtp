// Relay mode — a tick-free forwarding path for opaque media (video).
//
// A channel opened with `{ relay: true }` never enters the 20 ms audio
// pipeline: no jitter buffer, no codec graph, no player/recorder/DTMF.
// Instead its recv_loop decrypts each inbound RTP packet and hands a copy
// to the send task of every OTHER leg in its relay group (`fan_out`). Each
// send task keeps per-source outbound state (`Outbound`): for every stream a
// source sends (every source SSRC — a browser may add RTX, FEC or simulcast
// SSRCs to its main one) an outbound SSRC of its own and a sequence mapping
// that keeps the stream's gaps and order (`SeqMap`). Timestamp and marker
// bit pass through verbatim; the packet is re-encrypted with the receiving
// leg's keys and sent.
//
// Groups mirror the audio mix's semantics: `mix(a, b)` puts both legs in
// one group, a later `mix(a, c)` extends it (c now receives a and b, and
// they receive c), mixing legs already in *different* groups is refused,
// and `unmix()` / close removes the leg from its group so nobody forwards
// to or from it any more. A 2-leg group is the plain point-to-point relay:
// each browser sees exactly one SSRC — the leg's own — per direction.
// Keyframe requests from a receiver name the SSRC they are about; the leg
// maps that back to the source leg (`request_keyframe`) so only that
// source's remote is asked for a keyframe. NACKs are mapped back the same
// way and passed through (`relay_nacks`): SSRC and sequence numbers are
// translated into the source's own series, so the source browser
// retransmits the packets the receiver lost.
//
// Abuse limits: before fan-out, the source leg's recv_loop holds every
// authenticated inbound RTP packet to the leg's `RelayLimits` (bitrate,
// packet rate, per-stream frame rate and resolution) — see relay_limits.rs.
// A guest's modified browser can ignore every SDP limit; this cannot be.
//
// Lock order (deadlock safety): a leg's `group` slot, then the group's
// member list; when two slots are held (`join`) they are taken in
// ascending leg-id order. `retired`, `out_routes`, `pts`, `nacks`, `nack_limit`, `nack_budget` and the
// recv loop's `RelayRecv::guard` are leaves — nothing is locked while holding them. The per-packet path never holds a slot while
// taking another lock: it clones the group handle and releases the slot.
//
// Why not make the tick pipeline polymorphic: the audio tick pops exactly
// one packet per 20 ms (a 50 pps ceiling — video runs 200+ pps and bursts
// on keyframes) and synthesises its own 8 kHz timestamps, both of which
// are load-bearing for audio (see tick.rs) and fatal for video. A parallel
// path avoids touching either invariant.
//
// RTCP: the relay leg's send task owns *all* outbound RTCP for the leg —
// the periodic SR/RR in rtcp_tx is disabled for relay channels (tick.rs),
// because two `webrtc_srtp` Contexts encrypting RTCP under the same key
// would issue colliding SRTCP indices and the receiver's replay window
// would drop one stream of them. It sends PSFB PLI (keyframe requests, for
// an inbound PLI or FIR) and RTPFB generic NACK (an inbound NACK, translated
// to the source's SSRC and sequence series). A NACK is deliberately NOT
// turned into a PLI: the relay keeps the source's sequence gaps, so a
// receiver NACKs on any loss, and converting those would ask the source
// for a keyframe as often as the PLI rate limit allows under ordinary
// packet loss. The browser retransmits from its own history instead
// (without RTX negotiated it resends on the media SSRC with the original
// sequence number, which maps straight back through `SeqMap`); if that
// fails the receiver escalates to a PLI itself.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex as PLMutex;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch, Notify};
use tokio_util::sync::CancellationToken;

use super::dtls_session::{local_srtp_params, SrtpKeyingMaterial};
use super::relay_limits::{RelayLimits, TokenBucket};
use super::rtcp_stats::RxStats;
use super::rtp::RtpPacket;
use super::video_dims::VideoFormat;

/// Depth of the per-leg forward queue. Sized for a keyframe burst at
/// near-MTU packets (a 250 KB IDR is ~200 packets); beyond that we drop
/// newest — the PLI path recovers the picture, and blocking the sender's
/// recv_loop is never acceptable.
pub const RELAY_QUEUE_DEPTH: usize = 256;

/// Most NACK requests queued for a leg's send task before new ones are
/// dropped — a receiver re-NACKs, and escalates to a PLI, on its own.
pub const NACK_QUEUE_DEPTH: usize = 64;

/// Most FCI entries relayed from one inbound NACK compound. An entry asks
/// for up to 17 packets and a compound may carry hundreds, so without a cap
/// one small datagram from a receiver makes the source resend thousands of
/// packets. Real receivers NACK a handful at a time.
pub const MAX_NACK_FCI: usize = 32;

/// A sequence number already requested of a source within this window is
/// not requested again — N receivers that lost the same packet (or one that
/// re-NACKs quickly) cost the source one retransmission, not N. Roughly an
/// RTT: a genuine re-request after the retransmission itself was lost still
/// goes through.
pub const NACK_DEDUPE_WINDOW: Duration = Duration::from_millis(100);

/// Sequence numbers one leg's remote may NACK per second (a token bucket,
/// one second deep), all its streams together. Inbound RTCP is not held to
/// the leg's `RelayLimits`, and each compound a receiver sends costs the
/// source a NACK and up to 17 retransmissions per entry — so a receiver
/// cycling through sequence numbers (which also defeats `NackLimiter`, whose
/// window only catches repeats) could keep every source it watches resending
/// flat out. An honest receiver losing a fifth of a 1000 pps stream, and
/// asking twice for each, stays under this.
pub const NACK_SEQ_RATE: f64 = 500.0;

/// Depth of that bucket: two compounds of `MAX_NACK_FCI` full entries, so a
/// burst of loss is asked for at once.
pub const NACK_SEQ_BURST: f64 = (MAX_NACK_FCI * 17 * 2) as f64;

/// Minimum spacing between keyframe requests sent to our remote. Browsers
/// re-request on their own cadence; anything the far end sends faster than
/// this collapses into one PLI.
pub const PLI_MIN_INTERVAL: Duration = Duration::from_millis(300);

/// One forwarded packet on its way to a leg's send task, tagged with the
/// source leg so the send task can keep per-source SSRC / sequence state,
/// and with the source leg's payload types so it can map the packet's PT.
pub struct RelayFrame {
    pub src: u64,
    pub src_pts: Arc<PtMap>,
    pub pkt: RtpPacket,
}

/// Most named codecs (`remote.codecs` entries) a leg keeps.
pub const MAX_NAMED_CODECS: usize = 16;

/// The payload types a relay leg negotiated with its remote — the PTs its
/// remote sends, and the PTs it expects to receive, for each codec. Set by
/// `openchannel({ relay: true, remote })` and replaced by every `remote()`:
///
/// * `remote.codec: pt` — the leg's *primary* codec (0 or absent: none).
///   Today's callers negotiate exactly one video codec per leg and pass just
///   this; the primary PT of one leg maps to the primary PT of another.
/// * `remote.codecs: { <label>: pt, ... }` — optional, for a leg that
///   negotiated several codecs. The label names the codec configuration
///   (e.g. "vp8", or "h264/42e01f" to tell profiles apart); the signalling
///   layer must use the same label on every leg for the same codec. A packet
///   is forwarded under the receiving leg's PT for the codec it was sent
///   under on the source leg (labels compare case-insensitively).
///
/// A packet whose PT the source leg does not declare, or whose codec the
/// receiving leg does not declare, is dropped and counted
/// (`out.ptdropped`) — never re-stamped into a codec the receiver would
/// decode it as. That is what keeps an RTX / FEC / second-codec stream (we
/// negotiate none) off the main stream's PT. A receiving leg that declares
/// no codec at all takes packets under their source PT unchanged, as before.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PtMap {
    primary: Option<u8>,
    /// `(label lowercased, pt)`.
    named: Vec<(String, u8)>,
}

impl PtMap {
    /// `primary` as `remote.codec` (0 = none); `named` as `remote.codecs`.
    /// Entries whose PT is not a valid dynamic-or-static RTP PT (1..=127)
    /// are ignored, as are those past `MAX_NAMED_CODECS`.
    pub fn new(primary: u32, named: impl IntoIterator<Item = (String, u32)>) -> Self {
        let valid = |pt: u32| (1..=127).contains(&pt).then_some(pt as u8);
        let mut map = Self {
            primary: valid(primary),
            named: Vec::new(),
        };
        for (label, pt) in named {
            if map.named.len() >= MAX_NAMED_CODECS {
                break;
            }
            let label = label.trim().to_ascii_lowercase();
            if let (Some(pt), false) = (valid(pt), label.is_empty()) {
                if !map.named.iter().any(|(l, _)| *l == label) {
                    map.named.push((label, pt));
                }
            }
        }
        map
    }

    /// Declares no codec at all.
    pub fn is_empty(&self) -> bool {
        self.primary.is_none() && self.named.is_empty()
    }

    /// Does the leg negotiate `pt` (or declare nothing, so any PT goes)?
    pub fn declares(&self, pt: u8) -> bool {
        self.is_empty() || self.primary == Some(pt) || self.named.iter().any(|(_, p)| *p == pt)
    }

    /// The payload format `pt` is negotiated as on this leg, from the label
    /// `remote.codecs` gives it: "vp8" or "h264" (optionally followed by
    /// "/<profile>" and anything else). `None` when the PT has no label, a
    /// label naming another codec, or labels naming two different formats —
    /// the relay's resolution limit never guesses (see relay_limits.rs).
    pub fn format_of(&self, pt: u8) -> Option<VideoFormat> {
        let mut found = None;
        for (label, _) in self.named.iter().filter(|(_, p)| *p == pt) {
            let fmt = match label.split('/').next().map(str::trim) {
                Some("vp8") => VideoFormat::Vp8,
                Some("h264") => VideoFormat::H264,
                _ => return None,
            };
            if found.is_some_and(|f| f != fmt) {
                return None;
            }
            found = Some(fmt);
        }
        found
    }

    /// The PT a packet sent under `pt` on a leg with map `src` goes out
    /// under on this leg — `None` to drop it (see the type's comment).
    pub fn map_from(&self, src: &PtMap, pt: u8) -> Option<u8> {
        if self.is_empty() {
            return Some(pt);
        }
        if src.primary == Some(pt) {
            if let Some(p) = self.primary {
                return Some(p);
            }
        }
        src.named
            .iter()
            .filter(|(_, p)| *p == pt)
            .find_map(|(label, _)| self.named.iter().find(|(l, _)| l == label))
            .map(|(_, p)| *p)
    }
}

/// A generic NACK (RFC 4585 §6.2.1) addressed to a source: the media SSRC
/// and FCI entries `(pid, blp)` already translated into that source's own
/// SSRC and sequence series.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NackReq {
    pub media_ssrc: u32,
    pub fci: Vec<(u16, u16)>,
}

/// How one outbound stream of a leg maps back to its source (one source
/// leg's SSRC — a source may have several): published by
/// the send task (`Outbound`) so a NACK or keyframe request naming `ssrc`
/// can be translated into the source's terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutRoute {
    /// The SSRC this leg sends the stream under.
    pub ssrc: u32,
    /// The source leg's id.
    pub src: u64,
    /// The source's own SSRC for the stream.
    pub src_ssrc: u32,
    /// `out_seq = src_seq + offset` (wrapping).
    pub offset: u16,
    /// First outbound sequence number of the stream when it continued an
    /// earlier series under `ssrc` — numbers before it belong to a stream
    /// that no longer exists and are not translated.
    pub base: Option<u16>,
    /// The payload type the stream last went out under.
    pub pt: Option<u8>,
}

/// One outbound stream of a leg, as `livestats().streams` reports it: what a
/// signalling layer needs to announce the stream to the receiving endpoint
/// (`a=ssrc` / `msid` per source once a group has more than one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamInfo {
    /// The source leg's label (its JS channel `uuid`).
    pub source: String,
    /// The SSRC this leg forwards the stream under.
    pub ssrc: u32,
    /// The source's own SSRC for it.
    pub source_ssrc: u32,
    /// The payload type it last went out under.
    pub pt: Option<u8>,
}

/// The members of one relay group — see the module comment.
pub type RelayGroup = Arc<PLMutex<Vec<Arc<RelayShared>>>>;

/// The shared face of a relay channel — held by the ChannelObject (for
/// `mix`/`unmix` grouping and live stats), by the actor (close stats and
/// leaving the group on close), by the channel's own recv_loop (inbound
/// counting, fan-out) and, through the group, by every other member's
/// recv_loop (to feed our send task and request keyframes from our remote).
pub struct RelayShared {
    /// The channel id — identifies this leg as a source in its peers'
    /// per-source state and orders lock acquisition in `join`.
    pub id: u64,
    /// How this leg is named to JS as a source (`livestats().streams`): the
    /// channel's `uuid` from index.js; the id when none was given.
    label: String,
    /// Feed this channel's OUTBOUND send task — every other member's
    /// recv_loop pushes each decrypted inbound packet here.
    pub data_tx: mpsc::Sender<RelayFrame>,
    /// Ask this channel's send task to send a PLI to *its* remote (i.e.
    /// request a keyframe from the source this channel receives).
    pub pli: Notify,
    /// The group this leg belongs to; `None` when unmixed.
    group: PLMutex<Option<RelayGroup>>,
    /// Source legs that have left this leg's group since the send task last
    /// looked — it frees their outbound SSRCs for reuse.
    retired: PLMutex<Vec<u64>>,
    /// Set whenever `retired` gains an entry, so the send task's per-packet
    /// path can reap departed sources promptly without taking the lock.
    retire_pending: AtomicBool,
    /// Outbound SSRC → source mapping, published by the send task, so a
    /// keyframe request or NACK naming one of our SSRCs reaches the right
    /// source (and a NACK's sequence numbers can be translated).
    out_routes: PLMutex<Vec<OutRoute>>,
    /// NACKs for OUR remote (the source we receive), translated by the
    /// receiving legs; drained and sent by this leg's send task.
    nacks: PLMutex<Vec<NackReq>>,
    /// Recently requested `(source ssrc, seq)` pairs — see `NackLimiter`.
    nack_limit: PLMutex<NackLimiter>,
    /// What OUR remote may still NACK — see `NACK_SEQ_RATE`.
    nack_budget: PLMutex<TokenBucket>,
    pub nack_wake: Notify,
    /// Inbound RTP datagrams seen by recv_loop, counted BEFORE decryption.
    /// Duplicated from `ChannelState.in_count` so the JS-facing live stats
    /// getter needs no access to actor-owned state. Raw arrival only — it
    /// moves for a leg whose SRTP never keys, so it is NOT the liveness
    /// signal; see `liveness()`.
    pub in_count: AtomicU64,
    /// Inbound RTP accepted for forwarding: decrypted and authenticated on
    /// a secure leg, or simply well-formed on a clear one. Counted before
    /// the abuse limits (`rate_dropped` / `frame_dropped` /
    /// `oversize_dropped` are subsets of it): a remote being policed is
    /// still a live remote.
    pub accepted: AtomicU64,
    /// Inbound RTCP (muxed on the RTP port) accepted the same way. Counts
    /// towards liveness: a browser that turns its camera off stops sending
    /// video RTP but keeps sending RTCP on the m-line.
    pub rtcp_in: AtomicU64,
    /// Secure leg, keys present, but SRTP/SRTCP auth or decrypt failed (bad
    /// MAC, replay, wrong keys) — dropped.
    pub decrypt_failed: AtomicU64,
    /// RTP/RTCP that arrived before it could be authenticated — on a secure
    /// leg before keying material existed, on any leg before it was given a
    /// remote (see `configured`) — dropped (fail closed). A leg stuck here
    /// never completed DTLS, or never had `remote()` called.
    pub prekey_dropped: AtomicU64,
    /// Packets actually forwarded out of this leg by the send task.
    pub out_count: AtomicU64,
    /// The payload types this leg negotiated (see `PtMap`): what its remote
    /// sends us, and what a forwarded packet's PT is mapped to on the way to
    /// it. Replaced whole by `remote()`; read per packet (an `Arc` clone).
    pts: PLMutex<Arc<PtMap>>,
    /// Forwarded packets dropped because their PT has no mapping to a codec
    /// this leg negotiated.
    pub pt_dropped: AtomicU64,
    /// True when this leg negotiated DTLS-SRTP — media is withheld until
    /// keying material is published rather than ever sent in the clear
    /// (same fail-closed rule as `ChannelState::secure_not_ready`).
    pub secure: AtomicBool,
    /// Packets dropped because the forward queue was full (keyframe burst
    /// overrunning the peer) — surfaced in stats, never silent.
    pub dropped: AtomicU64,
    /// The abuse limits inbound RTP from our remote is held to (see
    /// relay_limits.rs), as effective. Fixed at open.
    limits: RelayLimits,
    /// Authenticated inbound RTP not forwarded because it was over the
    /// leg's bitrate or packet rate.
    pub rate_dropped: AtomicU64,
    /// ...because its frame was over its stream's frame rate.
    pub frame_dropped: AtomicU64,
    /// ...because its stream's latest keyframe / SPS was over max-fs (or
    /// unreadable).
    pub oversize_dropped: AtomicU64,
    /// NACKed sequence numbers from our remote not passed on to their source
    /// because the remote was over `NACK_SEQ_RATE`.
    pub nack_dropped: AtomicU64,
    /// False until the leg has been given a remote (at open, or by the first
    /// `remote()`). Only then is it known whether the leg is DTLS-SRTP, so
    /// until then it neither accepts nor sends media or feedback and its
    /// remote address moves only on a signed STUN request: a leg opened bare
    /// and mixed before `remote({ dtls })` would otherwise latch onto the
    /// first datagram to reach its port and send that address the group's
    /// media in the clear.
    configured: AtomicBool,
    /// Set when the channel closes — a closed leg can not be mixed again
    /// (see `join`).
    closed: AtomicBool,
}

impl RelayShared {
    /// `primary_pt`: the leg's primary codec PT (`remote.codec`, 0 = none);
    /// `set_pts` adds named codecs.
    pub fn new(id: u64, data_tx: mpsc::Sender<RelayFrame>, secure: bool, primary_pt: u32) -> Self {
        Self {
            id,
            label: id.to_string(),
            data_tx,
            pli: Notify::new(),
            group: PLMutex::new(None),
            retired: PLMutex::new(Vec::new()),
            retire_pending: AtomicBool::new(false),
            out_routes: PLMutex::new(Vec::new()),
            nacks: PLMutex::new(Vec::new()),
            nack_limit: PLMutex::new(NackLimiter::default()),
            nack_budget: PLMutex::new(TokenBucket::new(NACK_SEQ_RATE, NACK_SEQ_BURST)),
            nack_wake: Notify::new(),
            in_count: AtomicU64::new(0),
            accepted: AtomicU64::new(0),
            rtcp_in: AtomicU64::new(0),
            decrypt_failed: AtomicU64::new(0),
            prekey_dropped: AtomicU64::new(0),
            out_count: AtomicU64::new(0),
            pts: PLMutex::new(Arc::new(PtMap::new(primary_pt, []))),
            pt_dropped: AtomicU64::new(0),
            secure: AtomicBool::new(secure),
            dropped: AtomicU64::new(0),
            limits: RelayLimits::DEFAULT.effective(),
            rate_dropped: AtomicU64::new(0),
            frame_dropped: AtomicU64::new(0),
            oversize_dropped: AtomicU64::new(0),
            nack_dropped: AtomicU64::new(0),
            configured: AtomicBool::new(true),
            closed: AtomicBool::new(false),
        }
    }

    /// A leg opened without a remote: nothing is known about it until
    /// `set_configured` (see `configured`).
    pub fn awaiting_remote(self) -> Self {
        self.configured.store(false, Ordering::Release);
        self
    }

    /// The leg has been given its remote (see `configured`). Call after the
    /// `secure` flag is settled, so no packet sees a configured clear leg
    /// that is about to become a secure one.
    pub fn set_configured(&self) {
        self.configured.store(true, Ordering::Release);
    }

    /// Has the leg been given a remote yet? See `configured`.
    pub fn is_configured(&self) -> bool {
        self.configured.load(Ordering::Acquire)
    }

    /// The channel is closing: leave the group for good.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.leave_group();
    }

    /// Hold inbound RTP to `limits` instead of `RelayLimits::DEFAULT`.
    pub fn with_limits(mut self, limits: RelayLimits) -> Self {
        self.limits = limits.effective();
        self
    }

    /// The limits inbound RTP is held to, as effective.
    pub fn limits(&self) -> RelayLimits {
        self.limits
    }

    /// Count one packet the inbound guard refused.
    pub fn count_refused(&self, v: super::relay_limits::Verdict) {
        use super::relay_limits::Verdict;
        let c = match v {
            Verdict::Forward => return,
            Verdict::Rate => &self.rate_dropped,
            Verdict::Frame => &self.frame_dropped,
            Verdict::Oversize => &self.oversize_dropped,
        };
        c.fetch_add(1, Ordering::Relaxed);
    }

    /// Name this leg as a source (see `label`).
    pub fn with_label(mut self, label: String) -> Self {
        self.label = label;
        self
    }

    /// The streams this leg currently forwards: one per source SSRC, main
    /// stream first per source, in source-id order. Read-only — published
    /// by the send task as it maps streams, so a stream appears once its
    /// first packet has gone out. Streams of a source that has already left
    /// the group (awaiting reaping by the send task) are left out.
    pub fn streams(&self) -> Vec<StreamInfo> {
        let routes = self.out_routes.lock().clone();
        let Some(group) = self.group.lock().clone() else {
            return Vec::new();
        };
        let members = group.lock();
        routes
            .iter()
            .filter_map(|r| {
                let m = members.iter().find(|m| m.id == r.src && m.id != self.id)?;
                Some(StreamInfo {
                    source: m.label.clone(),
                    ssrc: r.ssrc,
                    source_ssrc: r.src_ssrc,
                    pt: r.pt,
                })
            })
            .collect()
    }

    /// Replace the leg's payload types (a new `remote()`).
    pub fn set_pts(&self, pts: PtMap) {
        *self.pts.lock() = Arc::new(pts);
    }

    /// The leg's current payload types.
    pub fn pts(&self) -> Arc<PtMap> {
        self.pts.lock().clone()
    }

    /// The idle detector's "media flowing" signal: packets from our remote
    /// that we could actually authenticate — accepted RTP plus accepted RTCP.
    /// Raw arrivals (`in_count`), pre-key drops and decrypt failures are
    /// deliberately excluded, so a leg whose DTLS never completes (or whose
    /// keys are wrong) idle-times-out instead of looking healthy forever,
    /// while a camera-off leg still sending RTCP stays up.
    pub fn liveness(&self) -> u64 {
        self.accepted.load(Ordering::Relaxed) + self.rtcp_in.load(Ordering::Relaxed)
    }

    /// Hand one accepted inbound packet to every other member of our group.
    /// Never blocks: a full queue is a counted drop on that receiving leg
    /// (and a sequence gap its browser can see).
    pub fn fan_out(&self, pkt: RtpPacket) {
        let Some(group) = self.group.lock().clone() else {
            return;
        };
        self.fan_out_to(&group, pkt);
    }

    /// The locked half of `fan_out`. The group handle was cloned without the
    /// member lock, so we may have left since: check membership under the
    /// lock `leave_group` takes, or a packet racing the leave reaches the
    /// remaining members after they were told we are gone.
    fn fan_out_to(&self, group: &RelayGroup, pkt: RtpPacket) {
        let members = group.lock();
        if !members.iter().any(|m| m.id == self.id) {
            return;
        }
        let mut peers = members.iter().filter(|m| m.id != self.id).peekable();
        let src_pts = self.pts();
        let mut pkt = Some(pkt);
        while let Some(peer) = peers.next() {
            // The last receiver takes the packet itself; the others a copy.
            let out = if peers.peek().is_some() {
                copy_packet(pkt.as_ref().expect("taken only on the last peer"))
            } else {
                pkt.take().expect("taken only on the last peer")
            };
            let frame = RelayFrame {
                src: self.id,
                src_pts: src_pts.clone(),
                pkt: out,
            };
            if peer.data_tx.try_send(frame).is_err() {
                peer.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Our remote asked for a keyframe about `media_ssrcs` (SSRCs WE send
    /// it). Ask each named source leg's send task to PLI its own remote. If
    /// none of the SSRCs is one we can map (unknown, zero, stale), every
    /// source in the group is asked — over-asking costs a keyframe, missing
    /// one costs a frozen picture. In a 2-leg group that is the one peer.
    pub fn request_keyframe(&self, media_ssrcs: &[u32]) {
        let sources: Vec<u64> = {
            let map = self.out_routes.lock();
            media_ssrcs
                .iter()
                .filter_map(|m| map.iter().find(|r| r.ssrc == *m).map(|r| r.src))
                .collect()
        };
        let Some(group) = self.group.lock().clone() else {
            return;
        };
        for peer in group.lock().iter().filter(|m| m.id != self.id) {
            if sources.is_empty() || sources.contains(&peer.id) {
                peer.pli.notify_one();
            }
        }
    }

    /// Our remote NACKed packets of streams WE send it: `(media_ssrc, fci)`.
    /// Translate each into the source's SSRC and sequence series and queue it
    /// on that source leg's send task, which NACKs its own remote — so the
    /// source retransmits. A NACK naming an SSRC we cannot map (unknown,
    /// stale, nothing forwarded yet) is dropped: unlike a keyframe request
    /// there is no meaningful "ask everyone" fallback, and the receiver
    /// escalates to a PLI itself if retransmission never comes.
    pub fn relay_nacks(&self, items: &[(u32, Vec<(u16, u16)>)]) {
        let mut per_src: Vec<(u64, NackReq)> = Vec::new();
        {
            let routes = self.out_routes.lock();
            let mut budget = MAX_NACK_FCI;
            for (media, fci) in items {
                if budget == 0 {
                    break;
                }
                let Some(r) = routes.iter().find(|r| r.ssrc == *media) else {
                    continue;
                };
                let fci: Vec<(u16, u16)> = fci
                    .iter()
                    .filter_map(|&(pid, blp)| match r.base {
                        Some(b) => clip_nack(pid, blp, b),
                        None => Some((pid, blp)),
                    })
                    .map(|(pid, blp)| (pid.wrapping_sub(r.offset), blp))
                    .take(budget)
                    .collect();
                budget -= fci.len();
                if !fci.is_empty() {
                    per_src.push((
                        r.src,
                        NackReq {
                            media_ssrc: r.src_ssrc,
                            fci,
                        },
                    ));
                }
            }
        }
        // Hold our remote to its NACK budget: whole entries while they fit.
        {
            let mut budget = self.nack_budget.lock();
            let now = Instant::now();
            for (_, req) in &mut per_src {
                req.fci.retain(|&(_, blp)| {
                    let seqs = 1 + blp.count_ones();
                    let ok = budget.admit(f64::from(seqs), now);
                    if !ok {
                        self.nack_dropped
                            .fetch_add(u64::from(seqs), Ordering::Relaxed);
                    }
                    ok
                });
            }
        }
        per_src.retain(|(_, req)| !req.fci.is_empty());
        if per_src.is_empty() {
            return;
        }
        let Some(group) = self.group.lock().clone() else {
            return;
        };
        let members = group.lock();
        for (src, req) in per_src {
            if src == self.id {
                continue;
            }
            if let Some(peer) = members.iter().find(|m| m.id == src) {
                let Some(req) = peer.nack_limit.lock().admit(&req, Instant::now()) else {
                    continue; // everything asked was asked moments ago
                };
                let mut q = peer.nacks.lock();
                if q.len() < NACK_QUEUE_DEPTH {
                    q.push(req);
                    drop(q);
                    peer.nack_wake.notify_one();
                }
            }
        }
    }

    /// React to one decrypted inbound RTCP compound from our remote:
    /// keyframe requests (PLI / FIR) and NACKs, each routed to its source.
    pub fn on_feedback(&self, fb: &Feedback) {
        if let Some(ssrcs) = &fb.keyframe {
            self.request_keyframe(ssrcs);
        }
        if !fb.nacks.is_empty() {
            self.relay_nacks(&fb.nacks);
        }
    }

    /// Leave our group (unmix, or close): nobody forwards to us, we forward
    /// to nobody, and every remaining member is told we are gone as a source
    /// so its send task can release the SSRC it used for us.
    pub fn leave_group(&self) {
        let Some(group) = self.group.lock().take() else {
            return;
        };
        let mut members = group.lock();
        members.retain(|m| m.id != self.id);
        for m in members.iter() {
            m.retired.lock().push(self.id);
            m.retire_pending.store(true, Ordering::Release);
        }
        self.retired.lock().extend(members.iter().map(|m| m.id));
        self.retire_pending.store(true, Ordering::Release);
    }

    /// Is `src` currently a member of our group? A frame can only have been
    /// queued while its source was a member (`fan_out_to` checks under the
    /// member lock), so a source that is not one now has left since.
    fn has_member(&self, src: u64) -> bool {
        let Some(group) = self.group.lock().clone() else {
            return false;
        };
        let members = group.lock();
        members.iter().any(|m| m.id == src)
    }

    /// Point-in-time copy of every counter, for live and close stats.
    pub fn snapshot(&self) -> RelayCounters {
        RelayCounters {
            in_count: self.in_count.load(Ordering::Relaxed),
            accepted: self.accepted.load(Ordering::Relaxed),
            rtcp_in: self.rtcp_in.load(Ordering::Relaxed),
            decrypt_failed: self.decrypt_failed.load(Ordering::Relaxed),
            prekey_dropped: self.prekey_dropped.load(Ordering::Relaxed),
            out_count: self.out_count.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            pt_dropped: self.pt_dropped.load(Ordering::Relaxed),
            rate_dropped: self.rate_dropped.load(Ordering::Relaxed),
            frame_dropped: self.frame_dropped.load(Ordering::Relaxed),
            oversize_dropped: self.oversize_dropped.load(Ordering::Relaxed),
            nack_dropped: self.nack_dropped.load(Ordering::Relaxed),
            limits: self.limits,
        }
    }
}

/// Drops NACK requests for sequence numbers a source was asked for within
/// `NACK_DEDUPE_WINDOW`, per source SSRC. Held by the SOURCE leg, so every
/// receiver's NACKs for it share one view.
#[derive(Default)]
pub struct NackLimiter {
    recent: HashMap<(u32, u16), Instant>,
}

/// Bound on remembered requests; past it, stale ones are purged (and, if
/// still over, all of them — forgetting only costs a duplicate request).
const NACK_LIMITER_MAX: usize = 4096;

impl NackLimiter {
    /// The part of `req` not requested within the window, re-packed into
    /// FCI entries (and recorded as requested now); `None` if nothing is left.
    pub fn admit(&mut self, req: &NackReq, now: Instant) -> Option<NackReq> {
        if self.recent.len() > NACK_LIMITER_MAX {
            self.recent
                .retain(|_, t| now.duration_since(*t) < NACK_DEDUPE_WINDOW);
            if self.recent.len() > NACK_LIMITER_MAX {
                self.recent.clear();
            }
        }
        let mut fci: Vec<(u16, u16)> = Vec::new();
        for &(pid, blp) in &req.fci {
            let seqs = std::iter::once(pid).chain(
                (0..16u16)
                    .filter(|i| blp & (1 << i) != 0)
                    .map(|i| pid.wrapping_add(i + 1)),
            );
            for seq in seqs {
                let key = (req.media_ssrc, seq);
                if let Some(t) = self.recent.get(&key) {
                    if now.duration_since(*t) < NACK_DEDUPE_WINDOW {
                        continue;
                    }
                }
                self.recent.insert(key, now);
                match fci.last_mut() {
                    Some((p, b)) if seq_newer(seq, *p) && seq.wrapping_sub(*p) <= 16 => {
                        *b |= 1 << (seq.wrapping_sub(*p) - 1);
                    }
                    _ => fci.push((seq, 0)),
                }
            }
        }
        (!fci.is_empty()).then_some(NackReq {
            media_ssrc: req.media_ssrc,
            fci,
        })
    }
}

/// Snapshot of a relay leg's counters — see the fields of `RelayShared`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RelayCounters {
    pub in_count: u64,
    pub accepted: u64,
    pub rtcp_in: u64,
    pub decrypt_failed: u64,
    pub prekey_dropped: u64,
    pub out_count: u64,
    pub dropped: u64,
    pub pt_dropped: u64,
    pub rate_dropped: u64,
    pub frame_dropped: u64,
    pub oversize_dropped: u64,
    pub nack_dropped: u64,
    /// The effective limits (not a counter; carried so close stats can
    /// report them alongside the drops they explain).
    pub limits: RelayLimits,
}

/// `mix(a, b)` for relay legs — the audio mix's group rules: both
/// ungrouped → a new group of two; one grouped → the other joins it; same
/// group → no-op; different groups → false (merging is unsupported, as for
/// audio). A leg whose channel has closed is refused too: its send task is
/// gone, so it would sit in the group as a member nothing ever removes. On
/// success every member is asked for a keyframe, so the joiner gets a picture
/// from each existing source and they get one from it.
pub fn join(a: &Arc<RelayShared>, b: &Arc<RelayShared>) -> bool {
    if a.id == b.id {
        return true; // mix(self, self); locking one slot twice would deadlock
    }
    let (lo, hi) = if a.id < b.id { (a, b) } else { (b, a) };
    let members = {
        let mut lo_slot = lo.group.lock();
        let mut hi_slot = hi.group.lock();
        // Checked under the slots `close` takes to leave: a close racing
        // this either is refused here or leaves the group it was added to.
        if lo.closed.load(Ordering::Acquire) || hi.closed.load(Ordering::Acquire) {
            return false;
        }
        let group = match (lo_slot.as_ref(), hi_slot.as_ref()) {
            (None, None) => Arc::new(PLMutex::new(vec![lo.clone(), hi.clone()])),
            (Some(g), None) => {
                g.lock().push(hi.clone());
                g.clone()
            }
            (None, Some(g)) => {
                g.lock().push(lo.clone());
                g.clone()
            }
            (Some(x), Some(y)) if Arc::ptr_eq(x, y) => x.clone(),
            _ => return false,
        };
        *lo_slot = Some(group.clone());
        *hi_slot = Some(group.clone());
        let members = group.lock().clone();
        members
    };
    // A leg that left and rejoins must not be reaped as a retired source.
    for m in &members {
        m.retired
            .lock()
            .retain(|id| !members.iter().any(|o| o.id == *id));
    }
    for m in &members {
        m.pli.notify_one();
    }
    true
}

fn copy_packet(pkt: &RtpPacket) -> RtpPacket {
    let mut rp = RtpPacket::new();
    rp.as_mut_slice_for_fill(pkt.len())
        .copy_from_slice(pkt.as_slice());
    rp
}

pub struct RelaySendConfig {
    pub shared: Arc<RelayShared>,
    pub data_rx: mpsc::Receiver<RelayFrame>,
    pub sock: Arc<UdpSocket>,
    pub remote_addr: Arc<PLMutex<Option<std::net::SocketAddr>>>,
    /// This leg's own SSRC: the PLI's packet-sender field, and the SSRC of
    /// the first source forwarded (so a 2-leg relay shows the browser one
    /// SSRC per direction, the one signalled for the leg).
    pub ssrc: u32,
    /// The source we receive — a PLI we send is *about* this SSRC.
    pub rx_stats: Arc<PLMutex<RxStats>>,
    /// DTLS keying material, published by the actor tick's handshake poll.
    pub key_rx: watch::Receiver<Option<SrtpKeyingMaterial>>,
    pub cancel: CancellationToken,
}

pub fn spawn_send_task(cfg: RelaySendConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(send_task(cfg))
}

async fn send_task(mut cfg: RelaySendConfig) {
    // One Context serves both encrypt_rtp and encrypt_rtcp — SRTP ROC and
    // SRTCP index live together, which is exactly why this task owns all
    // outbound crypto for the leg.
    let mut encrypt: Option<webrtc_srtp::context::Context> = None;
    let mut out = Outbound::new(cfg.ssrc);
    // Rate limiting: `last_pli` stamps only *sent* PLIs — a request that
    // could not go out (no remote yet, no source latched) must not consume
    // the window. A request arriving inside the window is deferred to
    // `pli_deadline` rather than dropped, so the far end's ask still lands
    // once the window opens.
    let mut last_pli: Option<Instant> = None;
    let mut pli_deadline: Option<tokio::time::Instant> = None;
    loop {
        let deadline =
            pli_deadline.unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(3600));
        tokio::select! {
            biased;
            _ = cfg.cancel.cancelled() => break,
            _ = cfg.shared.pli.notified() => {
                match last_pli {
                    Some(t) if t.elapsed() < PLI_MIN_INTERVAL => {
                        pli_deadline.get_or_insert(
                            tokio::time::Instant::now() + (PLI_MIN_INTERVAL - t.elapsed()),
                        );
                    }
                    _ => {
                        if send_pli(&mut cfg, &mut encrypt).await {
                            last_pli = Some(Instant::now());
                        }
                    }
                }
            }
            _ = tokio::time::sleep_until(deadline), if pli_deadline.is_some() => {
                pli_deadline = None;
                if send_pli(&mut cfg, &mut encrypt).await {
                    last_pli = Some(Instant::now());
                }
            }
            _ = cfg.shared.nack_wake.notified() => {
                let reqs = std::mem::take(&mut *cfg.shared.nacks.lock());
                for req in reqs {
                    send_nack(&mut cfg, &mut encrypt, &req).await;
                }
            }
            pkt = cfg.data_rx.recv() => {
                let Some(mut frame) = pkt else { break };
                forward(&mut cfg, &mut encrypt, &mut out, &mut frame).await;
            }
        }
    }
}

/// Build the leg's SRTP/SRTCP encrypt context once keys are published.
/// No-op when already built or (for a non-secure leg) never needed.
fn maybe_build_encrypt(
    key_rx: &watch::Receiver<Option<SrtpKeyingMaterial>>,
    slot: &mut Option<webrtc_srtp::context::Context>,
) {
    if slot.is_some() {
        return;
    }
    if let Some(keys) = key_rx.borrow().as_ref() {
        let (key, salt, profile) = local_srtp_params(keys);
        if let Ok(ctx) = webrtc_srtp::context::Context::new(key, salt, profile, None, None) {
            *slot = Some(ctx);
        }
    }
}

async fn forward(
    cfg: &mut RelaySendConfig,
    encrypt: &mut Option<webrtc_srtp::context::Context>,
    out: &mut Outbound,
    frame: &mut RelayFrame,
) {
    let pkt = &mut frame.pkt;
    if !cfg.shared.is_configured() {
        return; // no remote() yet — not known whether this leg is secure
    }
    let Some(addr) = *cfg.remote_addr.lock() else {
        return; // remote not yet confirmed — nowhere to send
    };
    let secure = cfg.shared.secure.load(Ordering::Relaxed);
    if secure {
        maybe_build_encrypt(&cfg.key_rx, encrypt);
        if encrypt.is_none() {
            return; // fail closed: never leak media before the handshake
        }
    }

    // Map the payload type into this leg's PT for the same codec — before
    // anything else, so a packet we cannot label correctly (an RTX / FEC /
    // unnegotiated stream) is dropped rather than given outbound state.
    let Some(out_pt) = cfg
        .shared
        .pts()
        .map_from(&frame.src_pts, pkt.payload_type())
    else {
        cfg.shared.pt_dropped.fetch_add(1, Ordering::Relaxed);
        return;
    };

    // Re-originate the SSRC (this leg's, per source) and map the sequence
    // number through the source's offset — NOT an arrival-order counter: the
    // receiver's loss detection (NACK/PLI), reordering and frame assembly
    // all read gaps and order from the sequence number, so a source loss
    // or a queue drop here must still show as a gap. Timestamp, marker
    // bit, extension and payload bytes pass through untouched — the
    // timestamp is the source's 90 kHz media clock and one frame's
    // fragments must share it.
    let Some((out_ssrc, out_sn)) = out.stamp(
        &cfg.shared,
        frame.src,
        pkt.ssrc(),
        pkt.sequence_number(),
        out_pt,
        Instant::now(),
    ) else {
        return; // straggler from a source leg that has left
    };
    pkt.set_ssrc(out_ssrc);
    pkt.set_sequence_number(out_sn);
    pkt.set_payload_type(out_pt);

    let sent = if let Some(ctx) = encrypt.as_mut() {
        match ctx.encrypt_rtp(pkt.as_slice()) {
            Ok(enc) => cfg.sock.send_to(&enc, addr).await.is_ok(),
            Err(_) => false,
        }
    } else {
        cfg.sock.send_to(pkt.as_slice(), addr).await.is_ok()
    };
    if sent {
        cfg.shared.out_count.fetch_add(1, Ordering::Relaxed);
    }
}

/// Outbound sequence numbering for one forwarded stream (one source SSRC):
/// `out = src + offset` (wrapping). The offset is fixed on the stream's first
/// packet, so every gap and reordering the source (or our own forward queue)
/// produced reaches the receiver unchanged. A stream on a fresh outbound SSRC
/// passes through with a zero offset; one that takes over an outbound SSRC
/// already used (`continuing`) is rebased so it carries on at `highest + 1` —
/// the receiver's SRTP replay window and ROC see one monotonic series under
/// that SSRC, never a reused number.
///
/// A map serves exactly one source SSRC. It used to follow the source across
/// SSRC changes, rebasing each time and dropping the previous SSRC's
/// stragglers (`prev_ssrc`) so a stream restart could not flip the offset
/// back and forth. But any second SSRC from the same source — RTX, FEC, a
/// simulcast layer, one stray packet — looked like a restart, and the
/// sequence A, B, A then marked the real stream A as the previous one and
/// dropped it for the rest of the call. Now each source SSRC has a map (and
/// an outbound SSRC) of its own inside `Outbound`, and a restart is a
/// hand-over of the outbound identity there (see `MAIN_IDLE_TAKEOVER`).
#[derive(Default)]
pub struct SeqMap {
    started: bool,
    offset: u16,
    highest: Option<u16>,
    base: Option<u16>,
}

impl SeqMap {
    /// A map whose stream continues after `highest` — for handing an
    /// outbound SSRC's series on to a new stream without reusing numbers.
    pub fn continuing(highest: Option<u16>) -> Self {
        Self {
            highest,
            ..Self::default()
        }
    }

    /// Highest outbound sequence number issued so far (wrap-aware).
    pub fn highest(&self) -> Option<u16> {
        self.highest
    }

    /// The offset applied to the source's numbers and (when the stream
    /// continued an earlier series) the first outbound number it used — what
    /// a NACK needs to translate outbound numbers back to the source's.
    pub fn route(&self) -> (u16, Option<u16>) {
        (self.offset, self.base)
    }

    /// Map a source packet's sequence number to the outbound one.
    pub fn map(&mut self, src_seq: u16) -> u16 {
        if !self.started {
            self.started = true;
            self.offset = match self.highest {
                Some(h) => h.wrapping_add(1).wrapping_sub(src_seq),
                None => 0,
            };
            self.base = self.highest.map(|h| h.wrapping_add(1));
        }
        let out = src_seq.wrapping_add(self.offset);
        match self.highest {
            Some(h) if !seq_newer(out, h) => {}
            _ => self.highest = Some(out),
        }
        if let (Some(b), Some(h)) = (self.base, self.highest) {
            if h.wrapping_sub(b) >= BASE_RETIRE_SPAN {
                self.base = None; // see BASE_RETIRE_SPAN
            }
        }
        out
    }
}

/// Most source SSRCs tracked per source leg. A browser sends one per stream
/// (plus RTX / FEC / simulcast layers); anything past this is odd or
/// hostile — on a non-secure leg anyone can send any SSRC — so the least
/// recently used stream other than the main one gives up its slot.
pub const MAX_STREAMS_PER_SOURCE: usize = 8;

/// How long a source's main stream must be silent before another of its
/// SSRCs takes over the main stream's outbound identity (its SSRC and
/// sequence series). That is how a source that restarted under a new SSRC
/// (renegotiation, a new endpoint behind the leg) stays on the SSRC the
/// receiver was told about, while a second SSRC that runs *alongside* the
/// main stream (RTX, FEC, simulcast, a stray packet) never disturbs it. Until
/// the takeover a new SSRC is forwarded under an outbound SSRC of its own.
pub const MAIN_IDLE_TAKEOVER: Duration = Duration::from_secs(1);

/// A leg's outbound state for every source it forwards. Each source SSRC is
/// a stream of its own: a distinct outbound SSRC (a receiver demuxes streams
/// by SSRC) and a `SeqMap`. A source's first stream is its *main* stream and
/// gets the source's outbound identity — the leg's own SSRC for the first
/// source, so a 2-leg relay shows the browser the SSRC signalled for the
/// leg. When a source leaves the group, or a stream is evicted, its SSRC
/// (and the highest sequence number used under it) goes back to the pool,
/// preferring the leg's own SSRC on reuse — so a re-paired 2-leg relay keeps
/// showing the browser the same SSRC with a continuous sequence series.
pub struct Outbound {
    primary: u32,
    sources: HashMap<u64, SourceOut>,
    free: Vec<(u32, Option<u16>)>,
    rng: u32,
}

/// One source leg's streams; `streams[0]` is the main stream.
struct SourceOut {
    streams: Vec<StreamOut>,
}

struct StreamOut {
    /// The source's SSRC for this stream.
    src_ssrc: u32,
    /// The SSRC we forward it under.
    ssrc: u32,
    seq: SeqMap,
    /// The payload type it last went out under.
    pt: Option<u8>,
    /// When its last packet was forwarded.
    last: Instant,
}

impl Outbound {
    pub fn new(primary: u32) -> Self {
        Self {
            primary,
            sources: HashMap::new(),
            free: vec![(primary, None)],
            rng: primary ^ 0x9E37_79B9,
        }
    }

    /// Outbound `(ssrc, seq)` for a packet from `src` under its `src_ssrc`,
    /// going out under payload type `pt`, arriving `now` — or `None` to drop
    /// it (a straggler from a source leg that has since left). Publishes the
    /// stream map to `shared` whenever it changes.
    pub fn stamp(
        &mut self,
        shared: &RelayShared,
        src: u64,
        src_ssrc: u32,
        src_seq: u16,
        pt: u8,
        now: Instant,
    ) -> Option<(u32, u16)> {
        // Reap sources that have left as soon as we hear of it, not only
        // when a new source arrives: otherwise a departed source's SSRCs stay
        // in `out_routes` (and its state in `sources`) for the rest of the
        // leg's life. A new source also reaps first — the SSRCs of the ones
        // that left are what it should inherit.
        let mut gone = false;
        if shared.retire_pending.swap(false, Ordering::AcqRel) || !self.sources.contains_key(&src) {
            let retired = std::mem::take(&mut *shared.retired.lock());
            let mut changed = false;
            for id in retired {
                if let Some(s) = self.sources.remove(&id) {
                    self.free
                        .extend(s.streams.iter().map(|t| (t.ssrc, t.seq.highest())));
                    changed = true;
                }
                gone |= id == src;
            }
            if changed && self.sources.contains_key(&src) {
                self.publish(shared);
            }
        }
        if !self.sources.contains_key(&src) {
            // Only a current member may be given outbound state. `gone`
            // alone is not enough: with [a11, b21] queued after b left, a11
            // reaps b and empties `retired`, so b21 would find b neither in
            // `sources` nor in `retired` and be handed a fresh SSRC that
            // nothing would ever retire. Asking the group settles it
            // whatever order the reap and the stragglers land in.
            if gone || !shared.has_member(src) {
                self.publish(shared); // the reap above may have changed it
                return None;
            }
            self.sources.insert(
                src,
                SourceOut {
                    streams: Vec::new(),
                },
            );
        }

        let (i, changed) = self.stream_for(src, src_ssrc, now);
        let t = &mut self.sources.get_mut(&src)?.streams[i];
        t.last = now;
        let pt_changed = t.pt.replace(pt) != Some(pt);
        let before = t.seq.route();
        let out = (t.ssrc, t.seq.map(src_seq));
        if changed || pt_changed || t.seq.route() != before {
            // A new stream, a hand-over, or its first packet: NACK
            // translation must use the new mapping from now on (and
            // `livestats` shows the new stream / PT).
            self.publish(shared);
        }
        Some(out)
    }

    /// Index of `src_ssrc`'s stream within source `src` (which exists),
    /// creating, promoting or evicting streams as needed; and whether the
    /// stream map changed.
    fn stream_for(&mut self, src: u64, src_ssrc: u32, now: Instant) -> (usize, bool) {
        let s = &self.sources[&src];
        let found = s.streams.iter().position(|t| t.src_ssrc == src_ssrc);
        let main_idle = s
            .streams
            .first()
            .is_some_and(|m| now.saturating_duration_since(m.last) >= MAIN_IDLE_TAKEOVER);
        match found {
            Some(0) => (0, false),
            Some(i) if main_idle => {
                // A stream that already runs alongside takes over from a
                // main stream gone quiet: it moves to the main SSRC,
                // continuing its series, and frees its own.
                let s = self.sources.get_mut(&src).expect("source exists");
                let t = s.streams.remove(i);
                self.free.push((t.ssrc, t.seq.highest()));
                let m = &mut s.streams[0];
                m.seq = SeqMap::continuing(m.seq.highest());
                m.src_ssrc = t.src_ssrc;
                (0, true)
            }
            Some(i) => (i, false),
            None if s.streams.is_empty() => {
                let (ssrc, highest) = self.alloc();
                self.push(src, 0, src_ssrc, ssrc, SeqMap::continuing(highest), now);
                (0, true)
            }
            None if main_idle => {
                // The main stream has gone quiet and a new SSRC appears: a
                // restart. It inherits the main identity and series.
                let m = &mut self.sources.get_mut(&src).expect("source exists").streams[0];
                m.seq = SeqMap::continuing(m.seq.highest());
                m.src_ssrc = src_ssrc;
                (0, true)
            }
            None => {
                // A second SSRC alongside a live main stream: its own
                // outbound SSRC, within the per-source bound.
                if s.streams.len() >= MAX_STREAMS_PER_SOURCE {
                    let lru = (1..s.streams.len())
                        .min_by_key(|&i| s.streams[i].last)
                        .expect("bound is above one");
                    let t = self
                        .sources
                        .get_mut(&src)
                        .expect("source exists")
                        .streams
                        .remove(lru);
                    self.free.push((t.ssrc, t.seq.highest()));
                }
                let (ssrc, highest) = self.alloc();
                let at = self.sources[&src].streams.len();
                self.push(src, at, src_ssrc, ssrc, SeqMap::continuing(highest), now);
                (at, true)
            }
        }
    }

    fn push(&mut self, src: u64, at: usize, src_ssrc: u32, ssrc: u32, seq: SeqMap, now: Instant) {
        let s = self.sources.get_mut(&src).expect("source exists");
        s.streams.insert(
            at,
            StreamOut {
                src_ssrc,
                ssrc,
                seq,
                pt: None,
                last: now,
            },
        );
    }

    fn publish(&self, shared: &RelayShared) {
        let mut ids: Vec<&u64> = self.sources.keys().collect();
        ids.sort_unstable();
        *shared.out_routes.lock() = ids
            .into_iter()
            .flat_map(|id| {
                self.sources[id].streams.iter().map(move |t| {
                    let (offset, base) = t.seq.route();
                    OutRoute {
                        ssrc: t.ssrc,
                        src: *id,
                        src_ssrc: t.src_ssrc,
                        offset,
                        base,
                        pt: t.pt,
                    }
                })
            })
            .collect();
    }

    fn in_use(&self, ssrc: u32) -> bool {
        self.sources
            .values()
            .any(|s| s.streams.iter().any(|t| t.ssrc == ssrc))
            || self.free.iter().any(|(s, _)| *s == ssrc)
    }

    fn alloc(&mut self) -> (u32, Option<u16>) {
        if let Some(i) = self.free.iter().position(|(s, _)| *s == self.primary) {
            return self.free.swap_remove(i);
        }
        if let Some(f) = self.free.pop() {
            return f;
        }
        loop {
            // xorshift32 — uniqueness within the leg is what matters.
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 17;
            self.rng ^= self.rng << 5;
            let c = self.rng;
            if c != 0 && c != self.primary && !self.in_use(c) {
                return (c, None);
            }
        }
    }
}

/// Once a continued stream's highest outbound number is this far past its
/// `base`, `base` is dropped. It exists to stop NACKs for the *previous*
/// source's numbers being translated into the new source's series, and no
/// receiver NACKs a packet 16k sequence numbers old; keeping it longer is
/// actively harmful, because the comparison is wrap-relative and 32768
/// packets past `base` every current number would read as "before" it.
const BASE_RETIRE_SPAN: u16 = 0x4000;

/// Restrict one NACK FCI entry to sequence numbers at or after `base`. The
/// entry asks for `pid` and, per set BLP bit i, `pid + i + 1`. When `pid`
/// itself predates `base` but some BLP bits land at or after it, the entry
/// is re-anchored on the first of those rather than dropped whole. `None`
/// when nothing it asks for is at or after `base`.
fn clip_nack(pid: u16, blp: u16, base: u16) -> Option<(u16, u16)> {
    let at_or_after = |s: u16| !seq_newer(base, s);
    if at_or_after(pid) {
        return Some((pid, blp));
    }
    let mut wanted = (0..16u16)
        .filter(|i| blp & (1 << i) != 0)
        .map(|i| pid.wrapping_add(i + 1))
        .filter(|&s| at_or_after(s));
    let first = wanted.next()?;
    let rest = wanted.fold(0u16, |acc, s| acc | 1 << (s.wrapping_sub(first) - 1));
    Some((first, rest))
}

/// RFC 3550 A.1-style wrap-aware "a is after b".
fn seq_newer(a: u16, b: u16) -> bool {
    a != b && a.wrapping_sub(b) < 0x8000
}

/// RFC 4585 §6.3.1 Picture Loss Indication: PT 206 (PSFB), FMT 1, our SSRC
/// as packet sender, the source we receive as media source. Sent over the
/// RTP socket — relay legs are rtcp-mux in practice (WebRTC always is);
/// a split-port peer simply ignores RTCP arriving on its RTP port.
fn build_pli(sender_ssrc: u32, media_ssrc: u32) -> [u8; 12] {
    let mut p = [0u8; 12];
    p[0] = 0x81; // V=2, P=0, FMT=1
    p[1] = 206; // PSFB
    p[2..4].copy_from_slice(&2u16.to_be_bytes()); // length: 3 words - 1
    p[4..8].copy_from_slice(&sender_ssrc.to_be_bytes());
    p[8..12].copy_from_slice(&media_ssrc.to_be_bytes());
    p
}

/// RFC 4585 §6.2.1 generic NACK: PT 205 (RTPFB), FMT 1, our SSRC as packet
/// sender, the source's stream as media source, one `(pid, blp)` FCI word
/// per entry.
fn build_nack(sender_ssrc: u32, req: &NackReq) -> Vec<u8> {
    let mut p = Vec::with_capacity(12 + 4 * req.fci.len());
    p.push(0x81); // V=2, P=0, FMT=1
    p.push(205); // RTPFB
    p.extend_from_slice(&((2 + req.fci.len()) as u16).to_be_bytes());
    p.extend_from_slice(&sender_ssrc.to_be_bytes());
    p.extend_from_slice(&req.media_ssrc.to_be_bytes());
    for (pid, blp) in &req.fci {
        p.extend_from_slice(&pid.to_be_bytes());
        p.extend_from_slice(&blp.to_be_bytes());
    }
    p
}

/// Send one translated NACK to our remote (the source). No time-based rate
/// limit — a NACK costs the source a retransmission, not a keyframe — but by
/// the time it is queued here it has been capped (`MAX_NACK_FCI`) and
/// stripped of anything requested within `NACK_DEDUPE_WINDOW`.
async fn send_nack(
    cfg: &mut RelaySendConfig,
    encrypt: &mut Option<webrtc_srtp::context::Context>,
    req: &NackReq,
) -> bool {
    if !cfg.shared.is_configured() {
        return false;
    }
    let Some(addr) = *cfg.remote_addr.lock() else {
        return false;
    };
    // FCI entries beyond what fits a length field are simply not asked for.
    let mut req = req.clone();
    req.fci.truncate(255);
    let nack = build_nack(cfg.ssrc, &req);
    if cfg.shared.secure.load(Ordering::Relaxed) {
        maybe_build_encrypt(&cfg.key_rx, encrypt);
        let Some(ctx) = encrypt.as_mut() else {
            return false;
        };
        match ctx.encrypt_rtcp(&nack) {
            Ok(enc) => cfg.sock.send_to(&enc, addr).await.is_ok(),
            Err(_) => false,
        }
    } else {
        cfg.sock.send_to(&nack, addr).await.is_ok()
    }
}

/// Returns true only when a PLI actually went out — the caller's rate
/// limiter must not be charged for a request that had nowhere to go.
async fn send_pli(
    cfg: &mut RelaySendConfig,
    encrypt: &mut Option<webrtc_srtp::context::Context>,
) -> bool {
    if !cfg.shared.is_configured() {
        return false;
    }
    let Some(addr) = *cfg.remote_addr.lock() else {
        return false;
    };
    // The PLI is about the stream we *receive*; until a source is latched
    // there is nothing to request a keyframe from.
    let Some(media_ssrc) = cfg.rx_stats.lock().remote_ssrc else {
        return false;
    };
    let pli = build_pli(cfg.ssrc, media_ssrc);
    if cfg.shared.secure.load(Ordering::Relaxed) {
        maybe_build_encrypt(&cfg.key_rx, encrypt);
        let Some(ctx) = encrypt.as_mut() else {
            return false;
        };
        match ctx.encrypt_rtcp(&pli) {
            Ok(enc) => cfg.sock.send_to(&enc, addr).await.is_ok(),
            Err(_) => false,
        }
    } else {
        cfg.sock.send_to(&pli, addr).await.is_ok()
    }
}

/// What one decrypted inbound RTCP compound asks of the relay.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Feedback {
    /// `Some(ssrcs)`: a keyframe request (PLI 206/1 or FIR 206/4) naming the
    /// media SSRCs it is about — the media-source field of a PLI, the FCI
    /// entries of a FIR (whose own media-source field is unused, RFC 5104
    /// §4.3.1.2). May be empty or name SSRCs we do not know; the caller then
    /// falls back to asking every source.
    pub keyframe: Option<Vec<u32>>,
    /// Generic NACKs (205/1): `(media_ssrc, [(pid, blp)])`, in our outbound
    /// terms — `RelayShared::relay_nacks` translates them for the source.
    pub nacks: Vec<(u32, Vec<(u16, u16)>)>,
}

/// Classify a decrypted inbound RTCP compound. Keyframe requests and NACKs
/// are extracted; everything else — transport-cc (205/15), REMB (206/15),
/// unknown — is ignored, and deliberately so: congestion-control feedback
/// arrives continuously and must not be turned into a PLI storm. A NACK is
/// NOT a keyframe request (see the module comment).
pub fn parse_feedback(pkt: &[u8]) -> Feedback {
    // Walk the whole compound (RFC 3550 §6.1): a browser's PLI almost never
    // travels alone — without reduced-size RTCP (RFC 5506, which we do not
    // negotiate) it MUST ride behind a leading RR/SR, so classifying only
    // the first packet header misses every real keyframe request and the
    // receiving side waits forever for a picture it can decode.
    let ssrc_at = |b: &[u8], i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let u16_at = |b: &[u8], i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
    let mut fb = Feedback::default();
    let mut off = 0usize;
    while off + 8 <= pkt.len() {
        let p = &pkt[off..];
        if !(128..=191).contains(&p[0]) {
            break; // not RTCP — refuse to walk garbage
        }
        let fmt = p[0] & 0x1F;
        let words = u16::from_be_bytes([p[2], p[3]]) as usize;
        let this = &p[..((words + 1) * 4).min(p.len())];
        match p[1] {
            // PLI: the media source names the stream.
            206 if fmt == 1 => {
                let v = fb.keyframe.get_or_insert_with(Vec::new);
                if this.len() >= 12 {
                    v.push(ssrc_at(this, 8));
                }
            }
            // FIR: one 8-byte FCI entry (SSRC, seq nr, reserved) per stream.
            206 if fmt == 4 => {
                let v = fb.keyframe.get_or_insert_with(Vec::new);
                let mut i = 12;
                while i + 8 <= this.len() {
                    v.push(ssrc_at(this, i));
                    i += 8;
                }
            }
            // Generic NACK: media source, then one (PID, BLP) word per FCI.
            205 if fmt == 1 && this.len() >= 16 => {
                let mut fci = Vec::new();
                let mut i = 12;
                while i + 4 <= this.len() {
                    fci.push((u16_at(this, i), u16_at(this, i + 2)));
                    i += 4;
                }
                fb.nacks.push((ssrc_at(this, 8), fci));
            }
            _ => {}
        }
        off += (words + 1) * 4;
    }
    fb
}

/// The keyframe half of `parse_feedback`.
#[cfg(test)]
pub fn keyframe_targets(pkt: &[u8]) -> Option<Vec<u32>> {
    parse_feedback(pkt).keyframe
}

/// Does this compound ask for a keyframe at all? See `keyframe_targets`.
#[cfg(test)]
pub fn wants_keyframe(pkt: &[u8]) -> bool {
    keyframe_targets(pkt).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed test clock: `t(ms)` is `ms` after an arbitrary origin.
    fn t(ms: u64) -> Instant {
        static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *ORIGIN.get_or_init(Instant::now) + Duration::from_millis(ms)
    }

    fn leg(id: u64) -> (Arc<RelayShared>, mpsc::Receiver<RelayFrame>) {
        let (tx, rx) = mpsc::channel(8);
        (Arc::new(RelayShared::new(id, tx, false, 96)), rx)
    }

    fn rtp(sn: u16) -> RtpPacket {
        let mut p = RtpPacket::new();
        p.as_mut_slice_for_fill(16).copy_from_slice(&[
            0x80,
            96,
            (sn >> 8) as u8,
            sn as u8,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0x55,
            1,
            2,
            3,
            4,
        ]);
        p
    }

    /// Frames queued for a leg, as (source leg, sequence number).
    fn drain(rx: &mut mpsc::Receiver<RelayFrame>) -> Vec<(u64, u16)> {
        let mut v = Vec::new();
        while let Ok(f) = rx.try_recv() {
            v.push((f.src, f.pkt.sequence_number()));
        }
        v
    }

    fn route(ssrc: u32, src: u64) -> OutRoute {
        OutRoute {
            ssrc,
            src,
            src_ssrc: 0,
            offset: 0,
            base: None,
            pt: None,
        }
    }

    fn keyframe_asked(l: &RelayShared) -> bool {
        // notify_one stores a permit; poll it once without blocking.
        let n = l.pli.notified();
        tokio::pin!(n);
        n.as_mut().enable()
    }

    #[test]
    fn group_fans_out_to_every_other_member() {
        let ((a, mut ra), (b, mut rb), (c, mut rc)) = (leg(1), leg(2), leg(3));
        assert!(join(&a, &b));
        assert!(join(&a, &c)); // extends the group, like the audio mix
        a.fan_out(rtp(10));
        b.fan_out(rtp(20));
        assert_eq!(drain(&mut ra), vec![(2, 20)]);
        assert_eq!(drain(&mut rb), vec![(1, 10)]);
        assert_eq!(drain(&mut rc), vec![(1, 10), (2, 20)]);
        // Same group again: fine. A leg in another group: refused.
        assert!(join(&c, &b));
        let ((d, _rd), (e, _re)) = (leg(4), leg(5));
        assert!(join(&d, &e));
        assert!(!join(&a, &d));
        assert!(join(&a, &a));
    }

    #[test]
    fn leaving_stops_forwarding_both_ways_and_retires_the_source() {
        let ((a, mut ra), (b, mut rb), (c, mut rc)) = (leg(1), leg(2), leg(3));
        assert!(join(&a, &b));
        assert!(join(&b, &c));
        c.leave_group();
        a.fan_out(rtp(1));
        c.fan_out(rtp(2));
        assert_eq!(drain(&mut rb), vec![(1, 1)]);
        assert!(drain(&mut rc).is_empty(), "forwarded into a leg that left");
        assert!(drain(&mut ra).is_empty(), "a leg that left still forwards");
        assert_eq!(*a.retired.lock(), vec![3]);
        assert_eq!(*b.retired.lock(), vec![3]);
        // The re-pairing case: mix(A,B), B unmixes, mix(A,C) — B's media
        // must not reach A (the old single peer slot kept B → A alive).
        let ((a, mut ra), (b, _rb), (c, _rc)) = (leg(1), leg(2), leg(3));
        assert!(join(&a, &b));
        b.leave_group();
        assert!(join(&a, &c));
        b.fan_out(rtp(7));
        c.fan_out(rtp(8));
        assert_eq!(drain(&mut ra), vec![(3, 8)]);
        // Rejoining clears the retirement so the rejoiner is not reaped.
        assert!(join(&a, &b));
        assert!(!a.retired.lock().contains(&2));
    }

    /// `fan_out` clones the group handle, then locks the member list. A
    /// leave landing between the two used to let one packet through to the
    /// remaining members after they were told the source was gone — which
    /// then created a per-source entry in their `Outbound` nothing reaped.
    #[test]
    fn fan_out_racing_a_leave_forwards_nothing() {
        let ((a, _ra), (b, mut rb), (c, mut rc)) = (leg(1), leg(2), leg(3));
        assert!(join(&a, &b));
        assert!(join(&a, &c));
        let group = a.group.lock().clone().unwrap(); // fan_out's clone ...
        a.leave_group(); // ... then the leave wins the member lock
        a.fan_out_to(&group, rtp(1));
        assert!(drain(&mut rb).is_empty(), "forwarded from a leg that left");
        assert!(drain(&mut rc).is_empty(), "forwarded from a leg that left");
    }

    /// A source that leaves is reaped from the per-source outbound state on
    /// the next packet forwarded, not only when some new source appears —
    /// otherwise its SSRC stays routed (and its state held) indefinitely.
    #[test]
    fn departed_source_is_reaped_without_a_new_source() {
        let ((me, _rm), (a, _ra), (b, _rb)) = (leg(9), leg(1), leg(2));
        assert!(join(&me, &a));
        assert!(join(&me, &b));
        let mut out = Outbound::new(0xAAAA);
        assert_eq!(out.stamp(&me, 1, 0x11, 10, 96, t(0)), Some((0xAAAA, 10)));
        let (s2, _) = out.stamp(&me, 2, 0x22, 20, 96, t(0)).unwrap();
        // A straggler from b queued before it left arrives after.
        b.leave_group();
        assert_eq!(out.stamp(&me, 2, 0x22, 21, 96, t(0)), None);
        assert_eq!(out.stamp(&me, 1, 0x11, 11, 96, t(0)), Some((0xAAAA, 11)));
        let routes: Vec<(u32, u64)> = me
            .out_routes
            .lock()
            .iter()
            .map(|r| (r.ssrc, r.src))
            .collect();
        assert_eq!(routes, vec![(0xAAAA, 1)]);
        assert_eq!(out.sources.len(), 1);
        assert!(out.free.iter().any(|&(s, _)| s == s2));
        // And without a straggler: a leaves, the next packet (from b, who
        // rejoined) reaps it.
        assert!(join(&me, &b));
        a.leave_group();
        out.stamp(&me, 2, 0x22, 22, 96, t(0));
        assert!(!me.out_routes.lock().iter().any(|r| r.src == 1));
        assert!(!out.sources.contains_key(&1));
    }

    fn routed_sources(me: &RelayShared) -> Vec<u64> {
        let mut v: Vec<u64> = me.out_routes.lock().iter().map(|r| r.src).collect();
        v.sort();
        v
    }

    /// Reviewer's interleaving: b leaves while [a11, b21] are queued at a
    /// receiver. a11 reaps b (emptying `retired`); b21 then found b in
    /// neither `sources` nor `retired` and was given a brand-new SSRC that
    /// nothing would ever retire.
    #[test]
    fn straggler_after_reap_gets_no_new_source_entry() {
        let ((me, _rm), (a, _ra), (b, _rb)) = (leg(9), leg(1), leg(2));
        assert!(join(&me, &a));
        assert!(join(&me, &b));
        let mut out = Outbound::new(0xAAAA);
        out.stamp(&me, 1, 0x11, 10, 96, t(0)).unwrap();
        out.stamp(&me, 2, 0x22, 20, 96, t(0)).unwrap();
        b.leave_group();
        assert_eq!(out.stamp(&me, 1, 0x11, 11, 96, t(0)), Some((0xAAAA, 11))); // a11 reaps b
        assert_eq!(
            out.stamp(&me, 2, 0x22, 21, 96, t(0)),
            None,
            "b21 re-created b"
        ); // b21
        assert_eq!(out.stamp(&me, 2, 0x22, 22, 96, t(0)), None);
        assert_eq!(routed_sources(&me), vec![1]);
        assert!(!out.sources.contains_key(&2));
    }

    /// Three sources at one receiver, two leaving with stragglers landing
    /// after other sources' packets reaped them — including a source that
    /// left before ANY of its packets reached the send task.
    #[test]
    fn stragglers_from_departed_sources_in_a_three_party_group() {
        let (me, _rm) = leg(9);
        let (a, b, c) = (leg(1).0, leg(2).0, leg(3).0);
        for l in [&a, &b, &c] {
            assert!(join(&me, l));
        }
        let mut out = Outbound::new(0xAAAA);
        out.stamp(&me, 1, 0x11, 10, 96, t(0)).unwrap();
        out.stamp(&me, 2, 0x22, 20, 96, t(0)).unwrap();
        // c's first packet is still queued when c leaves; b leaves too.
        c.leave_group();
        b.leave_group();
        out.stamp(&me, 1, 0x11, 11, 96, t(0)).unwrap(); // reaps b and c
        assert_eq!(
            out.stamp(&me, 3, 0x33, 30, 96, t(0)),
            None,
            "c created after leaving"
        );
        assert_eq!(out.stamp(&me, 2, 0x22, 21, 96, t(0)), None, "b re-created");
        out.stamp(&me, 1, 0x11, 12, 96, t(0)).unwrap();
        assert_eq!(routed_sources(&me), vec![1]);
        assert_eq!(out.sources.len(), 1);
        // A source that rejoins is forwarded again.
        assert!(join(&me, &b));
        assert!(out.stamp(&me, 2, 0x22, 40, 96, t(0)).is_some());
        assert_eq!(routed_sources(&me), vec![1, 2]);
    }

    #[test]
    fn outbound_ssrc_per_source_and_reuse_after_leave() {
        let (me, _rx) = leg(9);
        let legs: Vec<_> = (1..=5).map(|i| leg(i).0).collect();
        for l in &legs[..4] {
            assert!(join(&me, l));
        }
        let mut out = Outbound::new(0xAAAA);
        // First source: the leg's own SSRC, sequence passed through.
        assert_eq!(out.stamp(&me, 1, 0x11, 100, 96, t(0)), Some((0xAAAA, 100)));
        // Second source: a distinct SSRC of its own, own sequence space.
        let (s2, sn) = out.stamp(&me, 2, 0x22, 5000, 96, t(0)).unwrap();
        assert!(s2 != 0xAAAA && s2 != 0);
        assert_eq!(sn, 5000);
        assert_eq!(out.stamp(&me, 1, 0x11, 101, 96, t(0)), Some((0xAAAA, 101)));
        let mut map: Vec<(u32, u64)> = me
            .out_routes
            .lock()
            .iter()
            .map(|r| (r.ssrc, r.src))
            .collect();
        map.sort();
        let mut want = vec![(0xAAAA, 1), (s2, 2)];
        want.sort();
        assert_eq!(map, want);
        // Source 1 leaves; the next new source inherits the leg's own SSRC
        // and continues its series (no reuse, no jump back).
        legs[0].leave_group();
        assert_eq!(out.stamp(&me, 3, 0x33, 7, 96, t(0)), Some((0xAAAA, 102)));
        assert_eq!(out.stamp(&me, 3, 0x33, 9, 96, t(0)), Some((0xAAAA, 104)));
        assert!(!me.out_routes.lock().iter().any(|r| r.src == 1));
        // Source 2 leaves; a new source reaps it and inherits its SSRC.
        legs[1].leave_group();
        assert_eq!(out.stamp(&me, 4, 0x44, 1, 96, t(0)), Some((s2, 5001)));
        // A source that is not (or no longer) a member gets nothing.
        assert_eq!(out.stamp(&me, 5, 0x55, 1, 96, t(0)), None);
        assert_eq!(out.stamp(&me, 2, 0x22, 5001, 96, t(0)), None);
    }

    #[test]
    fn keyframe_request_routes_to_the_named_source() {
        let ((a, _ra), (b, _rb), (c, _rc)) = (leg(1), leg(2), leg(3));
        assert!(join(&a, &b));
        assert!(join(&a, &c));
        for l in [&a, &b, &c] {
            keyframe_asked(l); // clear the join-time requests
        }
        // A forwards b as 0xB0 and c as 0xC0.
        *a.out_routes.lock() = vec![route(0xB0, 2), route(0xC0, 3)];
        a.request_keyframe(&[0xC0]);
        assert!(keyframe_asked(&c));
        assert!(!keyframe_asked(&b));
        assert!(!keyframe_asked(&a));
        // Unknown SSRC: ask every source rather than leave a frozen picture.
        a.request_keyframe(&[0xDEAD]);
        assert!(keyframe_asked(&b) && keyframe_asked(&c));
        a.request_keyframe(&[]);
        assert!(keyframe_asked(&b) && keyframe_asked(&c));
    }

    #[test]
    fn keyframe_targets_name_the_media_ssrcs() {
        let rr = [0x80u8, 201, 0, 1, 0, 0, 0, 1];
        let mut compound = rr.to_vec();
        compound.extend_from_slice(&build_pli(0x1, 0xB0));
        assert_eq!(keyframe_targets(&compound), Some(vec![0xB0]));
        // FIR: the FCI entries name the streams (two here).
        let fir = [
            0x84u8, 206, 0, 6, 0, 0, 0, 1, 0, 0, 0, 0, // header, media ssrc unused
            0, 0, 0, 0xB0, 1, 0, 0, 0, // FCI 1
            0, 0, 0, 0xC0, 1, 0, 0, 0, // FCI 2
        ];
        assert_eq!(keyframe_targets(&fir), Some(vec![0xB0, 0xC0]));
        // A NACK is not a keyframe request (it is passed through instead).
        assert_eq!(
            keyframe_targets(&[0x81, 205, 0, 3, 0, 0, 0, 1, 0, 0, 0, 0xC0, 0, 1, 0, 0]),
            None
        );
        assert_eq!(keyframe_targets(&rr), None);
    }

    /// Finding: a NACK used to be converted into a PLI, so ordinary loss
    /// (which the relay now passes through as sequence gaps) asked the source
    /// for a keyframe as often as the PLI rate limit allowed. A NACK must
    /// instead reach the SOURCE leg's send queue, translated into the
    /// source's SSRC and sequence series, and ask no keyframe of anyone.
    #[test]
    fn nack_passes_through_to_the_source_translated() {
        let ((a, _ra), (b, _rb), (c, _rc)) = (leg(1), leg(2), leg(3));
        assert!(join(&a, &b));
        assert!(join(&a, &c));
        for l in [&a, &b, &c] {
            keyframe_asked(l); // clear the join-time requests
        }
        // A forwards b and c. b's stream restarted under a new SSRC, so its
        // sequence numbers are offset on the way out.
        let mut out = Outbound::new(0xAAAA);
        assert_eq!(out.stamp(&a, 2, 0xB1, 100, 96, t(0)), Some((0xAAAA, 100)));
        assert_eq!(
            out.stamp(&a, 2, 0xB2, 7000, 96, t(1000)),
            Some((0xAAAA, 101))
        );
        let (c_out, c_sn) = out.stamp(&a, 3, 0xC1, 40, 96, t(0)).unwrap();

        // The receiver NACKs 103 (+104, 106 via BLP) of b's stream and
        // 41 of c's, in one compound behind an RR, as a browser sends it.
        let mut compound = vec![0x80u8, 201, 0, 1, 0, 0, 0, 9];
        compound.extend_from_slice(&build_nack(
            9,
            &NackReq {
                media_ssrc: 0xAAAA,
                fci: vec![(103, 0b101), (100, 0)], // 100 predates the restart
            },
        ));
        compound.extend_from_slice(&build_nack(
            9,
            &NackReq {
                media_ssrc: c_out,
                fci: vec![(c_sn.wrapping_add(1), 0)],
            },
        ));
        let fb = parse_feedback(&compound);
        assert_eq!(
            fb.keyframe, None,
            "a NACK must not become a keyframe request"
        );
        a.on_feedback(&fb);

        assert_eq!(
            *b.nacks.lock(),
            vec![NackReq {
                media_ssrc: 0xB2,
                fci: vec![(7002, 0b101)]
            }]
        );
        assert_eq!(
            *c.nacks.lock(),
            vec![NackReq {
                media_ssrc: 0xC1,
                fci: vec![(41, 0)]
            }]
        );
        assert!(a.nacks.lock().is_empty());
        for l in [&a, &b, &c] {
            assert!(!keyframe_asked(l), "NACK asked leg {} for a keyframe", l.id);
        }
        // An SSRC we never sent is dropped, not broadcast.
        a.relay_nacks(&[(0xDEAD, vec![(1, 0)])]);
        assert_eq!(b.nacks.lock().len(), 1);
        assert_eq!(c.nacks.lock().len(), 1);
    }

    /// Reviewer's probe: `base` was compared wrap-relatively and never
    /// cleared, so 32768+ packets after a rebase every NACK for the current
    /// stream read as "before base" and was dropped, for half a cycle, over
    /// and over.
    #[test]
    fn nack_long_after_a_rebase_is_still_relayed() {
        let ((a, _ra), (b, _rb)) = (leg(1), leg(2));
        assert!(join(&a, &b));
        let mut out = Outbound::new(0xAAAA);
        out.stamp(&a, 2, 0xB1, 100, 96, t(0)).unwrap();
        out.stamp(&a, 2, 0xB2, 7000, 96, t(1000)).unwrap(); // rebase: base = 101
        let mut last = (0, 0);
        for i in 1..=40_000u16 {
            last = out
                .stamp(&a, 2, 0xB2, 7000u16.wrapping_add(i), 96, t(1000))
                .unwrap();
        }
        a.relay_nacks(&[(0xAAAA, vec![(last.1, 0)])]);
        assert_eq!(
            *b.nacks.lock(),
            vec![NackReq {
                media_ssrc: 0xB2,
                fci: vec![(7000u16.wrapping_add(40_000), 0)]
            }]
        );
    }

    /// An entry whose PID predates the restart but whose BLP bits reach
    /// into the new stream is re-anchored, not dropped whole.
    #[test]
    fn nack_entry_straddling_the_rebase_is_reanchored() {
        let ((a, _ra), (b, _rb)) = (leg(1), leg(2));
        assert!(join(&a, &b));
        let mut out = Outbound::new(0xAAAA);
        out.stamp(&a, 2, 0xB1, 100, 96, t(0)).unwrap();
        assert_eq!(
            out.stamp(&a, 2, 0xB2, 7000, 96, t(1000)),
            Some((0xAAAA, 101))
        );
        out.stamp(&a, 2, 0xB2, 7010, 96, t(1000)).unwrap();
        // Asks for 99 (pid), 100 (bit 0), 101 (bit 1), 104 (bit 4): only
        // 101 and 104 are the new stream's (source 7000 and 7003).
        a.relay_nacks(&[(0xAAAA, vec![(99, 0b1_0011)])]);
        assert_eq!(
            *b.nacks.lock(),
            vec![NackReq {
                media_ssrc: 0xB2,
                fci: vec![(7000, 0b100)]
            }]
        );
        assert_eq!(clip_nack(99, 0b1, 101), None); // 99, 100: all old
        assert_eq!(clip_nack(99, 0b11, 101), Some((101, 0)));
        assert_eq!(clip_nack(101, 0b1, 101), Some((101, 0b1)));
        // Across the wrap: base 1, pid 65534 asks 65534, 65535, 2 → (2, 0).
        assert_eq!(clip_nack(65534, 0b1001, 1), Some((2, 0)));
    }

    /// Reviewer's probe: one NACK compound could carry 255 entries of 17
    /// packets each, all relayed to the source. The relayed request is
    /// capped per compound.
    #[test]
    fn nack_entries_relayed_per_compound_are_capped() {
        let ((a, _ra), (b, _rb)) = (leg(1), leg(2));
        assert!(join(&a, &b));
        let mut out = Outbound::new(0xAAAA);
        out.stamp(&a, 2, 0xB1, 0, 96, t(0)).unwrap();
        let fci: Vec<(u16, u16)> = (0..255u16).map(|i| (i * 17, 0xFFFF)).collect();
        a.relay_nacks(&[(0xAAAA, fci.clone()), (0xAAAA, fci)]);
        let q = b.nacks.lock();
        let entries: usize = q.iter().map(|r| r.fci.len()).sum();
        assert!(
            entries > 0 && entries <= MAX_NACK_FCI,
            "{entries} entries relayed"
        );
    }

    /// Identical requests (the same receiver re-NACKing, or two receivers
    /// that lost the same packet) within the window reach the source once.
    #[test]
    fn repeated_nack_for_the_same_packets_is_deduplicated() {
        let ((a, _ra), (b, _rb), (c, _rc)) = (leg(1), leg(2), leg(3));
        assert!(join(&a, &b));
        assert!(join(&a, &c));
        let (mut out_a, mut out_c) = (Outbound::new(0xAAAA), Outbound::new(0xCCCC));
        out_a.stamp(&a, 2, 0xB1, 100, 96, t(0)).unwrap();
        out_c.stamp(&c, 2, 0xB1, 100, 96, t(0)).unwrap();
        a.relay_nacks(&[(0xAAAA, vec![(103, 0b101)])]);
        a.relay_nacks(&[(0xAAAA, vec![(103, 0b101)])]);
        c.relay_nacks(&[(0xCCCC, vec![(103, 0)])]);
        // A partly-new request passes only its new part, re-packed.
        c.relay_nacks(&[(0xCCCC, vec![(104, 0b11)])]);
        assert_eq!(
            *b.nacks.lock(),
            vec![
                NackReq {
                    media_ssrc: 0xB1,
                    fci: vec![(103, 0b101)]
                },
                NackReq {
                    media_ssrc: 0xB1,
                    fci: vec![(105, 0)]
                },
            ]
        );
    }

    #[test]
    fn nack_limiter_window_expires() {
        let mut l = NackLimiter::default();
        let t0 = Instant::now();
        let req = NackReq {
            media_ssrc: 7,
            fci: vec![(65535, 0b1)], // across the wrap: 65535, 0
        };
        assert_eq!(l.admit(&req, t0), Some(req.clone()));
        assert_eq!(l.admit(&req, t0 + Duration::from_millis(50)), None);
        assert_eq!(l.admit(&req, t0 + NACK_DEDUPE_WINDOW), Some(req.clone()));
        // Another SSRC is independent.
        let other = NackReq {
            media_ssrc: 8,
            ..req.clone()
        };
        assert_eq!(l.admit(&other, t0), Some(other.clone()));
    }

    #[test]
    fn nack_wire_format() {
        let n = build_nack(
            0x1122_3344,
            &NackReq {
                media_ssrc: 0xAABB_CCDD,
                fci: vec![(0x0102, 0x0304), (0x0506, 0)],
            },
        );
        assert_eq!(n.len(), 20);
        assert_eq!(n[0], 0x81);
        assert_eq!(n[1], 205);
        assert_eq!(u16::from_be_bytes([n[2], n[3]]), 4);
        assert_eq!(&n[4..8], &0x1122_3344u32.to_be_bytes());
        assert_eq!(&n[8..12], &0xAABB_CCDDu32.to_be_bytes());
        assert_eq!(&n[12..20], &[1, 2, 3, 4, 5, 6, 0, 0]);
        assert_eq!(
            parse_feedback(&n).nacks,
            vec![(0xAABB_CCDD, vec![(0x0102, 0x0304), (0x0506, 0)])]
        );
    }

    fn named(v: &[(&str, u32)]) -> Vec<(String, u32)> {
        v.iter().map(|(l, p)| (l.to_string(), *p)).collect()
    }

    /// Finding: every forwarded packet was re-stamped to the receiving leg's
    /// one PT, so an RTX / FEC / second-codec packet went out labelled as the
    /// main codec and the receiver decoded it as such. Today's callers (one
    /// codec PT per leg, `remote.codec`) map primary to primary exactly as
    /// before; anything else is dropped.
    #[test]
    fn pt_map_single_codec_is_primary_to_primary_and_drops_the_rest() {
        let (src, dst) = (PtMap::new(96, []), PtMap::new(98, []));
        assert_eq!(dst.map_from(&src, 96), Some(98));
        assert_eq!(
            dst.map_from(&src, 97),
            None,
            "RTX re-stamped as the video codec"
        );
        assert_eq!(src.map_from(&src, 96), Some(96));
        // A receiver that declared nothing takes the source PT unchanged.
        let none = PtMap::new(0, []);
        assert!(none.is_empty());
        assert_eq!(none.map_from(&src, 97), Some(97));
        // A source that declared nothing has no codec to map from.
        assert_eq!(dst.map_from(&none, 96), None);
        assert!(src.declares(96) && !src.declares(97) && none.declares(97));
    }

    /// The resolution limit needs a PT's payload format, and only a
    /// `remote.codecs` label names one — never guessed from a bare PT.
    #[test]
    fn pt_map_format_comes_from_the_codec_label_only() {
        use crate::channel::video_dims::VideoFormat;
        let m = PtMap::new(
            96,
            named(&[
                ("VP8", 96),
                ("h264/42e01f", 102),
                ("H264", 104),
                ("av1", 45),
            ]),
        );
        assert_eq!(m.format_of(96), Some(VideoFormat::Vp8));
        assert_eq!(m.format_of(102), Some(VideoFormat::H264));
        assert_eq!(m.format_of(104), Some(VideoFormat::H264));
        assert_eq!(m.format_of(45), None);
        assert_eq!(m.format_of(97), None);
        // primary only: unknown
        assert_eq!(PtMap::new(96, []).format_of(96), None);
        // one PT labelled as two formats: unknown
        let m = PtMap::new(0, named(&[("vp8", 96), ("h264", 96)]));
        assert_eq!(m.format_of(96), None);
    }

    #[test]
    fn pt_map_named_codecs_map_by_label() {
        let src = PtMap::new(96, named(&[("VP8", 96), ("h264/42e01f", 102), ("rtx", 97)]));
        let dst = PtMap::new(
            100,
            named(&[(" vp8 ", 100), ("H264/42E01F", 108), ("av1", 45)]),
        );
        assert_eq!(dst.map_from(&src, 96), Some(100));
        assert_eq!(dst.map_from(&src, 102), Some(108));
        assert_eq!(dst.map_from(&src, 97), None, "no rtx on the receiver");
        assert_eq!(dst.map_from(&src, 45), None, "the source never declared 45");
        // No primary on the receiver: a primary packet maps by its label.
        let dst2 = PtMap::new(0, named(&[("vp8", 120)]));
        assert_eq!(dst2.map_from(&src, 96), Some(120));
        assert_eq!(dst2.map_from(&src, 102), None);
        // Invalid entries are ignored; the table is bounded.
        let bad = PtMap::new(300, named(&[("x", 0), ("y", 128), ("", 99)]));
        assert!(bad.is_empty());
        let many: Vec<(String, u32)> = (0..100).map(|i| (format!("c{i}"), 96)).collect();
        assert_eq!(PtMap::new(0, many).named.len(), MAX_NAMED_CODECS);
    }

    /// `fan_out` hands each receiver the source's payload types with the
    /// packet, so its send task maps against the source's current table.
    #[test]
    fn frames_carry_the_source_payload_types() {
        let ((a, _ra), (b, mut rb)) = (leg(1), leg(2));
        assert!(join(&a, &b));
        a.set_pts(PtMap::new(96, named(&[("vp8", 96)])));
        a.fan_out(rtp(1));
        let f = rb.try_recv().unwrap();
        assert_eq!(*f.src_pts, PtMap::new(96, named(&[("vp8", 96)])));
    }

    /// Finding: the relay invented an outbound SSRC per source and never
    /// said which, so signalling could not announce extra streams to a
    /// receiver. `streams()` (livestats) reports the current mapping.
    #[test]
    fn streams_report_the_outbound_mapping_by_source_label() {
        let labelled = |id: u64, label: &str| {
            let (tx, rx) = mpsc::channel(8);
            let l = RelayShared::new(id, tx, false, 96).with_label(label.into());
            (Arc::new(l), rx)
        };
        let ((me, _rm), (a, _ra), (b, _rb)) = (
            labelled(9, "me"),
            labelled(1, "uuid-a"),
            labelled(2, "uuid-b"),
        );
        assert!(join(&me, &a));
        assert!(join(&me, &b));
        assert!(me.streams().is_empty());
        let mut out = Outbound::new(0xAAAA);
        out.stamp(&me, 1, 0xA1, 1, 100, t(0)).unwrap();
        let (a2, _) = out.stamp(&me, 1, 0xA2, 1, 100, t(1)).unwrap();
        let (b1, _) = out.stamp(&me, 2, 0xB1, 1, 101, t(1)).unwrap();
        let info = |source: &str, ssrc, source_ssrc, pt| StreamInfo {
            source: source.into(),
            ssrc,
            source_ssrc,
            pt: Some(pt),
        };
        assert_eq!(
            me.streams(),
            vec![
                info("uuid-a", 0xAAAA, 0xA1, 100),
                info("uuid-a", a2, 0xA2, 100),
                info("uuid-b", b1, 0xB1, 101),
            ]
        );
        // A PT change on a stream shows.
        out.stamp(&me, 2, 0xB1, 2, 102, t(2)).unwrap();
        assert_eq!(me.streams()[2].pt, Some(102));
        // A source that left is not listed, even before the send task reaps.
        b.leave_group();
        assert_eq!(me.streams().len(), 2);
        assert!(me.streams().iter().all(|s| s.source == "uuid-a"));
        // An unlabelled leg is named by its id.
        assert_eq!(leg(7).0.label, "7");
    }

    #[test]
    fn seq_map_preserves_gaps_and_order() {
        let mut m = SeqMap::default();
        // A fresh series passes straight through; loss and reorder survive.
        assert_eq!(m.map(1000), 1000);
        assert_eq!(m.map(1003), 1003); // 1001-1002 lost upstream
        assert_eq!(m.map(1002), 1002); // late, still in order slot
        assert_eq!(m.highest(), Some(1003));
        // Wrap.
        let mut w = SeqMap::default();
        assert_eq!(w.map(65534), 65534);
        assert_eq!(w.map(1), 1); // 65535, 0 lost across the wrap
        assert_eq!(w.highest(), Some(1));
        assert_eq!(w.map(65535), 65535); // late; highest unchanged
        assert_eq!(w.highest(), Some(1));
    }

    #[test]
    fn seq_map_continuing_rebases_without_reuse() {
        // A stream taking over a series that reached 501, starting anywhere,
        // continues at highest + 1 ...
        let mut m = SeqMap::continuing(Some(501));
        assert_eq!(m.map(9), 502);
        // ... and keeps its own gaps under the new offset.
        assert_eq!(m.map(12), 505);
        assert_eq!(m.map(11), 504);
        assert_eq!(m.route(), (502u16.wrapping_sub(9), Some(502)));
    }

    /// Finding: `SeqMap` followed one SSRC per source and remembered the one
    /// before it (`prev_ssrc`) to drop its stragglers. A second SSRC from the
    /// same source (RTX, FEC, simulcast, one stray packet) arriving between
    /// two packets of the main stream — A, B, A — left A marked as the
    /// previous stream, and every later A packet was dropped for the rest of
    /// the call. Each source SSRC now has its own mapping and outbound SSRC.
    #[test]
    fn second_ssrc_alongside_the_main_stream_never_kills_it() {
        let ((me, _rm), (a, _ra)) = (leg(9), leg(1));
        assert!(join(&me, &a));
        let mut out = Outbound::new(0xAAAA);
        assert_eq!(out.stamp(&me, 1, 0xA, 100, 96, t(0)), Some((0xAAAA, 100)));
        let (rtx, sn) = out.stamp(&me, 1, 0xB, 7, 96, t(1)).unwrap();
        assert!(rtx != 0xAAAA && rtx != 0, "second SSRC reused the main one");
        assert_eq!(sn, 7, "a fresh outbound SSRC passes its numbers through");
        // Interleaved: each keeps its own series; neither is dropped.
        for k in 1..=50u16 {
            let ms = 2 + u64::from(k);
            assert_eq!(
                out.stamp(&me, 1, 0xA, 100 + k, 96, t(ms)),
                Some((0xAAAA, 100 + k))
            );
            assert_eq!(out.stamp(&me, 1, 0xB, 7 + k, 96, t(ms)), Some((rtx, 7 + k)));
        }
        // A NACK for either outbound stream reaches the source under the
        // right source SSRC.
        me.relay_nacks(&[(0xAAAA, vec![(120, 0)]), (rtx, vec![(20, 0)])]);
        assert_eq!(
            *a.nacks.lock(),
            vec![
                NackReq {
                    media_ssrc: 0xA,
                    fci: vec![(120, 0)]
                },
                NackReq {
                    media_ssrc: 0xB,
                    fci: vec![(20, 0)]
                },
            ]
        );
        let mut routes: Vec<(u32, u32)> = me
            .out_routes
            .lock()
            .iter()
            .map(|r| (r.ssrc, r.src_ssrc))
            .collect();
        routes.sort();
        let mut want = vec![(0xAAAA, 0xA), (rtx, 0xB)];
        want.sort();
        assert_eq!(routes, want);
    }

    /// A source that restarts under a new SSRC once its main stream has gone
    /// quiet keeps the receiver on the same outbound SSRC, its series
    /// continuing — and a late packet of the old SSRC afterwards gets a
    /// stream of its own rather than rebasing (or disabling) the new one.
    #[test]
    fn restart_after_the_main_stream_goes_quiet_keeps_the_outbound_identity() {
        let ((me, _rm), (a, _ra)) = (leg(9), leg(1));
        assert!(join(&me, &a));
        let mut out = Outbound::new(0xAAAA);
        assert_eq!(out.stamp(&me, 1, 0xA, 500, 96, t(0)), Some((0xAAAA, 500)));
        assert_eq!(out.stamp(&me, 1, 0xA, 501, 96, t(10)), Some((0xAAAA, 501)));
        let later = 10 + MAIN_IDLE_TAKEOVER.as_millis() as u64;
        assert_eq!(out.stamp(&me, 1, 0xB, 9, 96, t(later)), Some((0xAAAA, 502)));
        assert_eq!(
            out.stamp(&me, 1, 0xB, 12, 96, t(later + 1)),
            Some((0xAAAA, 505))
        );
        let (old, _) = out.stamp(&me, 1, 0xA, 502, 96, t(later + 2)).unwrap();
        assert_ne!(
            old, 0xAAAA,
            "a straggler of the old SSRC took the main stream"
        );
        assert_eq!(
            out.stamp(&me, 1, 0xB, 13, 96, t(later + 3)),
            Some((0xAAAA, 506))
        );
    }

    /// A restart whose new SSRC shows up before the old one has been quiet
    /// for `MAIN_IDLE_TAKEOVER` starts on an SSRC of its own, and takes the
    /// main identity over as soon as the old stream has been quiet that long.
    #[test]
    fn a_new_ssrc_takes_over_once_the_main_stream_has_gone_quiet() {
        let ((me, _rm), (a, _ra)) = (leg(9), leg(1));
        assert!(join(&me, &a));
        let mut out = Outbound::new(0xAAAA);
        assert_eq!(out.stamp(&me, 1, 0xA, 100, 96, t(0)), Some((0xAAAA, 100)));
        let (own, _) = out.stamp(&me, 1, 0xB, 3000, 96, t(20)).unwrap();
        assert_ne!(own, 0xAAAA);
        let quiet = MAIN_IDLE_TAKEOVER.as_millis() as u64;
        assert_eq!(
            out.stamp(&me, 1, 0xB, 3001, 96, t(quiet)),
            Some((0xAAAA, 101))
        );
        let routes: Vec<(u32, u32)> = me
            .out_routes
            .lock()
            .iter()
            .map(|r| (r.ssrc, r.src_ssrc))
            .collect();
        assert_eq!(
            routes,
            vec![(0xAAAA, 0xB)],
            "the old outbound SSRC stays routed"
        );
        // Its old outbound SSRC is free again for the next stream.
        assert_eq!(
            out.stamp(&me, 1, 0xC, 1, 96, t(quiet + 1)).map(|o| o.0),
            Some(own)
        );
    }

    /// Per-source stream state is bounded: an endpoint (or, on a non-secure
    /// leg, anyone) sending ever-new SSRCs recycles the least recently used
    /// slots, never the main stream, and never grows the map.
    #[test]
    fn streams_per_source_are_bounded_and_the_main_stream_survives() {
        let ((me, _rm), (a, _ra)) = (leg(9), leg(1));
        assert!(join(&me, &a));
        let mut out = Outbound::new(0xAAAA);
        let mut ms = 0;
        let mut tick = || {
            ms += 1;
            t(ms)
        };
        assert_eq!(out.stamp(&me, 1, 0xA, 0, 96, tick()), Some((0xAAAA, 0)));
        let (kept, _) = out.stamp(&me, 1, 0xF00D, 0, 96, tick()).unwrap();
        for k in 0..1000u32 {
            let seq = (k + 1) as u16;
            out.stamp(&me, 1, 0x1_0000 + k, 0, 96, tick()).unwrap();
            // The main stream and one busy extra stream keep flowing.
            assert_eq!(out.stamp(&me, 1, 0xA, seq, 96, tick()), Some((0xAAAA, seq)));
            assert_eq!(
                out.stamp(&me, 1, 0xF00D, seq, 96, tick()),
                Some((kept, seq))
            );
        }
        assert_eq!(out.sources[&1].streams.len(), MAX_STREAMS_PER_SOURCE);
        assert_eq!(me.out_routes.lock().len(), MAX_STREAMS_PER_SOURCE);
        assert!(out.free.len() <= 1, "free pool grew: {}", out.free.len());
    }

    /// Packets still in flight from a source leg that has left are dropped
    /// whichever of its SSRCs they carry and in whatever order they land.
    #[test]
    fn every_ssrc_of_a_departed_source_is_dropped() {
        let ((me, _rm), (a, _ra), (b, _rb)) = (leg(9), leg(1), leg(2));
        assert!(join(&me, &a));
        assert!(join(&me, &b));
        let mut out = Outbound::new(0xAAAA);
        out.stamp(&me, 1, 0x11, 10, 96, t(0)).unwrap();
        out.stamp(&me, 2, 0x21, 20, 96, t(0)).unwrap();
        out.stamp(&me, 2, 0x22, 20, 96, t(0)).unwrap();
        b.leave_group();
        assert_eq!(out.stamp(&me, 2, 0x22, 21, 96, t(1)), None);
        assert_eq!(out.stamp(&me, 1, 0x11, 11, 96, t(1)), Some((0xAAAA, 11)));
        assert_eq!(out.stamp(&me, 2, 0x21, 21, 96, t(2)), None);
        assert_eq!(out.stamp(&me, 2, 0x23, 1, 96, t(2000)), None);
        assert_eq!(routed_sources(&me), vec![1]);
        assert_eq!(out.free.len(), 2, "both of b's outbound SSRCs freed");
    }

    #[test]
    fn keyframe_request_inside_compound() {
        // A browser's PLI arrives as [RR, PLI] — RFC 3550 compound rules put
        // an RR/SR first. Classifying only the first header misses it.
        let rr = [0x80u8, 201, 0, 1, 0, 0, 0, 1]; // RR, no report blocks
        let mut compound = rr.to_vec();
        compound.extend_from_slice(&build_pli(0x1, 0x2));
        assert!(wants_keyframe(&compound));
        // an RR alone asks for nothing
        assert!(!wants_keyframe(&rr));
        // transport-cc (205/15) behind an RR stays ignored
        let mut cc = rr.to_vec();
        cc.extend_from_slice(&[0x8Fu8, 205, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2]);
        assert!(!wants_keyframe(&cc));
    }

    #[test]
    fn pli_wire_format() {
        let p = build_pli(0x1122_3344, 0xAABB_CCDD);
        assert_eq!(p[0], 0x81);
        assert_eq!(p[1], 206);
        assert_eq!(u16::from_be_bytes([p[2], p[3]]), 2);
        assert_eq!(u32::from_be_bytes([p[4], p[5], p[6], p[7]]), 0x1122_3344);
        assert_eq!(u32::from_be_bytes([p[8], p[9], p[10], p[11]]), 0xAABB_CCDD);
        assert!(wants_keyframe(&p));
    }

    #[test]
    fn keyframe_classification() {
        // FIR (206 fmt 4) → yes; NACK (205 fmt 1) → no (passed through).
        assert!(wants_keyframe(&[0x84, 206, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2]));
        assert!(!wants_keyframe(&[
            0x81, 205, 0, 3, 0, 0, 0, 1, 0, 0, 0, 2, 0, 1, 0, 0
        ]));
        // transport-cc (205 fmt 15) and REMB (206 fmt 15) → no.
        assert!(!wants_keyframe(&[0x8F, 205, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2]));
        assert!(!wants_keyframe(&[0x8F, 206, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2]));
        // SR / RR → no.
        assert!(!wants_keyframe(&[0x80, 200, 0, 6, 0, 0, 0, 1, 0, 0, 0, 2]));
        assert!(!wants_keyframe(&[0x80, 201, 0, 1, 0, 0, 0, 1, 0, 0, 0, 2]));
        // Short garbage → no.
        assert!(!wants_keyframe(&[0x81, 206]));
    }

    /// A closed leg's send task is gone and nothing would ever take it out
    /// of a group again, so `mix()` with one is refused.
    #[test]
    fn closed_leg_cannot_be_mixed_again() {
        let ((a, _ra), (b, _rb), (c, _rc)) = (leg(1), leg(2), leg(3));
        assert!(join(&a, &b));
        b.close();
        assert!(!a.has_member(2));
        assert!(!join(&a, &b));
        assert!(!join(&b, &c));
        assert!(!a.has_member(2) && !c.has_member(2));
        assert!(join(&a, &c));
    }

    /// The dedupe window only catches repeats: a receiver NACKing ever-new
    /// sequence numbers is held to its budget instead.
    #[test]
    fn nacks_from_one_remote_are_held_to_a_budget() {
        let ((a, _ra), (b, _rb)) = (leg(1), leg(2));
        assert!(join(&a, &b));
        let mut out = Outbound::new(0xAAAA);
        out.stamp(&a, 2, 0xB1, 0, 96, t(0)).unwrap();
        let (mut asked, mut passed) = (0u64, 0u64);
        for i in 0..200u16 {
            a.relay_nacks(&[(0xAAAA, vec![(i * 17, 0xFFFF)])]);
            asked += 17;
            for req in b.nacks.lock().drain(..) {
                passed += req
                    .fci
                    .iter()
                    .map(|(_, blp)| 1 + u64::from(blp.count_ones()))
                    .sum::<u64>();
            }
        }
        let burst = NACK_SEQ_BURST as u64;
        assert!(
            passed >= burst - 17 && passed <= burst + 100,
            "{passed} sequence numbers relayed"
        );
        assert_eq!(a.snapshot().nack_dropped, asked - passed);
    }
}
