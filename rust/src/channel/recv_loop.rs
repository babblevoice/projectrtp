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

use super::dtls_session::{remote_srtp_params, SrtpKeyingMaterial};
use super::jitter::JitterBuffer;
use super::relay::{self, RelayShared};
use super::relay_limits::{InboundGuard, Verdict};
use super::rtcp_loop;
use super::rtcp_stats::{RemoteReport, RxStats};
use super::rtp::{self, RtpPacket};
use crate::stun;
use webrtc_srtp::protection_profile::ProtectionProfile;

/// Relay-mode wiring for the recv loop: inbound RTP is decrypted here and
/// fanned out to the other legs of the relay group instead of entering the
/// jitter buffer.
pub struct RelayRecv {
    /// This channel's own shared face — inbound counting, and (through its
    /// group, set by `mix()` / cleared by `unmix()`) the legs to forward to.
    pub own: Arc<RelayShared>,
    /// Highest authenticated RTP sequence / SRTCP index seen per SSRC — a
    /// secure leg's remote moves only for a packet newer than these.
    pub fresh: PLMutex<Freshness>,
    /// The leg's abuse limits, applied to every accepted packet before
    /// fan-out (see relay_limits.rs). Only this loop takes it — a leaf.
    pub guard: PLMutex<InboundGuard>,
}

impl RelayRecv {
    pub fn new(own: Arc<RelayShared>) -> Self {
        let guard = PLMutex::new(InboundGuard::new(own.limits()));
        Self {
            own,
            fresh: PLMutex::new(Freshness::default()),
            guard,
        }
    }
}

/// Replay guard for address latching on a secure relay leg. The inbound
/// SRTP/SRTCP contexts run without libsrtp replay protection, so a captured
/// authentic packet replayed from another address still decrypts; latching
/// on it would hand the leg's media to whoever replayed it. A packet may
/// move the remote only when it is newer than everything already accepted
/// under its SSRC — a replay, by definition, is not.
///
/// RTP compares 16-bit sequence numbers wrap-relatively, which is sound
/// here: the SRTP decrypt context estimates the packet index within ±2^15
/// of its highest, and the ROC is covered by the auth tag, so a packet that
/// authenticates and *looks* newer really is. SRTCP carries its 31-bit
/// index explicitly (and authenticated), so it compares directly.
#[derive(Default)]
pub struct Freshness {
    rtp: std::collections::HashMap<u32, u16>,
    rtcp: std::collections::HashMap<u32, u32>,
    /// Transaction IDs of STUN requests that have already been considered
    /// for a latch, oldest first. A signed request is authentic but not
    /// fresh: replayed verbatim from elsewhere it still verifies.
    stun: std::collections::VecDeque<[u8; 12]>,
}

/// Bound on remembered STUN transaction IDs. A browser pings the selected
/// pair every few seconds, so this spans several minutes of history; a
/// replay older than that can latch, but only until the browser's next
/// ping or SRTP packet latches back.
const FRESH_MAX_STUN: usize = 256;

/// Bound on SSRCs tracked per direction. Only authenticated packets insert,
/// so this is the far end's own stream count; the cap only guards memory.
const FRESH_MAX_SSRCS: usize = 64;

impl Freshness {
    /// Record an authenticated RTP packet; true when it is newer than any
    /// seen under its SSRC (and so may move the remote).
    pub fn rtp(&mut self, ssrc: u32, seq: u16) -> bool {
        match self.rtp.get(&ssrc) {
            Some(&h) if !(seq != h && seq.wrapping_sub(h) < 0x8000) => false,
            _ => {
                if self.rtp.len() >= FRESH_MAX_SSRCS && !self.rtp.contains_key(&ssrc) {
                    self.rtp.clear();
                }
                self.rtp.insert(ssrc, seq);
                true
            }
        }
    }

    /// Record an authenticated STUN request's transaction ID; true the
    /// first time it is seen.
    pub fn stun(&mut self, txid: [u8; 12]) -> bool {
        if self.stun.contains(&txid) {
            return false;
        }
        if self.stun.len() >= FRESH_MAX_STUN {
            self.stun.pop_front();
        }
        self.stun.push_back(txid);
        true
    }

    /// Record an authenticated SRTCP packet by its SRTCP index.
    pub fn rtcp(&mut self, ssrc: u32, index: u32) -> bool {
        match self.rtcp.get(&ssrc) {
            Some(&h) if index <= h => false,
            _ => {
                if self.rtcp.len() >= FRESH_MAX_SSRCS && !self.rtcp.contains_key(&ssrc) {
                    self.rtcp.clear();
                }
                self.rtcp.insert(ssrc, index);
                true
            }
        }
    }
}

/// The SRTCP index (RFC 3711 §3.4) of a protected packet: the E-flagged
/// word before the auth tag (AES-CM/HMAC) or the trailing word (AEAD-GCM,
/// whose tag sits inside the ciphertext).
fn srtcp_index(pkt: &[u8], profile: ProtectionProfile) -> Option<u32> {
    let end = pkt.len().checked_sub(profile.rtcp_auth_tag_len())?;
    let start = end.checked_sub(4)?;
    if start < 8 {
        return None;
    }
    let w = u32::from_be_bytes([pkt[start], pkt[start + 1], pkt[start + 2], pkt[start + 3]]);
    Some(w & 0x7FFF_FFFF)
}

/// Is this authenticated SRTCP datagram newer than any seen from its sender?
/// Most SSRCs a relay leg's real SRTP/SRTCP context is allowed to hold. Only
/// SSRCs that have authenticated get there, i.e. the browser's own streams.
const GATE_MAX_TRUSTED: usize = 64;
/// SSRCs a probe context holds before it is thrown away and rebuilt.
const GATE_MAX_PROBED: usize = 64;

/// Bounds the per-SSRC state webrtc-srtp keeps for a relay leg.
///
/// A webrtc-srtp context creates an SSRC's state (replay window, rollover
/// count) on the first decrypt *attempt*, before the packet authenticates,
/// and never drops it. A relay leg decrypts every datagram it receives, so
/// forged packets under ever-new SSRCs would grow the context without bound
/// (~86 bytes a packet) until the node runs out of memory. The real context
/// can't be pruned or rebuilt either: its rollover counts are private.
///
/// So a packet under an SSRC that has not authenticated yet is first tried
/// on a throwaway probe context - rebuilt every `GATE_MAX_PROBED` SSRCs, so
/// bounded - and only once it authenticates there is the SSRC trusted and
/// handed to the real context. A new stream starts at rollover count 0,
/// which is what a fresh probe context assumes. One gate per context: RTP
/// and RTCP SSRCs are separate spaces.
pub struct SsrcGate {
    key: Vec<u8>,
    salt: Vec<u8>,
    profile: ProtectionProfile,
    probe: Option<webrtc_srtp::context::Context>,
    probed: std::collections::HashSet<u32>,
    trusted: std::collections::HashSet<u32>,
}

impl SsrcGate {
    pub fn new(keys: &SrtpKeyingMaterial) -> Self {
        let (key, salt, profile) = remote_srtp_params(keys);
        Self {
            key: key.to_vec(),
            salt: salt.to_vec(),
            profile,
            probe: None,
            probed: std::collections::HashSet::new(),
            trusted: std::collections::HashSet::new(),
        }
    }

    /// Decrypt `pkt` (under `ssrc`) with the leg's real context `ctx`, via
    /// the probe context first if `ssrc` has not authenticated before.
    /// `decrypt` is `Context::decrypt_rtp` or a length-checked
    /// `decrypt_rtcp`. `None` on any failure.
    pub fn decrypt(
        &mut self,
        ctx: &mut webrtc_srtp::context::Context,
        ssrc: u32,
        pkt: &[u8],
        decrypt: impl Fn(&mut webrtc_srtp::context::Context, &[u8]) -> Option<bytes::Bytes>,
    ) -> Option<bytes::Bytes> {
        if !self.trusted.contains(&ssrc)
            && (self.trusted.len() >= GATE_MAX_TRUSTED || !self.probe(ssrc, pkt, &decrypt))
        {
            return None;
        }
        let plain = decrypt(ctx, pkt)?;
        self.trusted.insert(ssrc);
        Some(plain)
    }

