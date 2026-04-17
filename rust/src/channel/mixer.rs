// Mix group actor — owns every member's ChannelState + Subsystems for the
// duration of the mix, and runs their tick pipeline in lockstep under a
// single 20 ms ticker.
//
// This replaces the earlier "each channel keeps its own ticker and shares a
// Mutex<MixGroupShared>" arrangement, which suffered a deposit/emit race:
// channel X's tick could fire between peer Y's deposit and its own frame
// update, so X's summed_minus() read stale peer audio. Migrating state in
// lets us do everything in order per tick:
//
//   Phase 1: per-member — drain UDP, classify, push to jitter. For N=2 this
//            phase also implements the byte-relay fast path (packet in →
//            transcode-and-forward to the peer immediately, no mix math).
//   Phase 2: per-member — pop jitter, decode, advance player, feed recorders,
//            run barge-in detection, populate `frame` (N≥3 mix path).
//   Phase 3: per-member — compute outbound.
//            • N=2: byte relay already emitted in Phase 1 — here we only
//              handle DTMF send/relay queues.
//            • N≥3: summed-minus-own, encode to own remote_pt, send.
//   Phase 4: fan out DTMF digits detected in Phase 1 into peers'
//            `dtmf_relay` so each peer regenerates a burst on its own remote.
//   Phase 5: drain `state.pending_events` through each member's EventSink.
//
// Member state only exits the mixer on `Remove` (which the channel actor
// issues on `Close` or `LeaveMix`).

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio::time::{interval, MissedTickBehavior};

use super::actor::{Event, EventSink, PlayState, RecordState, Subsystems, TICK_MS};
use super::commands::{ChannelId, Command};
use super::player::Player;
use super::recorder::{FinishReason, Recorder, RecorderState};
use super::rtp::{self, RtpPacket};
use super::state::ChannelState;
use crate::stun;

pub const MIX_FRAME_SAMPLES: usize = 160;

/// One channel currently living inside a mix group. Moves into the mixer on
/// `EnterMix` and out again on `LeaveMix` / close.
pub struct Member {
    pub state: Box<ChannelState>,
    pub subs: Subsystems,
    /// Per-channel event sink — mixer drains `state.pending_events` through
    /// this after each tick so JS sees the same event stream as when Local.
    pub events: Arc<dyn EventSink>,
    /// Decoded PCM samples this member contributes to the mix this tick.
    /// Zero at the start of each tick; populated from player (preferred) or
    /// inbound RTP (fallback) during Phase 2. `frame_present` tracks whether
    /// the frame is real audio vs. silence — used by N≥3 summed builder.
    pub frame: Vec<i16>,
    pub frame_present: bool,
}

impl Member {
    pub fn new(
        state: Box<ChannelState>,
        subs: Subsystems,
        events: Arc<dyn EventSink>,
    ) -> Self {
        Self {
            state,
            subs,
            events,
            frame: vec![0; MIX_FRAME_SAMPLES],
            frame_present: false,
        }
    }
}

pub enum MixerCommand {
    Add { member: Box<Member>, ack: oneshot::Sender<()> },
    Remove { id: ChannelId, ack: oneshot::Sender<Option<Box<Member>>> },
    Forward { id: ChannelId, cmd: Command },
    Stop,
}

static NEXT_MIX_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct MixHandle {
    pub id: u64,
    pub cmd: mpsc::Sender<MixerCommand>,
}

impl MixHandle {
    pub async fn add(&self, member: Box<Member>) -> Result<(), ()> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(MixerCommand::Add { member, ack: tx }).await.map_err(|_| ())?;
        rx.await.map_err(|_| ())
    }

    pub async fn remove(&self, id: ChannelId) -> Option<Box<Member>> {
        let (tx, rx) = oneshot::channel();
        self.cmd.send(MixerCommand::Remove { id, ack: tx }).await.ok()?;
        rx.await.ok().flatten()
    }

    pub async fn forward(&self, id: ChannelId, cmd: Command) {
        let _ = self.cmd.send(MixerCommand::Forward { id, cmd }).await;
    }

    pub async fn stop(&self) {
        let _ = self.cmd.send(MixerCommand::Stop).await;
    }
}