    fn probe(
        &mut self,
        ssrc: u32,
        pkt: &[u8],
        decrypt: &impl Fn(&mut webrtc_srtp::context::Context, &[u8]) -> Option<bytes::Bytes>,
    ) -> bool {
        if self.probed.len() >= GATE_MAX_PROBED && !self.probed.contains(&ssrc) {
            self.probe = None;
            self.probed.clear();
        }
        if self.probe.is_none() {
            self.probe =
                webrtc_srtp::context::Context::new(&self.key, &self.salt, self.profile, None, None)
                    .ok();
        }
        let Some(probe) = self.probe.as_mut() else {
            return false;
        };
        self.probed.insert(ssrc);
        decrypt(probe, pkt).is_some()
    }
}

/// The relay leg's SSRC gates, built with its decrypt contexts.
#[derive(Default)]
pub struct RelayGates {
    pub rtp: Option<SsrcGate>,
    pub rtcp: Option<SsrcGate>,
}

impl RelayGates {
    fn rtp(
        &mut self,
        key_rx: &watch::Receiver<Option<SrtpKeyingMaterial>>,
    ) -> Option<&mut SsrcGate> {
        if self.rtp.is_none() {
            self.rtp = key_rx.borrow().as_ref().map(SsrcGate::new);
        }
        self.rtp.as_mut()
    }

    fn rtcp(
        &mut self,
        key_rx: &watch::Receiver<Option<SrtpKeyingMaterial>>,
    ) -> Option<&mut SsrcGate> {
        if self.rtcp.is_none() {
            self.rtcp = key_rx.borrow().as_ref().map(SsrcGate::new);
        }
        self.rtcp.as_mut()
    }
}

/// Decrypt one inbound SRTCP datagram: length-checked always, and on a relay
/// leg through its SSRC gate (see `SsrcGate`). Audio legs keep main's
/// behaviour apart from the length check.
fn decrypt_srtcp(
    cfg: &RecvLoopConfig,
    ctx: &mut webrtc_srtp::context::Context,
    gates: &mut RelayGates,
    pkt: &[u8],
) -> Option<bytes::Bytes> {
    let profile = cfg.key_rx.borrow().as_ref().map(|k| k.profile);
    let checked = |c: &mut webrtc_srtp::context::Context, p: &[u8]| {
        rtcp_loop::decrypt_rtcp_checked(c, p, profile)
    };
    if cfg.relay.is_none() {
        return checked(ctx, pkt);
    }
    // the length check first: the sender SSRC below needs 8 bytes
    let min = rtcp_loop::srtcp_min_len(profile?);
    if pkt.len() < min {
        return None;
    }
    let ssrc = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);
    gates.rtcp(&cfg.key_rx)?.decrypt(ctx, ssrc, pkt, checked)
}

fn fresh_rtcp(cfg: &RecvLoopConfig, pkt: &[u8]) -> bool {
    let (Some(relay_cfg), Some(profile)) =
        (&cfg.relay, cfg.key_rx.borrow().as_ref().map(|k| k.profile))
    else {
        return false;
    };
    match srtcp_index(pkt, profile) {
        // RTCP's sender SSRC is the second word (cleartext under SRTCP).
        Some(idx) => {
            let ssrc = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);
            relay_cfg.fresh.lock().rtcp(ssrc, idx)
        }
        None => false,
    }
}

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
    /// `Some` for relay-mode (video) channels — see channel/relay.rs.
    pub relay: Option<RelayRecv>,
}

pub fn spawn(cfg: RecvLoopConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run(cfg))
}