pub fn spawn() -> MixHandle {
    let id = NEXT_MIX_ID.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel::<MixerCommand>(64);
    tokio::spawn(run(rx));
    MixHandle { id, cmd: tx }
}

async fn run(mut cmds: mpsc::Receiver<MixerCommand>) {
    let mut members: HashMap<ChannelId, Box<Member>> = HashMap::new();
    let mut ticker = interval(Duration::from_millis(TICK_MS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut stopped = false;

    while !stopped {
        tokio::select! {
            biased;

            cmd = cmds.recv() => match cmd {
                None => stopped = true,
                Some(MixerCommand::Stop) => stopped = true,
                Some(MixerCommand::Add { member, ack }) => {
                    members.insert(member.state.id, member);
                    let _ = ack.send(());
                }
                Some(MixerCommand::Remove { id, ack }) => {
                    let m = members.remove(&id);
                    let _ = ack.send(m);
                    // Stop when empty — the facade drops its handle alongside,
                    // so no-one else is holding a sender to us.
                    if members.is_empty() {
                        stopped = true;
                    }
                }
                Some(MixerCommand::Forward { id, cmd }) => {
                    if let Some(m) = members.get_mut(&id) {
                        apply_forwarded(m, cmd).await;
                    }
                }
            },

            _ = ticker.tick() => {
                mix_tick(&mut members).await;
            }
        }
    }
}

async fn mix_tick(members: &mut HashMap<ChannelId, Box<Member>>) {
    // Snapshot alive-member count. Branch between 2-chan byte relay and
    // N≥3 summed mix. One map lookup per tick; matches C++ mix2 vs mixall.
    let n_alive = members.len();
    let use_relay = n_alive == 2;

    // ----- Phase 1: drain inbound on every member. -----
    // A two-member group gets byte-relay forwarding inline. Collect DTMF
    // origin+digit tuples for Phase 4 broadcasting. Sequential per member
    // so classify_and_route_member() can hold a single &mut borrow.
    let mut dtmf_broadcast: Vec<(ChannelId, char)> = Vec::new();
    if use_relay {
        // Cross-configure iLBC dynamic PTs so each member's transcoder
        // recognises the peer's wire PT for iLBC encode/decode. Only applies
        // when the peer uses a dynamic PT (≥96), otherwise standard codecs
        // don't need this mapping. Cheap — just sets a field.
        {
            let pts: Vec<(ChannelId, u8)> = members.iter()
                .map(|(&id, m)| (id, m.state.remote_pt))
                .collect();
            for (&id, m) in members.iter_mut() {
                if let Some(&(_, peer_pt)) = pts.iter().find(|(pid, _)| *pid != id) {
                    if peer_pt >= 96 {
                        m.state.transcoder.set_peer_ilbc_pt(peer_pt);
                    }
                }
            }
        }
        // Peer's outbound target for the relay — snapshot before the mutable
        // iteration since we need all peer addresses upfront.
        let peers: HashMap<ChannelId, PeerForRelay> = members
            .iter()
            .map(|(id, m)| (*id, PeerForRelay {
                remote: m.state.remote_addr,
                pt: m.state.remote_pt,
                rfc2833_pt: m.state.rfc2833_pt,
            }))
            .collect();
        let ids: Vec<ChannelId> = members.keys().copied().collect();
        for id in ids {
            let peer_id = peers.keys().find(|&&k| k != id).copied();
            let peer = peer_id.and_then(|pid| peers.get(&pid)).copied();
            let m = members.get_mut(&id).expect("member present");
            m.state.tick_count += 1;
            let digits = drain_inbound_into_member(m, peer).await;
            for d in digits {
                dtmf_broadcast.push((id, d));
            }
        }
    } else {
        for m in members.values_mut() {
            m.state.tick_count += 1;
            let digits = drain_inbound_into_member(m, None).await;
            for d in digits {
                dtmf_broadcast.push((m.state.id, d));
            }
        }
    }

    // ----- Phase 4 (runs before Phase 2 so peers' queues are primed): -----
    // Fan out detected digits to every other member's dtmf_relay queue.
    for (origin, digit) in &dtmf_broadcast {
        let digit_str = digit.to_string();
        for (other_id, m) in members.iter_mut() {
            if other_id == origin { continue; }
            m.subs.dtmf_relay.enqueue(&digit_str);
        }
    }

    // ----- Phase 2: per-member pop + decode + recorder + player + barge-in. -----
    for m in members.values_mut() {
        // Reset this tick's frame — silence unless we populate below.
        for s in m.frame.iter_mut() { *s = 0; }
        m.frame_present = false;

        let popped = m.state.jitter.pop();

        // Player advances every tick regardless of mix size. For 2-chan we
        // still track its completion so play/end fires on natural end, but
        // 2-chan byte relay already emitted so the player frame isn't used
        // as an outbound signal — matching C++ `mix2` where the player is
        // inert while byte-relaying.
        let mut player_frame: Option<Vec<i16>> = None;
        if let Some(player) = m.subs.player.as_mut() {
            let frame = player.read(160).await;
            if !frame.samples.is_empty() {
                player_frame = Some(frame.samples);
            }
            if player.is_finished() {
                m.subs.player = None;
                m.subs.bargein = None;
                m.state.pending_events.push(Event::Play {
                    state: PlayState::End,
                    reason: Some("completed".into()),
                });
            }
        }

        // Decode inbound (if any) → samples used by recorder, barge-in and
        // (for N≥3) the mix contribution.
        let inbound_samples: Option<Vec<i16>> = popped.as_ref().and_then(|in_pk| {
            m.state.transcoder.decode(in_pk.payload_type(), in_pk.payload())
        });

        // Populate this tick's contribution. Player wins over inbound — when
        // JS has asked us to play audio into the mix, that's our voice.
        if m.state.direction.recv {
            if let Some(samples) = player_frame.as_ref() {
                copy_into_frame(&mut m.frame, samples);
                m.frame_present = true;
            } else if let Some(samples) = inbound_samples.as_ref() {
                copy_into_frame(&mut m.frame, samples);
                m.frame_present = true;
            }
        }

        // Recorder feed (inbound samples, not mix output — matches C++).
        if let Some(samples) = inbound_samples.as_ref() {
            feed_recorders(&mut m.subs.recorders, samples, &mut m.state.pending_events).await;
        }

        // Barge-in on inbound power.
        if let (Some(samples), Some(bi)) = (inbound_samples.as_ref(), m.subs.bargein.as_mut()) {
            if m.subs.player.is_some() && m.state.in_count >= 100 {
                let mut sum_sq: u64 = 0;
                for s in samples {
                    let v = *s as i64;
                    sum_sq += (v * v) as u64;
                }
                let rms = if samples.is_empty() {
                    0
                } else {
                    ((sum_sq / samples.len() as u64) as f64).sqrt() as i32
                };
                let smoothed = bi.power_ma.execute(rms.min(i16::MAX as i32) as i16) as i32;
                if smoothed > bi.power_threshold {
                    m.subs.player = None;
                    m.subs.bargein = None;
                    m.state.pending_events.push(Event::Play {
                        state: PlayState::End,
                        reason: Some("interrupted".into()),
                    });
                }
            }
        }
    }

    // ----- Phase 3: emit per member. -----
    if !use_relay {
        // Build summed once — saturating i32 accumulator, then clamp to i16
        // after subtracting self. Matches C++ sum-then-clamp ordering for
        // better headroom than saturating-each-add.
        let mut summed = vec![0i32; MIX_FRAME_SAMPLES];
        for m in members.values() {
            if !m.state.direction.recv { continue; }
            if !m.frame_present { continue; }
            for (slot, &s) in summed.iter_mut().zip(m.frame.iter()) {
                *slot = slot.saturating_add(s as i32);
            }
        }

        for m in members.values_mut() {
            if !m.state.direction.send { continue; }
            let Some(remote) = m.state.remote_addr else { continue; };

            // DTMF and mix-audio both emit in the same tick — RFC 2833 events
            // use a separate PT and are designed to interleave with media. In
            // the C++ port, audio+DTMF coexist on the wire; the test bands
            // assume both (expected count ≈ 60 media + ~14 DTMF per digit).
            if let Some((_ev, payload)) = m.subs.dtmf_send.next_event() {
                let pt = m.state.rfc2833_pt;
                send_dtmf_to_remote(&mut m.state, remote, pt, &payload).await;
            }
            if let Some((_ev, payload)) = m.subs.dtmf_relay.next_event() {
                let pt = m.state.rfc2833_pt;
                send_dtmf_to_remote(&mut m.state, remote, pt, &payload).await;
            }

            let mut out_samples = vec![0i16; MIX_FRAME_SAMPLES];
            if m.frame_present {
                for i in 0..MIX_FRAME_SAMPLES {
                    let v = summed[i] - m.frame[i] as i32;
                    out_samples[i] = v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                }
            } else {
                for i in 0..MIX_FRAME_SAMPLES {
                    out_samples[i] = summed[i].clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                }
            }

            if let Some(payload) = m.state.transcoder.encode(m.state.remote_pt, &out_samples) {
                send_encoded(&mut m.state, remote, &payload).await;
            }
        }
    } else {
        // 2-chan path — byte relay handled in Phase 1. Here we emit DTMF
        // queues + player frames. Player audio goes to the PEER's remote
        // (not own remote) so the other end of the call hears hold music /
        // prompts. The relay carries inbound peer audio; the player inject
        // carries our generated audio — both coexist, matching C++ mix2.
        let peer_targets: HashMap<ChannelId, PeerForRelay> = members
            .iter()
            .map(|(&id, m)| (id, PeerForRelay {
                remote: m.state.remote_addr,
                pt: m.state.remote_pt,
                rfc2833_pt: m.state.rfc2833_pt,
            }))
            .collect();
        let ids: Vec<ChannelId> = members.keys().copied().collect();
        for id in ids {
            let m = members.get_mut(&id).unwrap();
            if !m.state.direction.send { continue; }
            let Some(remote) = m.state.remote_addr else { continue; };
            if let Some((_ev, payload)) = m.subs.dtmf_send.next_event() {
                let pt = m.state.rfc2833_pt;
                send_dtmf_to_remote(&mut m.state, remote, pt, &payload).await;
                continue;
            }
            if let Some((_ev, payload)) = m.subs.dtmf_relay.next_event() {
                let pt = m.state.rfc2833_pt;
                send_dtmf_to_remote(&mut m.state, remote, pt, &payload).await;
                continue;
            }
            // Player: encode frame to the peer's codec and send to the
            // peer's remote — the player voices "our side" into the mix.
            if m.subs.player.is_some() && m.frame_present {
                let peer_id = peer_targets.keys().find(|&&k| k != id).copied();
                if let Some(peer) = peer_id.and_then(|pid| peer_targets.get(&pid)) {
                    if let Some(peer_remote) = peer.remote {
                        if let Some(payload) = m.state.transcoder.encode(peer.pt, &m.frame) {
                            let mut pkt = RtpPacket::new();
                            pkt.init(m.state.ssrc);
                            pkt.set_payload_type(peer.pt);
                            pkt.set_sequence_number(m.state.out_sn);
                            pkt.set_timestamp(m.state.out_ts);
                            pkt.set_payload(&payload);
                            m.state.out_sn = m.state.out_sn.wrapping_add(1);
                            m.state.out_ts = m.state.out_ts.wrapping_add(MIX_FRAME_SAMPLES as u32);
                            if m.state.rtp_sock.send_to(pkt.as_slice(), peer_remote).await.is_ok() {
                                m.state.out_count += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    // ----- Phase 5: drain pending events through per-member sinks. -----
    for m in members.values_mut() {
        for ev in m.state.pending_events.drain(..) {
            m.events.post(ev);
        }
    }
}

/// 2-chan relay snapshot. Captured once per tick before we start the mutable
/// iteration so classify_and_route_member can forward bytes to the peer
/// without holding a second borrow on `members`.
#[derive(Copy, Clone)]
struct PeerForRelay {
    remote: Option<SocketAddr>,
    pt: u8,
    rfc2833_pt: u8,
}

fn copy_into_frame(dst: &mut [i16], src: &[i16]) {
    let n = src.len().min(dst.len());
    dst[..n].copy_from_slice(&src[..n]);
    for s in &mut dst[n..] { *s = 0; }
}

async fn feed_recorders(
    recorders: &mut Vec<Recorder>,
    samples: &[i16],
    pending_events: &mut Vec<Event>,
) {
    let mut i = 0;
    while i < recorders.len() {
        let rec = &mut recorders[i];
        let prev_state = rec.state();
        let frame: Vec<i16> = if rec.num_channels() == 2 {
            let mut inter = Vec::with_capacity(samples.len() * 2);
            for s in samples { inter.push(*s); inter.push(*s); }
            inter
        } else {
            samples.to_vec()
        };
        let _ = rec.write(&frame).await;
        let new_state = rec.state();
        let file_str = rec.file().to_string_lossy().into_owned();
        if prev_state == RecorderState::Pending && new_state == RecorderState::Active {
            pending_events.push(Event::Record {
                state: RecordState::Recording,
                reason: Some("abovepower".into()),
                file: Some(file_str.clone()),
            });
        }
        if rec.is_finished() {
            let reason = rec.finish_reason().cloned();
            let reason_str = match reason {
                Some(FinishReason::Completed) => "completed",
                Some(FinishReason::MaxDurationReached) => "timeout",
                Some(FinishReason::BelowPowerThreshold) => "belowpower",
                Some(FinishReason::ChannelClosed) => "channelclosed",
                Some(FinishReason::Requested) => "requested",
                None => "completed",
            };
            pending_events.push(Event::Record {
                state: RecordState::Finished,
                reason: Some(reason_str.into()),
                file: Some(file_str),
            });
            recorders.remove(i);
            continue;
        }
        i += 1;
    }
}

/// Drain the member's inbound socket. If `relay_peer` is `Some`, each
/// non-DTMF RTP packet is byte-relayed to the peer (transcoding between
/// G.711 codecs when PTs differ) — the 2-chan fast path. Returns digits
/// detected this tick so the outer loop can fan them out to peers'
/// dtmf_relay queues.
async fn drain_inbound_into_member(
    m: &mut Member,
    relay_peer: Option<PeerForRelay>,
) -> Vec<char> {
    const MAX_INBOUND_PER_TICK: usize = 2;
    let mut digits = Vec::new();
    let mut scratch = [0u8; rtp::RTP_MAX_LENGTH];
    for _ in 0..MAX_INBOUND_PER_TICK {
        match m.state.rtp_sock.try_recv_from(&mut scratch) {
            Ok((n, peer)) => {
                // Autocorrect: latch onto the observed remote regardless of
                // what `remote()` set, matching the C++ NAT-hairpin behavior.
                m.state.remote_addr = Some(peer);
                let d = classify_and_route_member(m, &scratch[..n], peer, relay_peer).await;
                if let Some(c) = d { digits.push(c); }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
    digits
}

async fn classify_and_route_member(
    m: &mut Member,
    pkt: &[u8],
    peer: SocketAddr,
    relay_peer: Option<PeerForRelay>,
) -> Option<char> {
    if pkt.is_empty() { return None; }

    if stun::is_stun(pkt) {
        if m.state.local_icepwd.is_empty() { return None; }
        let key = m.state.local_icepwd.as_bytes();
        let mut req = pkt.to_vec();
        let mut resp = [0u8; rtp::RTP_MAX_LENGTH];
        let n = stun::handle(&mut req, &mut resp, peer, key, key);
        if n > 0 {
            let _ = m.state.rtp_sock.send_to(&resp[..n], peer).await;
        }
        return None;
    }

    let first = pkt[0];
    if (20..=23).contains(&first) { return None; } // DTLS — TODO

    if pkt.len() < rtp::RTP_FIXED_HEADER_LEN { return None; }
    m.state.in_count += 1;

    let pt = rtp::payload_type(pkt);

    // RFC 2833 — detect + emit local event + mark for peer broadcast. The
    // raw DTMF packet is *not* forwarded on the byte relay: peers
    // regenerate their own burst via `dtmf_relay` next tick.
    if pt == m.state.rfc2833_pt {
        let sn = rtp::sequence_number(pkt);
        let payload = &pkt[rtp::RTP_FIXED_HEADER_LEN..];
        if let Some(digit) = m.subs.dtmf_recv.feed(sn, payload) {
            if let Some(p) = m.subs.player.as_ref() {
                if p.interrupts() {
                    m.subs.player = None;
                    m.state.pending_events.push(Event::Play {
                        state: PlayState::End,
                        reason: Some("telephone-event".into()),
                    });
                }
            }
            m.state.pending_events.push(Event::TelephoneEvent { digit });
            return Some(digit);
        }
        return None;
    }

    // 2-chan byte relay: forward directly to the peer (transcoding if the
    // codecs differ). Matches C++ mix2's "pass-through or transcode" path.
    if let Some(peer_info) = relay_peer {
        if let Some(peer_remote) = peer_info.remote {
            let pass_through = pt == peer_info.pt;
            if pass_through {
                if m.state.rtp_sock.send_to(pkt, peer_remote).await.is_ok() {
                    m.state.out_count += 1;
                }
            } else {
                let header_len = rtp::header_len(pkt);
                let payload = &pkt[header_len..];
                match m.state.transcoder.transcode(pt, peer_info.pt, payload) {
                    Some(transcoded) => {
                        let mut buf = pkt[..header_len].to_vec();
                        buf.extend_from_slice(&transcoded);
                        rtp::set_payload_type(&mut buf, peer_info.pt);
                        if m.state.rtp_sock.send_to(&buf, peer_remote).await.is_ok() {
                            m.state.out_count += 1;
                        }
                    }
                    None => {
                        m.state.in_dropped += 1;
                    }
                }
            }
        }
        // Do NOT push into jitter: the relay already decoded via
        // `transcode()`, and pushing here would cause Phase 2's jitter-pop
        // to re-decode the same packet through the stateful codec (G.722,
        // iLBC), corrupting filter history. Recorder/barge-in will see
        // nothing in Phase 2 — matches C++ mix2 where the byte relay
        // doesn't feed per-channel subsystems.
        return None;
    }

    // N≥3 path — no inline relay, just buffer.
    let mut rp = RtpPacket::new();
    rp.as_mut_slice_for_fill(pkt.len()).copy_from_slice(pkt);
    m.state.jitter.push(rp);
    None
}

async fn send_encoded(state: &mut ChannelState, remote: SocketAddr, payload: &[u8]) {
    let mut pkt = RtpPacket::new();
    pkt.init(state.ssrc);
    pkt.set_payload_type(state.remote_pt);
    pkt.set_sequence_number(state.out_sn);
    pkt.set_timestamp(state.out_ts);
    pkt.set_payload(payload);
    state.out_sn = state.out_sn.wrapping_add(1);
    state.out_ts = state.out_ts.wrapping_add(MIX_FRAME_SAMPLES as u32);
    if state.rtp_sock.send_to(pkt.as_slice(), remote).await.is_ok() {
        state.out_count += 1;
    }
}

async fn send_dtmf_to_remote(
    state: &mut ChannelState,
    remote: SocketAddr,
    pt: u8,
    payload: &[u8; 4],
) {
    let mut pkt = RtpPacket::new();
    pkt.init(state.ssrc);
    pkt.set_payload_type(pt);
    pkt.set_sequence_number(state.out_sn);
    pkt.set_timestamp(state.out_ts);
    pkt.set_payload(payload);
    state.out_sn = state.out_sn.wrapping_add(1);
    if state.rtp_sock.send_to(pkt.as_slice(), remote).await.is_ok() {
        state.out_count += 1;
    }
}

/// Apply a JS-originated command to a mixed member. Mirrors the equivalent
/// Local-mode handler in `actor.rs::handle_command_local` — kept in sync by
/// covering the same variants.
async fn apply_forwarded(m: &mut Member, cmd: Command) {
    match cmd {
        Command::Direction(d) => { m.state.direction = d; }
        Command::Echo { enabled } => { m.state.echo = enabled; }
        Command::Dtmf { digits } => { m.subs.dtmf_send.enqueue(&digits); }
        Command::Remote { cfg, ack } => {
            m.state.remote_addr = Some(cfg.addr);
            m.state.remote_pt = cfg.payload_type;
            if let Some(pt) = cfg.rfc2833_payload_type {
                m.state.rfc2833_pt = pt;
            }
            if let Some(pt) = cfg.ilbc_payload_type {
                m.state.transcoder.set_local_ilbc_pt(pt);
            }
            if let Some(pwd) = &cfg.icepwd {
                m.state.remote_icepwd = pwd.clone();
            }
            m.state.remote = Some(cfg);
            m.state.remote_confirmed = true;
            let _ = ack.send(());
        }
        Command::Play { cfg, ack } => {
            if m.subs.player.is_some() {
                m.subs.player = None;
                m.events.post(Event::Play {
                    state: PlayState::End,
                    reason: Some("new".into()),
                });
            }
            m.subs.player = Some(Player::new(cfg));
            m.events.post(Event::Play {
                state: PlayState::Start,
                reason: Some("new".into()),
            });
            let _ = ack.send(());
        }
        Command::Record { cfg, ack } => {
            let file_str = cfg.file.to_string_lossy().into_owned();
            let is_gated = cfg.start_above_power.is_some();
            match Recorder::open(cfg.clone()).await {
                Ok(rec) => {
                    if let Some(idx) = m.subs.recorders.iter().position(|r| r.file() == rec.file()) {
                        let mut old = m.subs.recorders.remove(idx);
                        old.close(FinishReason::ChannelClosed);
                        m.events.post(Event::Record {
                            state: RecordState::Finished,
                            reason: Some("channelclosed".into()),
                            file: Some(file_str.clone()),
                        });
                    }
                    m.subs.recorders.push(rec);
                    if !is_gated {
                        m.events.post(Event::Record {
                            state: RecordState::Recording,
                            reason: None,
                            file: Some(file_str),
                        });
                    }
                }
                Err(e) => {
                    m.events.post(Event::Record {
                        state: RecordState::Finished,
                        reason: Some(format!("open-failed: {e}")),
                        file: Some(file_str),
                    });
                }
            }
            let _ = ack.send(());
        }
        Command::RecordFinish { file } => {
            if let Some(idx) = m.subs.recorders.iter().position(|r| r.file() == file) {
                let mut rec = m.subs.recorders.remove(idx);
                let file_str = rec.file().to_string_lossy().into_owned();
                rec.close(FinishReason::Requested);
                m.events.post(Event::Record {
                    state: RecordState::Finished,
                    reason: Some("requested".into()),
                    file: Some(file_str),
                });
            }
        }
        Command::RecordSetPaused { file, paused } => {
            if let Some(rec) = m.subs.recorders.iter_mut().find(|r| r.file() == file) {
                if paused { rec.pause(); } else { rec.resume(); }
            }
        }
        Command::PlayRecord { cfg, ack } => {
            if m.subs.player.is_some() {
                m.subs.player = None;
                m.events.post(Event::Play {
                    state: PlayState::End,
                    reason: Some("new".into()),
                });
            }
            m.subs.player = Some(Player::new(cfg.player));
            m.events.post(Event::Play {
                state: PlayState::Start,
                reason: Some("new".into()),
            });
            if cfg.interrupt {
                if let Some(threshold) = cfg.bargein_power {
                    let mut ma = crate::firfilter::MaFilter::new();
                    if let Some(n) = cfg.bargein_packets {
                        ma.reset(n as usize);
                    }
                    m.subs.bargein = Some(super::actor::BargeInState {
                        power_threshold: threshold,
                        power_ma: ma,
                    });
                }
            }
            let file_str = cfg.recorder.file.to_string_lossy().into_owned();
            let is_gated = cfg.recorder.start_above_power.is_some();
            match Recorder::open(cfg.recorder).await {
                Ok(rec) => {
                    if let Some(idx) = m.subs.recorders.iter().position(|r| r.file() == rec.file()) {
                        let mut old = m.subs.recorders.remove(idx);
                        old.close(FinishReason::ChannelClosed);
                    }
                    m.subs.recorders.push(rec);
                    if !is_gated {
                        m.events.post(Event::Record {
                            state: RecordState::Recording,
                            reason: None,
                            file: Some(file_str),
                        });
                    }
                }
                Err(e) => {
                    m.events.post(Event::Record {
                        state: RecordState::Finished,
                        reason: Some(format!("open-failed: {e}")),
                        file: Some(file_str),
                    });
                }
            }
            let _ = ack.send(());
        }
        // Mode-change + lifecycle commands don't apply inside the mixer —
        // the channel actor handles those at the top level (it pulls state
        // back out before EnterMix/LeaveMix/Close take effect).
        Command::EnterMix { .. } | Command::LeaveMix { .. } | Command::Close { .. } => {}
    }
}