async fn run(cfg: RecvLoopConfig) {
    let mut buf = [0u8; rtp::RTP_MAX_LENGTH];
    // Built lazily on the first muxed SRTCP packet after keys arrive; a plain
    // (non-secure) channel never populates it and muxed RTCP stays cleartext.
    let mut srtcp_decrypt: Option<webrtc_srtp::context::Context> = None;
    // Relay only: inbound SRTP decrypt context, built lazily like the above.
    // The audio path decrypts at jitter-pop time in the tick instead.
    let mut srtp_decrypt: Option<webrtc_srtp::context::Context> = None;
    let mut gates = RelayGates::default();
    loop {
        tokio::select! {
            biased;
            _ = cfg.cancel.cancelled() => break,
            result = cfg.sock.recv_from(&mut buf) => {
                match result {
                    Ok((n, peer)) => {
                        handle_packet(&cfg, &buf[..n], peer, &mut srtcp_decrypt, &mut srtp_decrypt, &mut gates).await;
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
    srtp_decrypt: &mut Option<webrtc_srtp::context::Context>,
    gates: &mut RelayGates,
) {
    // Symmetric RTP: send back to wherever the far end sends from. On a
    // secure relay leg only an *authenticated* packet may move the address
    // (see `latch`) — the latch calls below; everything else latches here,
    // unchanged.
    let guarded = cfg
        .relay
        .as_ref()
        .is_some_and(|r| r.own.secure.load(Ordering::Relaxed));
    if !guarded {
        latch(cfg, peer);
    }
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
        // On a secure relay leg a bare Binding Request (no MESSAGE-INTEGRITY)
        // is neither answered nor latched: answering is a free reflector and
        // latching would let anyone redirect the leg's media.
        let n = stun::handle_checked(&mut req, &mut resp, peer, &key, &key, guarded);
        if n > 0 {
            // On a guarded leg a response means MESSAGE-INTEGRITY verified —
            // authentic, but not necessarily fresh or nominated. Every such
            // request is answered (consent and backup-pair checks need it),
            // but only a *nomination* (signed USE-CANDIDATE, RFC 8445
            // §7.3.1.5; we are ice-lite so the browser is controlling and
            // sends it on the pair it selects) with an unseen transaction
            // ID moves the remote. Otherwise a captured ping replayed from
            // elsewhere, or a backup pair's check, would redirect media.
            // Before any remote exists, any fresh signed request latches so
            // ICE/DTLS can start whatever the browser's nomination mode.
            // NAT rebinding is followed by the SRTP/SRTCP freshness latch.
            if guarded {
                let first = cfg.remote_addr.lock().is_none();
                let nominated = first || stun::nominates(pkt);
                if nominated {
                    let fresh = cfg
                        .relay
                        .as_ref()
                        .is_some_and(|r| r.fresh.lock().stun(stun::transaction_id(pkt)));
                    if fresh {
                        latch(cfg, peer);
                    }
                }
            }
            let _ = cfg.sock.send_to(&resp[..n], peer).await;
        }
        return;
    }

    // DTLS — feed to DTLSConn if active.
    if (20..=63).contains(&first) {
        // The handshake needs an address to answer; once keyed, DTLS is no
        // longer allowed to steer where media goes.
        if guarded && cfg.key_rx.borrow().is_none() {
            latch(cfg, peer);
        }
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
        // Decrypt ONCE and hand the plaintext to both consumers (the RTCP
        // accounting and, on relay legs, the keyframe scan below) rather than
        // decrypting the same datagram twice.
        let plain: Option<Vec<u8>> = match srtcp_decrypt.as_mut() {
            Some(ctx) => decrypt_srtcp(cfg, ctx, gates, pkt).map(|b| b.to_vec()),
            None => Some(pkt.to_vec()),
        };
        if let Some(relay_cfg) = &cfg.relay {
            count_rtcp(&relay_cfg.own, srtcp_decrypt.is_some(), plain.is_some());
        }
        if guarded && srtcp_decrypt.is_some() && plain.is_some() && fresh_rtcp(cfg, pkt) {
            latch(cfg, peer);
        }
        if let Some(plain) = plain {
            rtcp_loop::handle_rtcp_plain(&plain, &cfg.rx_stats, &cfg.remote_report, cfg.local_ssrc);
            // Relay only: a browser's PLI/FIR/NACK rides INSIDE this compound
            // (a leading RR/SR first, per RFC 3550 §6.1) — it is not a
            // separate 205/206-first datagram, so it must be scanned for
            // here or a keyframe request never crosses the relay and a
            // receiver that missed the first keyframe shows black forever.
            // On a secure leg only scan once a decrypt context exists: before
            // keys, `plain` is still ciphertext and random bytes could
            // classify as 205/206.
            if let Some(relay_cfg) = &cfg.relay {
                let scannable =
                    srtcp_decrypt.is_some() || !relay_cfg.own.secure.load(Ordering::Relaxed);
                if scannable {
                    relay_cfg.own.on_feedback(&relay::parse_feedback(&plain));
                }
            }
        }
        return;
    }

    // Relay only: RTCP *feedback* (RTPFB 205 / PSFB 206) also rides the
    // muxed port but is outside `is_muxed_rtcp`'s 200..=204 window. Decrypt
    // and classify — a keyframe request from our remote is about a stream
    // we send, which originates at another leg's source (mapped back from
    // the SSRC it names), so that leg's send task is asked to PLI its
    // remote. Audio channels never see this
    // branch, keeping their demux behaviour bit-exact.
    if let Some(relay_cfg) = &cfg.relay {
        if is_muxed_rtcp_fb(pkt) {
            rtcp_loop::maybe_build_decrypt(&cfg.key_rx, srtcp_decrypt);
            let keyed = srtcp_decrypt.is_some();
            let (ok, fb) = match srtcp_decrypt.as_mut() {
                Some(ctx) => match decrypt_srtcp(cfg, ctx, gates, pkt) {
                    Some(plain) => (true, Some(relay::parse_feedback(&plain))),
                    None => (false, None),
                },
                // Pre-key on a secure leg this is ciphertext — don't scan it.
                None if relay_cfg.own.secure.load(Ordering::Relaxed) => (true, None),
                None => (true, Some(relay::parse_feedback(pkt))),
            };
            count_rtcp(&relay_cfg.own, keyed, ok);
            if guarded && keyed && ok && fresh_rtcp(cfg, pkt) {
                latch(cfg, peer);
            }
            if let Some(fb) = fb {
                relay_cfg.own.on_feedback(&fb);
            }
            return;
        }
    }

    // RTP / DTMF — push to jitter. DTMF (rfc2833) classification happens
    // at pop time in the tick, since it needs access to Subsystems.
    if pkt.len() >= rtp::RTP_FIXED_HEADER_LEN {
        cfg.in_count.fetch_add(1, Ordering::Relaxed);
        // RFC 3550 receiver accounting: sequence/loss (A.1/A.3) and
        // interarrival jitter (A.8). DTMF (rfc2833) shares the audio stream's
        // SSRC and sequence space, so it is counted here too. A relay leg
        // accounts below, only for what it accepts.
        if cfg.relay.is_none() {
            rx_account(cfg, pkt);
        }

        // Relay: decrypt here and hand the packet straight to the other
        // group legs' send tasks — no jitter buffer, no tick, no codec graph. Forwarded
        // in arrival order; the send task keeps the source's sequence
        // numbers (plus a per-leg offset), so the receiving browser still
        // sees any reordering and loss and reorders / NACKs itself. `try_send`
        // so a slow peer can never block this socket's read loop — a drop is
        // counted, shows as a sequence gap at the receiver, and the PLI path
        // recovers the picture.
        if let Some(relay_cfg) = &cfg.relay {
            relay_cfg.own.in_count.fetch_add(1, Ordering::Relaxed);
            let plain: Option<RtpPacket> = if relay_cfg.own.secure.load(Ordering::Relaxed) {
                maybe_build_rtp_decrypt(&cfg.key_rx, srtp_decrypt);
                let gate = gates.rtp(&cfg.key_rx);
                match srtp_decrypt.as_mut().zip(gate) {
                    Some((ctx, gate)) => {
                        match gate.decrypt(ctx, rtp::ssrc(pkt), pkt, |c, p| c.decrypt_rtp(p).ok()) {
                            Some(dec) => {
                                let mut rp = RtpPacket::new();
                                let n = dec.len().min(rtp::RTP_MAX_LENGTH);
                                rp.as_mut_slice_for_fill(n).copy_from_slice(&dec[..n]);
                                Some(rp)
                            }
                            None => {
                                // bad MAC / replay / untrusted SSRC — drop
                                relay_cfg.own.decrypt_failed.fetch_add(1, Ordering::Relaxed);
                                None
                            }
                        }
                    }
                    None => {
                        // fail closed until keys arrive
                        relay_cfg.own.prekey_dropped.fetch_add(1, Ordering::Relaxed);
                        None
                    }
                }
            } else {
                let mut rp = RtpPacket::new();
                rp.as_mut_slice_for_fill(pkt.len()).copy_from_slice(pkt);
                Some(rp)
            };
            if let Some(rp) = plain {
                // Authenticated — and newer than anything accepted before
                // under this SSRC, so not a captured packet replayed from
                // elsewhere.
                if guarded
                    && relay_cfg
                        .fresh
                        .lock()
                        .rtp(rtp::ssrc(pkt), rtp::sequence_number(pkt))
                {
                    latch(cfg, peer);
                }
                // The source a PLI names (`RxStats::remote_ssrc`) follows
                // only authenticated packets of a codec the leg negotiated:
                // a forged packet, or an RTX / FEC stream under its own SSRC
                // and PT, must not re-point keyframe requests away from the
                // main stream.
                let pts = relay_cfg.own.pts();
                if pts.declares(rp.payload_type()) {
                    rx_account(cfg, rp.as_slice());
                }
                relay_cfg.own.accepted.fetch_add(1, Ordering::Relaxed);
                // Abuse limits (bitrate, packet rate, frame rate, resolution)
                // — after authentication, so only our remote's own packets
                // spend its budget, and before fan-out, so nothing over the
                // limits reaches any receiver. Still accounted above: the
                // PLI a receiver sends must name the source even while its
                // stream is being refused, or it can never recover.
                let verdict = relay_cfg.guard.lock().check(
                    rp.as_slice(),
                    pts.format_of(rp.payload_type()),
                    Instant::now(),
                );
                if verdict != Verdict::Forward {
                    relay_cfg.own.count_refused(verdict);
                    return;
                }
                relay_cfg.own.fan_out(rp);
            }
            return;
        }

        let mut rp = RtpPacket::new();
        rp.as_mut_slice_for_fill(pkt.len()).copy_from_slice(pkt);
        cfg.jitter.lock().push(rp);
    }
}

/// Fold one inbound RTP packet into the receiver accounting.
fn rx_account(cfg: &RecvLoopConfig, pkt: &[u8]) {
    cfg.rx_stats.lock().on_packet_at(
        rtp::ssrc(pkt),
        rtp::sequence_number(pkt),
        rtp::timestamp(pkt),
        Instant::now(),
    );
}

/// Point our outbound at `peer`. On a secure relay leg this is called only
/// for a packet that authenticated (SRTP/SRTCP, or STUN with valid
/// MESSAGE-INTEGRITY) or for DTLS before keys exist: forwarded media goes
/// wherever this points, so latching from any datagram would let a single
/// spoofed packet redirect a keyed leg's media to the spoofer. Plain RTP
/// (and audio) legs keep latching from every packet — there is nothing to
/// authenticate against.
fn latch(cfg: &RecvLoopConfig, peer: SocketAddr) {
    *cfg.remote_addr.lock() = Some(peer);
}

/// Relay only: account one inbound muxed RTCP datagram. `keyed` — an SRTCP
/// decrypt context existed; `ok` — it decrypted (always true when unkeyed).
/// On a secure leg, unkeyed RTCP is unauthenticated and so is a pre-key drop
/// for liveness purposes even though the audio-path accounting still reads
/// it; on a clear leg every well-formed datagram is accepted.
fn count_rtcp(own: &RelayShared, keyed: bool, ok: bool) {
    let counter = if keyed {
        if ok {
            &own.rtcp_in
        } else {
            &own.decrypt_failed
        }
    } else if own.secure.load(Ordering::Relaxed) {
        &own.prekey_dropped
    } else {
        &own.rtcp_in
    };
    counter.fetch_add(1, Ordering::Relaxed);
}

/// Relay only: build the inbound SRTP (RTP, not RTCP) decrypt context once
/// keying material is published. Mirrors `rtcp_loop::maybe_build_decrypt`.
fn maybe_build_rtp_decrypt(
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

/// RFC 5761 demux for the feedback range: RTPFB (205) / PSFB (206). Kept
/// separate from `is_muxed_rtcp` so the audio path's classification is
/// untouched — only relay channels consult this.
fn is_muxed_rtcp_fb(pkt: &[u8]) -> bool {
    pkt.len() >= 2 && (128..=191).contains(&pkt[0]) && (205..=206).contains(&pkt[1])
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

    /// Relay secure branch, driven through `handle_packet` with real SRTCP.
    /// A browser's PLI arrives as an SRTCP-protected `[RR][PLI]` compound —
    /// the shape that lands in the `is_muxed_rtcp` branch. These pin:
    /// fail-closed (no scan of pre-key ciphertext), scanning the decrypted
    /// plaintext rather than the ciphertext, and the walk past the leading
    /// RR, and that the one decrypt feeds BOTH consumers (accounting and
    /// keyframe scan) — each is asserted.
    mod relay_secure {
        use super::super::*;
        use crate::channel::relay::RelayShared;
        use webrtc_srtp::protection_profile::ProtectionProfile;

        const KEY: [u8; 16] = [7u8; 16];
        const SALT: [u8; 14] = [9u8; 14];

        fn keys() -> SrtpKeyingMaterial {
            // local_is_server = true → remote (inbound) params are the
            // client_write pair, which is what the test encrypts with.
            SrtpKeyingMaterial {
                profile: ProtectionProfile::Aes128CmHmacSha1_80,
                client_write_key: KEY.to_vec(),
                client_write_salt: SALT.to_vec(),
                server_write_key: vec![1u8; 16],
                server_write_salt: vec![2u8; 14],
                local_is_server: true,
            }
        }

        /// The far endpoint's outbound SRTCP context.
        fn remote_encryptor() -> webrtc_srtp::context::Context {
            webrtc_srtp::context::Context::new(
                &KEY,
                &SALT,
                ProtectionProfile::Aes128CmHmacSha1_80,
                None,
                None,
            )
            .unwrap()
        }

        /// RR (one report block about our SSRC 0x66, so the accounting
        /// consumer has something observable to fold) followed by a PSFB PLI.
        fn rr_pli() -> Vec<u8> {
            let mut c = vec![0x81u8, 201, 0, 7, 0, 0, 0, 0x55];
            c.extend_from_slice(&[0, 0, 0, 0x66]); // report block: ssrc
            c.extend_from_slice(&[0x40, 0, 0, 3]); // fraction lost, cumulative
            c.extend_from_slice(&[0, 0, 0x12, 0x34]); // ext highest seq
            c.extend_from_slice(&[0, 0, 0, 9]); // jitter
            c.extend_from_slice(&[0u8; 8]); // LSR, DLSR
            c.extend_from_slice(&[0x81u8, 206, 0, 2, 0, 0, 0, 0x55, 0, 0, 0, 0x66]);
            c
        }

        struct Rig {
            cfg: RecvLoopConfig,
            peer: Arc<RelayShared>,
            key_tx: watch::Sender<Option<SrtpKeyingMaterial>>,
        }

        async fn rig() -> Rig {
            let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
            let (key_tx, key_rx) = watch::channel(None);
            let (own_tx, _own_rx) = mpsc::channel(4);
            let (peer_tx, _peer_rx) = mpsc::channel(4);
            let own = Arc::new(RelayShared::new(1, own_tx, true, 96));
            let peer = Arc::new(RelayShared::new(2, peer_tx, true, 96));
            assert!(relay::join(&own, &peer));
            // join asks every member for a keyframe; start the tests clean.
            let _ = pli_requested(&peer).await;
            let cfg = RecvLoopConfig {
                sock,
                jitter: Arc::new(PLMutex::new(JitterBuffer::new(32, 10))),
                remote_addr: Arc::new(PLMutex::new(None)),
                in_count: Arc::new(AtomicU64::new(0)),
                rx_stats: Arc::new(PLMutex::new(RxStats::new(90000))),
                remote_report: Arc::new(PLMutex::new(RemoteReport::default())),
                local_ssrc: 0x66,
                local_icepwd: Arc::new(PLMutex::new(String::new())),
                dtls_tx: Arc::new(PLMutex::new(None)),
                key_rx,
                cancel: CancellationToken::new(),
                relay: Some(RelayRecv::new(own)),
            };
            Rig { cfg, peer, key_tx }
        }

        /// Did the recv loop ask the peer leg's send task for a keyframe?
        async fn pli_requested(peer: &RelayShared) -> bool {
            tokio::time::timeout(std::time::Duration::from_millis(50), peer.pli.notified())
                .await
                .is_ok()
        }

        #[tokio::test]
        async fn compound_pli_over_srtcp_crosses_the_relay() {
            let r = rig().await;
            let from: SocketAddr = "127.0.0.1:9".parse().unwrap();
            let (mut rtcp_ctx, mut rtp_ctx, mut gates) = (None, None, RelayGates::default());
            r.key_tx.send(Some(keys())).unwrap();

            let enc = remote_encryptor().encrypt_rtcp(&rr_pli()).unwrap();
            assert!(
                is_muxed_rtcp(&enc),
                "compound must hit the is_muxed_rtcp branch"
            );
            handle_packet(&r.cfg, &enc, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            assert!(
                pli_requested(&r.peer).await,
                "PLI inside SRTCP compound lost"
            );
            let rr = *r.cfg.remote_report.lock();
            assert!(rr.valid, "RR inside SRTCP compound not folded");
            assert_eq!(rr.ext_highest_seq, 0x1234);
        }

        #[tokio::test]
        async fn bare_pli_over_srtcp_crosses_the_relay() {
            let r = rig().await;
            let from: SocketAddr = "127.0.0.1:9".parse().unwrap();
            let (mut rtcp_ctx, mut rtp_ctx, mut gates) = (None, None, RelayGates::default());
            r.key_tx.send(Some(keys())).unwrap();

            let pli = [0x81u8, 206, 0, 2, 0, 0, 0, 0x55, 0, 0, 0, 0x66];
            let enc = remote_encryptor().encrypt_rtcp(&pli).unwrap();
            handle_packet(&r.cfg, &enc, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            assert!(pli_requested(&r.peer).await, "bare SRTCP PLI lost");
        }

        #[tokio::test]
        async fn pre_key_rtcp_is_not_scanned_on_a_secure_leg() {
            let r = rig().await;
            let from: SocketAddr = "127.0.0.1:9".parse().unwrap();
            let (mut rtcp_ctx, mut rtp_ctx, mut gates) = (None, None, RelayGates::default());
            // No keys published. Even a datagram that *would* classify as a
            // keyframe request if read as plaintext must not trigger one —
            // on a secure leg, pre-key bytes are ciphertext.
            handle_packet(
                &r.cfg,
                &rr_pli(),
                from,
                &mut rtcp_ctx,
                &mut rtp_ctx,
                &mut gates,
            )
            .await;
            assert!(!pli_requested(&r.peer).await, "scanned pre-key compound");
            let pli = [0x81u8, 206, 0, 2, 0, 0, 0, 0x55, 0, 0, 0, 0x66];
            handle_packet(&r.cfg, &pli, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            assert!(!pli_requested(&r.peer).await, "scanned pre-key feedback");
        }

        /// webrtc-srtp panics on SRTCP shorter than its trailer. One such
        /// datagram used to kill a keyed leg's receive task (and with it the
        /// leg's media) - it must just be dropped as a failed decrypt.
        #[tokio::test]
        async fn short_srtcp_is_dropped_not_panicked() {
            let r = rig().await;
            let own = r.cfg.relay.as_ref().unwrap().own.clone();
            let from: SocketAddr = "127.0.0.1:9".parse().unwrap();
            let (mut rtcp_ctx, mut rtp_ctx, mut gates) = (None, None, RelayGates::default());
            r.key_tx.send(Some(keys())).unwrap();

            let short: [&[u8]; 4] = [
                &[0x80, 206, 0, 0],
                &[0x81, 205, 0, 0, 1, 2],
                &[0x80, 200, 0, 0],
                // one byte under the AES-CM-80 minimum (8 + 4 + 10)
                &[
                    0x80, 201, 0, 1, 0, 0, 0, 0x55, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                ],
            ];
            for pkt in short {
                handle_packet(&r.cfg, pkt, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            }
            assert_eq!(own.snapshot().decrypt_failed, short.len() as u64);

            // ...and the leg still takes a genuine packet afterwards.
            let enc = remote_encryptor().encrypt_rtcp(&rr_pli()).unwrap();
            handle_packet(&r.cfg, &enc, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            assert!(pli_requested(&r.peer).await, "leg dead after short SRTCP");

            // The P+1 loop and the audio branch share the checked decrypt;
            // AEAD profiles have their own (longer) trailer.
            for profile in [
                ProtectionProfile::Aes128CmHmacSha1_80,
                ProtectionProfile::AeadAes128Gcm,
            ] {
                let salt = vec![9u8; profile.salt_len()];
                let mut ctx =
                    webrtc_srtp::context::Context::new(&KEY, &salt, profile, None, None).unwrap();
                for len in 0..rtcp_loop::srtcp_min_len(profile) {
                    let mut pkt = vec![0u8; len];
                    if len > 1 {
                        pkt[0] = 0x80;
                        pkt[1] = 200;
                    }
                    assert!(
                        rtcp_loop::decrypt_rtcp_checked(&mut ctx, &pkt, Some(profile)).is_none()
                    );
                }
            }
        }

        /// SRTCP with its E flag clear: webrtc-srtp hands it back as plaintext
        /// without checking the auth tag, so with no key at all a forged
        /// compound would count as authenticated - moving the leg's media to
        /// the spoofer, forwarding its PLI, and (under new sender SSRCs)
        /// filling the RTCP gate so the browser's real feedback is locked out.
        #[tokio::test]
        async fn srtcp_with_the_e_flag_clear_is_not_authenticated() {
            let r = rig().await;
            let own = r.cfg.relay.as_ref().unwrap().own.clone();
            let real: SocketAddr = "127.0.0.1:9".parse().unwrap();
            let spoof: SocketAddr = "127.0.0.1:6666".parse().unwrap();
            let (mut rtcp_ctx, mut rtp_ctx, mut gates) = (None, None, RelayGates::default());
            r.key_tx.send(Some(keys())).unwrap();
            let mut enc = remote_encryptor();
            let mut rtp = vec![0x80u8, 96, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0x55];
            rtp.extend_from_slice(&[0xAB; 20]);
            let good = enc.encrypt_rtp(&rtp).unwrap();
            handle_packet(&r.cfg, &good, real, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            assert_eq!(*r.cfg.remote_addr.lock(), Some(real));
            let _ = pli_requested(&r.peer).await;

            // plaintext compound + E=0 index word + 10 junk "tag" bytes, no key
            let forge = |plain: &[u8], ssrc: u32| {
                let mut p = plain.to_vec();
                p[4..8].copy_from_slice(&ssrc.to_be_bytes());
                p.extend_from_slice(&[0, 0, 0, 1]);
                p.extend_from_slice(&[0x5A; 10]);
                p
            };
            let failed = own.snapshot().decrypt_failed;
            let bare_pli = [0x81u8, 206, 0, 2, 0, 0, 0, 0x55, 0, 0, 0, 0x66];
            for i in 0..100u32 {
                for plain in [&rr_pli()[..], &bare_pli[..]] {
                    let pkt = forge(plain, 0x1000 + i);
                    handle_packet(&r.cfg, &pkt, spoof, &mut rtcp_ctx, &mut rtp_ctx, &mut gates)
                        .await;
                }
            }
            assert_eq!(
                *r.cfg.remote_addr.lock(),
                Some(real),
                "E=0 SRTCP moved the remote"
            );
            assert!(
                !pli_requested(&r.peer).await,
                "E=0 SRTCP PLI crossed the relay"
            );
            assert_eq!(own.snapshot().decrypt_failed, failed + 200);
            assert_eq!(own.snapshot().rtcp_in, 0);

            // ...and the browser's genuine SRTCP still gets through.
            let genuine = enc.encrypt_rtcp(&rr_pli()).unwrap();
            handle_packet(
                &r.cfg,
                &genuine,
                real,
                &mut rtcp_ctx,
                &mut rtp_ctx,
                &mut gates,
            )
            .await;
            assert!(pli_requested(&r.peer).await, "genuine SRTCP locked out");
            assert_eq!(own.snapshot().rtcp_in, 1);
        }

        /// Forged SRTP under ever-new SSRCs must never create state in the
        /// leg's real context (webrtc-srtp keeps it forever - unbounded
        /// memory); only an SSRC that authenticates gets there.
        #[test]
        fn ssrc_gate_only_hands_authenticated_ssrcs_to_the_real_context() {
            let mut gate = SsrcGate::new(&keys());
            let mut real = remote_encryptor(); // same keys: a decrypt context
            let real_ptr: *const webrtc_srtp::context::Context = &real;
            let calls = std::cell::Cell::new(0u32);
            let dec = |c: &mut webrtc_srtp::context::Context, p: &[u8]| {
                if std::ptr::eq(c, real_ptr) {
                    calls.set(calls.get() + 1);
                }
                c.decrypt_rtp(p).ok()
            };

            for i in 0..10_000u32 {
                let mut pkt = vec![0x80u8, 96, 0, 1, 0, 0, 0, 1];
                pkt.extend_from_slice(&i.to_be_bytes());
                pkt.extend_from_slice(&[0xAB; 30]);
                assert!(gate.decrypt(&mut real, i, &pkt, dec).is_none());
            }
            assert_eq!(calls.get(), 0, "a forged SSRC reached the real context");
            assert!(gate.trusted.is_empty());
            assert!(gate.probed.len() <= GATE_MAX_PROBED);

            let mut enc = remote_encryptor();
            let mut rtp = vec![0x80u8, 96, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0x55];
            rtp.extend_from_slice(&[0xCD; 20]);
            let first = enc.encrypt_rtp(&rtp).unwrap();
            assert_eq!(
                &gate.decrypt(&mut real, 0x55, &first, dec).unwrap()[..],
                &rtp[..]
            );
            assert!(gate.trusted.contains(&0x55));
            rtp[3] = 2;
            let second = enc.encrypt_rtp(&rtp).unwrap();
            assert_eq!(
                &gate.decrypt(&mut real, 0x55, &second, dec).unwrap()[..],
                &rtp[..]
            );
            // the first packet once (after its probe), then straight through
            assert_eq!(calls.get(), 2);
        }

        /// The four inbound outcomes land in separate counters, and only the
        /// authenticated ones in `liveness()`.
        #[tokio::test]
        async fn secure_leg_counts_prekey_decrypt_failure_and_accepted() {
            let r = rig().await;
            let own = r.cfg.relay.as_ref().unwrap().own.clone();
            let from: SocketAddr = "127.0.0.1:9".parse().unwrap();
            let (mut rtcp_ctx, mut rtp_ctx, mut gates) = (None, None, RelayGates::default());
            let mut rtp = vec![0x80u8, 96, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0x55];
            rtp.extend_from_slice(&[0xAB; 20]);

            // Before keys: RTP and RTCP are pre-key drops.
            handle_packet(&r.cfg, &rtp, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            handle_packet(
                &r.cfg,
                &rr_pli(),
                from,
                &mut rtcp_ctx,
                &mut rtp_ctx,
                &mut gates,
            )
            .await;
            let c = own.snapshot();
            assert_eq!((c.in_count, c.prekey_dropped, c.accepted), (1, 2, 0));
            assert_eq!(own.liveness(), 0);

            r.key_tx.send(Some(keys())).unwrap();
            // Keys present but the packet was not protected with them.
            let bogus = webrtc_srtp::context::Context::new(
                &[3u8; 16],
                &SALT,
                ProtectionProfile::Aes128CmHmacSha1_80,
                None,
                None,
            )
            .unwrap()
            .encrypt_rtp(&rtp)
            .unwrap();
            handle_packet(
                &r.cfg,
                &bogus,
                from,
                &mut rtcp_ctx,
                &mut rtp_ctx,
                &mut gates,
            )
            .await;
            assert_eq!(own.snapshot().decrypt_failed, 1);
            assert_eq!(own.liveness(), 0);

            // Properly protected RTP and SRTCP are accepted.
            let mut enc = remote_encryptor();
            let good = enc.encrypt_rtp(&rtp).unwrap();
            handle_packet(&r.cfg, &good, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            let rtcp = enc.encrypt_rtcp(&rr_pli()).unwrap();
            handle_packet(&r.cfg, &rtcp, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            let c = own.snapshot();
            assert_eq!((c.accepted, c.rtcp_in, c.decrypt_failed), (1, 1, 1));
            assert_eq!(own.liveness(), 2);
        }

        /// A keyed secure leg must not move its outbound address for a
        /// datagram that fails to authenticate — forwarded media follows it.
        #[tokio::test]
        async fn secure_leg_latches_remote_only_from_authenticated_packets() {
            let r = rig().await;
            let real: SocketAddr = "127.0.0.1:9".parse().unwrap();
            let spoof: SocketAddr = "127.0.0.1:6666".parse().unwrap();
            let (mut rtcp_ctx, mut rtp_ctx, mut gates) = (None, None, RelayGates::default());
            let mut rtp = vec![0x80u8, 96, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0x55];
            rtp.extend_from_slice(&[0xAB; 20]);
            let remote = || *r.cfg.remote_addr.lock();

            // Pre-key: DTLS may latch (the handshake needs an answer
            // address); unauthenticatable RTP may not.
            handle_packet(
                &r.cfg,
                &[22u8, 0xfe, 0xfd, 0],
                real,
                &mut rtcp_ctx,
                &mut rtp_ctx,
                &mut gates,
            )
            .await;
            assert_eq!(remote(), Some(real));
            handle_packet(&r.cfg, &rtp, spoof, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            assert_eq!(remote(), Some(real), "pre-key RTP moved the remote");

            r.key_tx.send(Some(keys())).unwrap();
            let mut enc = remote_encryptor();
            handle_packet(
                &r.cfg,
                &enc.encrypt_rtp(&rtp).unwrap(),
                real,
                &mut rtcp_ctx,
                &mut rtp_ctx,
                &mut gates,
            )
            .await;
            assert_eq!(remote(), Some(real));

            // Spoofed plaintext RTP, forged SRTP/SRTCP, and post-key DTLS.
            let bogus = webrtc_srtp::context::Context::new(
                &[3u8; 16],
                &SALT,
                ProtectionProfile::Aes128CmHmacSha1_80,
                None,
                None,
            )
            .unwrap();
            let mut bogus = bogus;
            rtp[3] = 2;
            let forged_rtp = bogus.encrypt_rtp(&rtp).unwrap();
            let forged_rtcp = bogus.encrypt_rtcp(&rr_pli()).unwrap();
            let forged_pli = bogus
                .encrypt_rtcp(&[0x81u8, 206, 0, 2, 0, 0, 0, 0x55, 0, 0, 0, 0x66])
                .unwrap();
            for pkt in [
                rtp.clone(),
                forged_rtp.to_vec(),
                forged_rtcp.to_vec(),
                forged_pli.to_vec(),
                vec![22u8, 0xfe, 0xfd, 0],
            ] {
                handle_packet(&r.cfg, &pkt, spoof, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
                assert_eq!(
                    remote(),
                    Some(real),
                    "unauthenticated {:?} moved the remote",
                    &pkt[..2]
                );
            }

            // An authenticated packet from a new address does move it (the
            // browser's network changed).
            let moved: SocketAddr = "127.0.0.1:7777".parse().unwrap();
            handle_packet(
                &r.cfg,
                &enc.encrypt_rtcp(&rr_pli()).unwrap(),
                moved,
                &mut rtcp_ctx,
                &mut rtp_ctx,
                &mut gates,
            )
            .await;
            assert_eq!(remote(), Some(moved));
        }

        /// STUN request signed with `pwd` (MESSAGE-INTEGRITY), or bare
        /// (header only) when `pwd` is `None`. `txid` fills the transaction
        /// ID; `nominate` adds USE-CANDIDATE inside the signed part.
        fn stun_request(pwd: Option<&[u8]>, txid: u8, nominate: bool) -> Vec<u8> {
            use hmac::{Hmac, Mac};
            let mut pk = vec![0u8; 20];
            pk[1] = 0x01; // Binding Request
            pk[4..8].copy_from_slice(&0x2112_A442u32.to_be_bytes());
            pk[8..20].copy_from_slice(&[txid; 12]);
            let Some(pwd) = pwd else { return pk };
            if nominate {
                pk.extend_from_slice(&[0x00, 0x25, 0, 0]);
            }
            let mi_off = pk.len();
            pk.extend_from_slice(&[0x00, 0x08, 0, 20]);
            let len = (mi_off - 20 + 24) as u16;
            pk[2..4].copy_from_slice(&len.to_be_bytes());
            let mut mac = <Hmac<sha1::Sha1> as Mac>::new_from_slice(pwd).unwrap();
            mac.update(&pk[..mi_off]);
            pk.extend_from_slice(&mac.finalize().into_bytes());
            pk
        }

        const PWD: &[u8] = b"icepwd-secret";

        /// Reviewer's probe: a bare 20-byte Binding Request from anywhere
        /// used to be answered and then latched the remote of a keyed leg.
        /// Only a request carrying valid MESSAGE-INTEGRITY may move it —
        /// and a fresh nomination must still move it.
        #[tokio::test]
        async fn secure_leg_latches_only_integrity_checked_stun() {
            let r = rig().await;
            *r.cfg.local_icepwd.lock() = "icepwd-secret".into();
            let real: SocketAddr = "127.0.0.1:9".parse().unwrap();
            let spoof: SocketAddr = "127.0.0.1:6666".parse().unwrap();
            let (mut rtcp_ctx, mut rtp_ctx, mut gates) = (None, None, RelayGates::default());
            let remote = || *r.cfg.remote_addr.lock();
            let signed = stun_request(Some(PWD), 1, true);
            handle_packet(
                &r.cfg,
                &signed,
                real,
                &mut rtcp_ctx,
                &mut rtp_ctx,
                &mut gates,
            )
            .await;
            assert_eq!(remote(), Some(real));

            for pkt in [
                stun_request(None, 2, false),
                stun_request(Some(b"wrong"), 3, true),
            ] {
                handle_packet(&r.cfg, &pkt, spoof, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
                assert_eq!(
                    remote(),
                    Some(real),
                    "unauthenticated STUN moved the remote"
                );
            }

            let moved: SocketAddr = "127.0.0.1:7777".parse().unwrap();
            handle_packet(
                &r.cfg,
                &stun_request(Some(PWD), 4, true),
                moved,
                &mut rtcp_ctx,
                &mut rtp_ctx,
                &mut gates,
            )
            .await;
            assert_eq!(remote(), Some(moved), "fresh nomination must re-latch");
        }

        /// Reviewer's probe: a captured signed request re-sent verbatim from
        /// another address verified and latched there. A replay must not
        /// move the remote, nor may a signed check that does not nominate
        /// (a consent or backup-pair ping) — but it is still answered.
        #[tokio::test]
        async fn secure_leg_stun_latch_needs_fresh_nomination() {
            let r = rig().await;
            *r.cfg.local_icepwd.lock() = "icepwd-secret".into();
            let peer_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let real = peer_sock.local_addr().unwrap();
            let spoof_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let spoof = spoof_sock.local_addr().unwrap();
            let (mut rtcp_ctx, mut rtp_ctx, mut gates) = (None, None, RelayGates::default());
            let remote = || *r.cfg.remote_addr.lock();

            // Before any remote exists a signed check latches even without
            // USE-CANDIDATE, so ICE/DTLS can start (regular nomination).
            let first = stun_request(Some(PWD), 9, false);
            handle_packet(
                &r.cfg,
                &first,
                real,
                &mut rtcp_ctx,
                &mut rtp_ctx,
                &mut gates,
            )
            .await;
            assert_eq!(remote(), Some(real), "first signed check must latch");

            let captured = stun_request(Some(PWD), 1, true);
            handle_packet(
                &r.cfg,
                &captured,
                real,
                &mut rtcp_ctx,
                &mut rtp_ctx,
                &mut gates,
            )
            .await;
            assert_eq!(remote(), Some(real));

            handle_packet(
                &r.cfg,
                &captured,
                spoof,
                &mut rtcp_ctx,
                &mut rtp_ctx,
                &mut gates,
            )
            .await;
            assert_eq!(remote(), Some(real), "replayed signed STUN latched");

            let ping = stun_request(Some(PWD), 2, false);
            handle_packet(
                &r.cfg,
                &ping,
                spoof,
                &mut rtcp_ctx,
                &mut rtp_ctx,
                &mut gates,
            )
            .await;
            assert_eq!(remote(), Some(real), "non-nominating ping latched");
            let mut buf = [0u8; 256];
            let got = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                spoof_sock.recv_from(&mut buf),
            )
            .await;
            assert!(got.is_ok(), "signed ping must still be answered");

            handle_packet(
                &r.cfg,
                &stun_request(Some(PWD), 3, true),
                spoof,
                &mut rtcp_ctx,
                &mut rtp_ctx,
                &mut gates,
            )
            .await;
            assert_eq!(remote(), Some(spoof), "fresh nomination must latch");
        }

        /// Reviewer's probe: the inbound contexts have no replay protection,
        /// so a captured authentic SRTP/SRTCP packet replayed from another
        /// address decrypted and latched the remote there. A replay (not
        /// newer than what the SSRC already delivered) must not move it; a
        /// genuinely new packet from a new address still must.
        #[tokio::test]
        async fn secure_leg_does_not_latch_on_a_replayed_packet() {
            let r = rig().await;
            let real: SocketAddr = "127.0.0.1:9".parse().unwrap();
            let spoof: SocketAddr = "127.0.0.1:6666".parse().unwrap();
            let (mut rtcp_ctx, mut rtp_ctx, mut gates) = (None, None, RelayGates::default());
            let remote = || *r.cfg.remote_addr.lock();
            r.key_tx.send(Some(keys())).unwrap();
            let mut enc = remote_encryptor();
            let rtp_sn = |sn: u16| {
                let mut p = vec![0x80u8, 96, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0x55];
                p[2..4].copy_from_slice(&sn.to_be_bytes());
                p.extend_from_slice(&[0xAB; 20]);
                p
            };
            let mut sent_rtp = Vec::new();
            for sn in 1..=3u16 {
                let e = enc.encrypt_rtp(&rtp_sn(sn)).unwrap().to_vec();
                handle_packet(&r.cfg, &e, real, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
                sent_rtp.push(e);
            }
            let rtcp1 = enc.encrypt_rtcp(&rr_pli()).unwrap().to_vec();
            let rtcp2 = enc.encrypt_rtcp(&rr_pli()).unwrap().to_vec();
            let fb1 = enc
                .encrypt_rtcp(&[0x81u8, 206, 0, 2, 0, 0, 0, 0x55, 0, 0, 0, 0x66])
                .unwrap()
                .to_vec();
            for p in [&rtcp1, &rtcp2, &fb1] {
                handle_packet(&r.cfg, p, real, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            }
            assert_eq!(remote(), Some(real));

            // Replays of old and of the latest packets, RTP and SRTCP
            // (muxed-RTCP branch and feedback branch) — all authenticate.
            let before = r.cfg.relay.as_ref().unwrap().own.snapshot();
            for p in sent_rtp.iter().chain([&rtcp1, &rtcp2, &fb1]) {
                handle_packet(&r.cfg, p, spoof, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
                assert_eq!(
                    remote(),
                    Some(real),
                    "replayed {:?} moved the remote",
                    &p[..2]
                );
            }
            let after = r.cfg.relay.as_ref().unwrap().own.snapshot();
            assert_eq!(
                after.accepted + after.rtcp_in,
                before.accepted + before.rtcp_in + 6,
                "replays must authenticate, or this test proves nothing"
            );

            // Fresh traffic from a new address (NAT rebinding) still moves it,
            // via RTP and via SRTCP.
            let moved: SocketAddr = "127.0.0.1:7777".parse().unwrap();
            let e = enc.encrypt_rtp(&rtp_sn(4)).unwrap();
            handle_packet(&r.cfg, &e, moved, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            assert_eq!(remote(), Some(moved));
            let moved2: SocketAddr = "127.0.0.1:7778".parse().unwrap();
            let e = enc.encrypt_rtcp(&rr_pli()).unwrap();
            handle_packet(&r.cfg, &e, moved2, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            assert_eq!(remote(), Some(moved2));
        }

        #[test]
        fn freshness_is_wrap_aware_and_per_ssrc() {
            let mut f = Freshness::default();
            assert!(f.rtp(1, 65534));
            assert!(f.rtp(1, 2)); // across the wrap
            assert!(!f.rtp(1, 65535)); // older, late
            assert!(!f.rtp(1, 2)); // duplicate
            assert!(f.rtp(2, 0)); // another SSRC has its own series
            assert!(f.rtcp(1, 5));
            assert!(!f.rtcp(1, 5));
            assert!(!f.rtcp(1, 4));
            assert!(f.rtcp(1, 6));
        }

        /// A plain (non-secure) relay leg keeps symmetric-RTP latching.
        #[tokio::test]
        async fn clear_leg_latches_from_any_packet() {
            let mut r = rig().await;
            r.cfg
                .relay
                .as_ref()
                .unwrap()
                .own
                .secure
                .store(false, Ordering::Relaxed);
            let from: SocketAddr = "127.0.0.1:4242".parse().unwrap();
            let (mut rtcp_ctx, mut rtp_ctx, mut gates) = (None, None, RelayGates::default());
            let rtp = vec![0x80u8, 96, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0x55];
            handle_packet(&r.cfg, &rtp, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            assert_eq!(*r.cfg.remote_addr.lock(), Some(from));
            r.cfg.relay = None; // and an audio leg
            let other: SocketAddr = "127.0.0.1:4343".parse().unwrap();
            handle_packet(&r.cfg, &rtp, other, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            assert_eq!(*r.cfg.remote_addr.lock(), Some(other));
        }

        /// The SSRC a relay leg's PLI names (`RxStats::remote_ssrc`) used to
        /// follow every inbound datagram, before any check: two packets of an
        /// RTX / FEC stream (its own SSRC and an un-negotiated PT), or two
        /// forged packets on a secure leg, re-pointed keyframe requests away
        /// from the main stream. Only accepted packets of a negotiated PT
        /// count now.
        #[tokio::test]
        async fn pli_target_follows_only_accepted_negotiated_packets() {
            let r = rig().await;
            let from: SocketAddr = "127.0.0.1:9".parse().unwrap();
            let (mut rtcp_ctx, mut rtp_ctx, mut gates) = (None, None, RelayGates::default());
            let pkt = |pt: u8, sn: u8, ssrc: u8| {
                let mut p = vec![0x80u8, pt, 0, sn, 0, 0, 0, 1, 0, 0, 0, ssrc];
                p.extend_from_slice(&[0xAB; 20]);
                p
            };
            // Secure leg: forged packets (keys present, no valid auth) under
            // a new SSRC do not latch the PLI target.
            r.key_tx.send(Some(keys())).unwrap();
            let mut enc = remote_encryptor();
            let e = enc.encrypt_rtp(&pkt(96, 1, 0x55)).unwrap();
            handle_packet(&r.cfg, &e, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            assert_eq!(r.cfg.rx_stats.lock().remote_ssrc, Some(0x55));
            for sn in 1..=3 {
                let forged = pkt(96, sn, 0x77);
                handle_packet(
                    &r.cfg,
                    &forged,
                    from,
                    &mut rtcp_ctx,
                    &mut rtp_ctx,
                    &mut gates,
                )
                .await;
            }
            assert_eq!(
                r.cfg.rx_stats.lock().remote_ssrc,
                Some(0x55),
                "forged SSRC latched"
            );
            // An authentic second stream under a PT the leg did not
            // negotiate (RTX, say) is forwarded but not the PLI target.
            for sn in 1..=3 {
                let e = enc.encrypt_rtp(&pkt(97, sn, 0x99)).unwrap();
                handle_packet(&r.cfg, &e, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            }
            assert_eq!(
                r.cfg.rx_stats.lock().remote_ssrc,
                Some(0x55),
                "RTX SSRC latched"
            );
            // A real restart under the negotiated PT still relatches.
            for sn in 1..=3 {
                let e = enc.encrypt_rtp(&pkt(96, sn, 0x44)).unwrap();
                handle_packet(&r.cfg, &e, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            }
            assert_eq!(r.cfg.rx_stats.lock().remote_ssrc, Some(0x44));
        }

        #[tokio::test]
        async fn secure_leg_drops_inbound_rtp_until_keys() {
            let r = rig().await;
            let from: SocketAddr = "127.0.0.1:9".parse().unwrap();
            let (mut rtcp_ctx, mut rtp_ctx, mut gates) = (None, None, RelayGates::default());
            let (tx, mut rx) = mpsc::channel(4);
            let peer = Arc::new(RelayShared::new(3, tx, false, 96));
            assert!(relay::join(&r.cfg.relay.as_ref().unwrap().own, &peer));

            let mut rtp = vec![0x80u8, 96, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0x55];
            rtp.extend_from_slice(&[0xAB; 20]);
            handle_packet(&r.cfg, &rtp, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            assert!(
                rx.try_recv().is_err(),
                "forwarded pre-key (unauthenticated) RTP"
            );

            r.key_tx.send(Some(keys())).unwrap();
            let enc = remote_encryptor().encrypt_rtp(&rtp).unwrap();
            handle_packet(&r.cfg, &enc, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            let got = rx.try_recv().expect("decrypted RTP not forwarded");
            assert_eq!(got.src, 1);
            assert_eq!(
                got.pkt.as_slice(),
                &rtp[..],
                "forwarded bytes are not the plaintext"
            );
        }

        /// The abuse limits sit between authentication and fan-out: forged
        /// packets never spend a leg's budget, a packet over it reaches no
        /// receiver and is counted, and an oversized keyframe is refused
        /// while the PLI target still follows its stream (so the receiver's
        /// keyframe request can reach it and recover the picture).
        #[tokio::test]
        async fn abuse_limits_apply_to_authenticated_packets_before_fan_out() {
            use crate::channel::relay::PtMap;
            use crate::channel::relay_limits::RelayLimits;
            use crate::channel::video_dims::tests::vp8_payload;
            let mut r = rig().await;
            let from: SocketAddr = "127.0.0.1:9".parse().unwrap();
            let (mut rtcp_ctx, mut rtp_ctx, mut gates) = (None, None, RelayGates::default());
            let (own_tx, _own_rx) = mpsc::channel(4);
            let own = Arc::new(
                RelayShared::new(10, own_tx, true, 96).with_limits(RelayLimits {
                    packetrate: 3,
                    ..RelayLimits::DEFAULT
                }),
            );
            own.set_pts(PtMap::new(96, [("vp8".to_string(), 96)]));
            let (tx, mut rx) = mpsc::channel(16);
            let peer = Arc::new(RelayShared::new(11, tx, false, 96));
            assert!(relay::join(&own, &peer));
            r.cfg.relay = Some(RelayRecv::new(own.clone()));
            r.key_tx.send(Some(keys())).unwrap();
            let mut enc = remote_encryptor();
            let pkt = |sn: u8, ts: u8, payload: &[u8]| {
                let mut p = vec![0x80u8, 96, 0, sn, 0, 0, 0, ts, 0, 0, 0, 0x55];
                p.extend_from_slice(payload);
                p
            };
            let inter = vp8_payload(false, true, 0, 0);

            // Forged packets: refused by SRTP, so they cost the leg nothing.
            for sn in 1..=10 {
                let forged = pkt(sn, 1, &inter);
                handle_packet(
                    &r.cfg,
                    &forged,
                    from,
                    &mut rtcp_ctx,
                    &mut rtp_ctx,
                    &mut gates,
                )
                .await;
            }
            // 1080p keyframe: authenticated, refused, counted — and still
            // the stream a PLI names.
            let big = enc
                .encrypt_rtp(&pkt(1, 2, &vp8_payload(true, true, 1920, 1080)))
                .unwrap();
            handle_packet(&r.cfg, &big, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            assert!(rx.try_recv().is_err(), "oversized keyframe forwarded");
            let c = own.snapshot();
            assert_eq!(
                (c.accepted, c.oversize_dropped, c.decrypt_failed),
                (1, 1, 10)
            );
            assert_eq!(r.cfg.rx_stats.lock().remote_ssrc, Some(0x55));

            // 720p keyframe then interframes: 3 packets/s admits exactly 3.
            let ok = enc
                .encrypt_rtp(&pkt(2, 3, &vp8_payload(true, true, 1280, 720)))
                .unwrap();
            handle_packet(&r.cfg, &ok, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            for sn in 3..=6 {
                let e = enc.encrypt_rtp(&pkt(sn, 4, &inter)).unwrap();
                handle_packet(&r.cfg, &e, from, &mut rtcp_ctx, &mut rtp_ctx, &mut gates).await;
            }
            let mut forwarded = 0;
            while rx.try_recv().is_ok() {
                forwarded += 1;
            }
            assert_eq!(forwarded, 3);
            let c = own.snapshot();
            assert_eq!((c.rate_dropped, c.oversize_dropped, c.accepted), (2, 1, 6));
        }
    }

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
