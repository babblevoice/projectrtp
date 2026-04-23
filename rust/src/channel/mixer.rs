// Mix group actor — owns every member's ChannelState + Subsystems for the
// duration of the mix, and runs their tick pipeline in lockstep under a
// single 20 ms ticker.
//
// All mix sizes (N=2 and N≥3) share the same inbound pipeline:
//
//   Phase 1: per-member — drain ALL inbound from the UDP socket into the
//            jitter buffer. Draining everything prevents kernel-buffer
//            accumulation (the Freeswitch bug where a delayed server leaves
//            stale audio in the OS buffer, introducing seconds of delay).
//            DTMF (RFC 2833) is detected and consumed here.
//   Phase 2: per-member — pop one packet from jitter, decode to PCM, cache
//            as `frame`. Feed recorders, advance player, barge-in detect.
//            The decode is done once and shared by all consumers.
//   Phase 3: per-member outbound.
//            • N=2: encode own decoded frame to peer's codec, send via
//              peer's socket to peer's remote. Matches C++ mix2 which
//              decoded via `srcchan->incodec` and encoded via
//              `dstchan->outcodec` — never raw byte-forwarded.
//            • N≥3: build summed = Σ all recv-enabled frames. For each
//              member: out = summed - own, encode to own codec, send to
//              own remote.
//            DTMF send/relay queues fire to own remote (all sizes).
//   Phase 4: fan out DTMF digits detected in Phase 1 into peers'
//            `dtmf_relay` queues.
//   Phase 5: drain `state.pending_events` through each member's EventSink.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio::time::{interval, MissedTickBehavior};

use super::actor::{
    activate_pending_recorder, Event, EventSink, PlayState, RecordState, Subsystems, TICK_MS,
    PREBUFFER_CAPACITY_SAMPLES,
};
use super::commands::{ChannelId, Command};
use super::player::Player;
use super::recorder::{FinishReason, Recorder, RecorderState};
use super::rtp::RtpPacket;
use super::state::ChannelState;

pub const MIX_FRAME_SAMPLES: usize = 160;

/// One channel currently living inside a mix group.
pub struct Member {
    pub state: Box<ChannelState>,
    pub subs: Subsystems,
    pub events: Arc<dyn EventSink>,
    pub frame: Vec<i16>,
    pub frame_present: bool,
    /// True when `frame` was populated from an inbound jitter pop this tick
    /// (vs player or silence). N=2 relay only fires on inbound-driven frames
    /// — player frames don't relay in mix2 (matching C++).
    pub inbound_this_tick: bool,
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
            inbound_this_tick: false,
        }
    }
}

pub enum MixerCommand {
    Add { member: Box<Member>, ack: oneshot::Sender<()> },
    Remove { id: ChannelId, ack: oneshot::Sender<Option<Box<Member>>> },
    Forward { id: ChannelId, cmd: Command },
    #[allow(dead_code)]
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

    #[allow(dead_code)]
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

// ---- Outbound helpers (Phase 3 of the mix tick) ----

/// N=2 outbound: each member's output is the peer's frame (source→dest
/// pass-through, matches C++ `mix2`). We extract both members' mutable
/// refs via `iter_mut` and call `send_leg` for each direction in turn
/// — no intermediate Vec, one pass, analogous to the C++ which grabs
/// chan1 and chan2 directly.
///
/// The RTP packet sent to the destination MUST use the DESTINATION's
/// SRTP encrypt context, SSRC and outbound counters — those were
/// negotiated on the destination's leg and are what the destination
/// peer expects to decrypt/reassemble. Using the source's state
/// produces audible noise or silence whenever at least one leg is SRTP
/// (e.g. Chromium ↔ SIP), since the destination can't decrypt with
/// the wrong key.
async fn mix2_outbound(members: &mut HashMap<ChannelId, Box<Member>>) {
    let mut iter = members.iter_mut();
    let Some((_, first)) = iter.next() else { return; };
    let Some((_, second)) = iter.next() else { return; };
    // Peel the `Box<Member>` so we work with two `&mut Member`s.
    let (a, b): (&mut Member, &mut Member) = (first, second);

    send_leg(a, b).await;
    send_leg(b, a).await;
}

/// One direction of the N=2 pass-through. Dst pulls the best-available
/// representation from src's bundle via `encode_from`:
///   - same codec both ends → src's wire bytes are copied through
///     verbatim (no decode, no encode — the G.722↔G.722 / PCMA↔PCMA
///     fast path);
///   - dst G.722 with different src codec → uses src's wideband (may
///     already be cached) so the LP filter chain runs once instead of
///     downsample-then-upsample;
///   - else → src's 8 kHz linear feeds dst's stateful encoder.
///
/// The encoder state (G.722 predictor, iLBC LP, up-sample filter)
/// persists on `dst.state.codecx` for a coherent outbound stream.
async fn send_leg(src: &mut Member, dst: &mut Member) {
    if !src.state.direction.send { return; }
    if !src.frame_present { return; }
    let Some(dest_addr) = dst.state.get_remote_addr() else { return; };
    let peer_pt = dst.state.remote_pt;

    let Some(wire) = dst.state.codecx.encode_from(peer_pt, &mut src.state.codecx) else { return; };
    // Owned copy so the `&mut dst.state.codecx` borrow ends before the
    // `&mut dst.state` borrows later for the RTP send.
    let payload: Vec<u8> = wire.to_vec();

    let mut pkt = RtpPacket::new();
    pkt.init(dst.state.ssrc);
    pkt.set_payload_type(peer_pt);
    pkt.set_sequence_number(dst.state.out_sn);
    pkt.set_timestamp(dst.state.out_ts);
    pkt.set_payload(&payload);
    dst.state.out_sn = dst.state.out_sn.wrapping_add(1);
    dst.state.out_ts = dst.state.out_ts.wrapping_add(MIX_FRAME_SAMPLES as u32);

    let send_ok = if let Some(ref mut ctx) = dst.state.srtp_encrypt {
        match ctx.encrypt_rtp(pkt.as_slice()) {
            Ok(encrypted) => dst.state.rtp_sock.send_to(&encrypted, dest_addr).await.is_ok(),
            Err(_) => false,
        }
    } else {
        dst.state.rtp_sock.send_to(pkt.as_slice(), dest_addr).await.is_ok()
    };
    if send_ok {
        dst.state.out_count += 1;
    }
}

/// N≥3 outbound: each member's output is `summed - own`, encoded with
/// its own codec, sent to its own remote. The summing is deferred until
/// here — it's only useful at N≥3 (for N=2 it equals the peer's frame
/// directly, which `mix2_outbound` uses via pass-through).
async fn mix_all_outbound(members: &mut HashMap<ChannelId, Box<Member>>) {
    // Sum all members' frames into a single int32 buffer.
    let mut summed = vec![0i32; MIX_FRAME_SAMPLES];
    for m in members.values() {
        if !m.state.direction.recv { continue; }
        if !m.frame_present { continue; }
        for (slot, &sample) in summed.iter_mut().zip(m.frame.iter()) {
            *slot = slot.saturating_add(sample as i32);
        }
    }

    for m in members.values_mut() {
        if !m.state.direction.send { continue; }
        let Some(remote) = m.state.get_remote_addr() else { continue; };

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

        // Feed dst's (this member's) bundle the computed mix samples,
        // then pull the wire bytes in its own remote_pt. Encoder state
        // persists on `m.state.codecx`.
        m.state.codecx.feed_linear_8k(&out_samples);
        let pt = m.state.remote_pt;
        if let Some(wire) = m.state.codecx.require_wire_as(pt) {
            // Owned copy to release the `&mut m.state.codecx` borrow
            // before the send call below needs the rest of m.state.
            let payload: Vec<u8> = wire.to_vec();
            send_encoded(&mut m.state, remote, &payload).await;
        }
    }
}

// ---- Per-channel operations (methods on Member) ----
//
// Each tick calls a sequence of these on every member — mirroring the C++
// style where each channel has `senddtmf`, `writerecordings`, `checkidlerecv`,
// `endticktimer` etc. as methods. Keeps the tick orchestrator readable and
// each operation reasonable in isolation.

impl Member {
    /// Clear per-tick state at the start of each tick. The mix frame is
    /// zeroed so silent inbound doesn't leak last tick's audio into the
    /// mix output.
    fn reset_for_tick(&mut self) {
        for s in self.frame.iter_mut() { *s = 0; }
        self.frame_present = false;
        self.inbound_this_tick = false;
    }

    /// Pop one packet from the jitter buffer and SRTP-decrypt in place if
    /// an SRTP context is active. Returns None if no packet is available
    /// or if decryption fails.
    fn pop_inbound(&mut self) -> Option<RtpPacket> {
        let mut popped = self.state.jitter.lock().pop();
        if let (Some(ref mut pk), Some(ref mut ctx)) =
            (&mut popped, &mut self.state.srtp_decrypt)
        {
            match ctx.decrypt_rtp(pk.as_slice()) {
                Ok(decrypted) => {
                    let n = decrypted.len().min(pk.buf.len());
                    pk.buf[..n].copy_from_slice(&decrypted[..n]);
                    pk.len = n;
                }
                Err(_) => return None,
            }
        }
        popped
    }

    /// Process an RFC-2833 DTMF packet: feed the receiver, interrupt an
    /// interrupt-enabled player, emit `telephone-event`. Returns the
    /// digit so the caller can queue it for peer broadcast.
    async fn handle_dtmf_in(&mut self, pk: &RtpPacket) -> Option<char> {
        let sn = pk.sequence_number();
        let ts = pk.timestamp();
        let payload = &pk.as_slice()[super::rtp::RTP_FIXED_HEADER_LEN..pk.len()];
        let digit = self.subs.dtmf_recv.feed(sn, ts, payload)?;

        let mut activate_recorder = false;
        if let Some(p) = self.subs.player.as_ref() {
            if p.interrupts() {
                self.subs.player = None;
                self.subs.bargein = None;
                self.state.pending_events.push(Event::Play {
                    state: PlayState::End,
                    reason: Some("telephone-event".into()),
                });
                activate_recorder = true;
            }
        }
        self.state.pending_events.push(Event::TelephoneEvent { digit });
        if activate_recorder {
            let _ = activate_pending_recorder(
                &mut self.subs, &mut self.state.pending_events,
            ).await;
        }
        Some(digit)
    }

    /// Pull one frame from the active player, if any. Clears player +
    /// bargein and activates a queued `playrecord` recorder when the
    /// player has just finished.
    async fn tick_player(&mut self) -> Option<Vec<i16>> {
        let mut player_frame: Option<Vec<i16>> = None;
        let mut player_just_ended = false;
        if let Some(player) = self.subs.player.as_mut() {
            let frame = player.read(160).await;
            if !frame.samples.is_empty() {
                player_frame = Some(frame.samples);
            }
            if player.is_finished() {
                self.subs.player = None;
                self.subs.bargein = None;
                self.state.pending_events.push(Event::Play {
                    state: PlayState::End,
                    reason: Some("completed".into()),
                });
                player_just_ended = true;
            }
        }
        if player_just_ended {
            let _ = activate_pending_recorder(
                &mut self.subs, &mut self.state.pending_events,
            ).await;
        }
        player_frame
    }

    /// Populate the member's mix contribution. Inbound wins over player
    /// (matches C++ relay semantics — the far end's audio is what should
    /// be mixed, not our own playback).
    fn populate_mix_frame(&mut self, inbound: Option<&[i16]>, player: Option<&[i16]>) {
        if !self.state.direction.recv { return; }
        if let Some(samples) = inbound {
            copy_into_frame(&mut self.frame, samples);
            self.frame_present = true;
            self.inbound_this_tick = true;
        } else if let Some(samples) = player {
            copy_into_frame(&mut self.frame, samples);
            self.frame_present = true;
        }
    }

    /// Barge-in on inbound power. Fires a player-end event and activates
    /// a queued recorder when the smoothed RMS crosses the threshold.
    async fn run_bargein(&mut self, inbound: Option<&[i16]>) {
        let Some(samples) = inbound else { return; };
        let Some(bi) = self.subs.bargein.as_mut() else { return; };
        if self.subs.player.is_none() { return; }
        if self.state.in_count.load(Ordering::Relaxed) < 100 { return; }

        let mut sum_sq: u64 = 0;
        for s in samples {
            let v = *s as i64;
            sum_sq += (v * v) as u64;
        }
        let rms = if samples.is_empty() { 0 }
                  else { ((sum_sq / samples.len() as u64) as f64).sqrt() as i32 };
        let smoothed = bi.power_ma.execute(rms.min(i16::MAX as i32) as i16) as i32;
        if smoothed <= bi.power_threshold { return; }

        self.subs.player = None;
        self.subs.bargein = None;
        self.state.pending_events.push(Event::Play {
            state: PlayState::End,
            reason: Some("interrupted".into()),
        });
        let _ = activate_pending_recorder(
            &mut self.subs, &mut self.state.pending_events,
        ).await;
    }

    /// While `playrecord` is in its play phase (player active AND
    /// recorder queued), accumulate inbound samples into `prebuffer`.
    /// Drops oldest samples when the cap is hit.
    fn accumulate_playrecord_prebuffer(&mut self, inbound: Option<&[i16]>) {
        if self.subs.player.is_none() || self.subs.pending_recorder.is_none() { return; }
        let Some(samples) = inbound else { return; };

        if self.subs.prebuffer.len() + samples.len() > PREBUFFER_CAPACITY_SAMPLES {
            let drop_n = self.subs.prebuffer.len() + samples.len() - PREBUFFER_CAPACITY_SAMPLES;
            for _ in 0..drop_n.min(self.subs.prebuffer.len()) {
                self.subs.prebuffer.pop_front();
            }
        }
        for s in samples { self.subs.prebuffer.push_back(*s); }
    }

    /// Feed active recorders the inbound 8 kHz linear samples for this
    /// tick. The codec bundle's `narrowband_8k` cache was populated
    /// earlier in the tick (frame populate). Called AFTER the mix phase
    /// to match C++ `writerecordings` ordering — the mix output has
    /// already been sent by the time recordings land on disk.
    ///
    /// Run one tick's worth of inbound-side work for this member —
    /// reset-for-tick, pull the next packet, short-circuit on DTMF,
    /// otherwise decode + populate the mix frame + run barge-in + feed
    /// the prebuffer. Returns `Some(digit)` when a DTMF digit was
    /// detected so the caller can fan it out to the other legs.
    ///
    /// Mirrors the Local `tick::run` phase order so the two paths track
    /// each other by eye: housekeeping → pop → DTMF → player → decode →
    /// bargein → prebuffer. The only steps that live on the Local side
    /// but not here are recorder/reader feed and outbound send — those
    /// happen after the mix phase in `mix_tick`.
    async fn process_inbound(&mut self) -> Option<char> {
        self.state.tick_count += 1;
        self.reset_for_tick();

        // Build SRTP contexts as soon as the DTLS handshake completes.
        // In Local mode this runs via `tick::run`; in the mixer the
        // Local tick stops firing once the channel joins the mix, so
        // without this call the keying-material oneshot sits unread
        // forever and SRTP packets can't be decrypted.
        super::tick::poll_dtls_handshake(&mut self.state);

        let popped = self.pop_inbound();
        if popped.is_some() {
            self.state.ticks_without_rtp = 0;
        } else {
            self.state.ticks_without_rtp += 1;
        }

        // DTMF consumes the packet — skip all audio work for this tick.
        if let Some(ref pk) = popped {
            if pk.payload_type() == self.state.rfc2833_pt {
                return self.handle_dtmf_in(pk).await;
            }
        }

        let player_frame = self.tick_player().await;
        let writer_frame = self.tick_writer();
        let out_frame = writer_frame.as_deref().or(player_frame.as_deref());

        // Feed the wire bytes into the codec bundle. Decoding is lazy —
        // the first consumer (mix encode_from, write_recordings) that
        // calls `require_narrowband_8k` triggers it; subsequent callers
        // share the cached result.
        if let Some(in_pk) = popped.as_ref() {
            self.state.codecx.feed_wire(in_pk.payload_type(), in_pk.payload());
        }
        let inbound: Option<Vec<i16>> = if popped.is_some() {
            self.state.codecx.require_narrowband_8k().map(|s| s.to_vec())
        } else {
            None
        };

        self.populate_mix_frame(inbound.as_deref(), out_frame);
        self.run_bargein(inbound.as_deref()).await;
        self.accumulate_playrecord_prebuffer(inbound.as_deref());

        None
    }

    /// Sibling of `tick_player` on the mixer side — same role (outbound
    /// source) but fed from a JS `Writable` rather than a WAV file.
    /// Tears the writer down and emits `play/end` once the JS side has
    /// ended and we've drained the last partial frame.
    fn tick_writer(&mut self) -> Option<Vec<i16>> {
        let frame = {
            let w = self.subs.writer.as_mut()?;
            w.next_frame_8k()
        };
        let drained = self.subs.writer.as_ref().is_some_and(|w| w.is_drained_and_ended());
        if drained {
            self.subs.writer = None;
            self.state.pending_events.push(Event::Play {
                state: PlayState::End,
                reason: Some("completed".into()),
            });
        }
        frame
    }

    /// `peer_samples` (when `Some`) is the mix peer's inbound 8 kHz
    /// linear. Stereo recorders on this channel use it as the right
    /// channel, giving a true stereo call recording (L=this leg,
    /// R=far leg) — matches C++ mix recording semantics. When `None`
    /// (Local mode, N≥3, or peer had no inbound this tick) stereo
    /// falls back to duplicate-mono.
    async fn write_recordings(&mut self, peer_samples: Option<&[i16]>) {
        if !self.inbound_this_tick { return; }
        let Some(samples) = self.state.codecx.require_narrowband_8k().map(|s| s.to_vec()) else { return; };
        let ch_count = self.state.in_count.load(Ordering::Relaxed);
        feed_recorders(
            &mut self.subs.recorders, &samples, peer_samples, ch_count,
            &mut self.state.pending_events,
        ).await;
        // AudioReaders share the recorder's L/R convention: L=self inbound,
        // R=peer inbound (in mix mode that's the "outbound" side — what
        // we'd send out). Same cache, same timing as the recorder.
        for reader in self.subs.readers.iter_mut() {
            reader.feed(&mut self.state.codecx, Some(&samples), peer_samples);
        }
        self.subs.readers.retain(|r| !r.is_closed());
    }

    /// Emit any queued RFC-2833 DTMF (own + relayed-from-peer) to this
    /// channel's own remote. Fires on every mix size.
    async fn send_dtmf_outbound(&mut self) {
        if !self.state.direction.send { return; }
        let Some(remote) = self.state.get_remote_addr() else { return; };
        let pt = self.state.rfc2833_pt;
        if let Some((_ev, payload)) = self.subs.dtmf_send.next_event() {
            send_dtmf_to_remote(&mut self.state, remote, pt, &payload).await;
        }
        if let Some((_ev, payload)) = self.subs.dtmf_relay.next_event() {
            send_dtmf_to_remote(&mut self.state, remote, pt, &payload).await;
        }
    }

    /// Drain `state.pending_events` through the channel's `EventSink`.
    /// Events are batched within the tick so downstream sees them in
    /// tick-produced order.
    fn drain_pending_events(&mut self) {
        for ev in self.state.pending_events.drain(..) {
            self.events.post(ev);
        }
    }

    /// Multi-tier idle check matching C++ `checkidlerecv`.
    fn is_idle(&self) -> bool {
        use super::tick::{IDLE_TICK_LIMIT, HARD_TIMEOUT_NO_REMOTE, HARD_TIMEOUT_NO_RECV};
        if self.state.direction.recv {
            if self.state.remote_confirmed {
                self.state.ticks_without_rtp >= IDLE_TICK_LIMIT
            } else {
                self.state.tick_count >= HARD_TIMEOUT_NO_REMOTE
            }
        } else {
            self.state.tick_count >= HARD_TIMEOUT_NO_RECV
        }
    }

    /// Emit a Close event with the current stats snapshot. Called when
    /// the channel is being removed from the mix (idle or externally).
    fn emit_close_event(&self, reason: &str) {
        self.events.post(Event::Close {
            reason: reason.into(),
            stats: super::actor::ChannelStats {
                in_count: self.state.in_count.load(Ordering::Relaxed),
                in_dropped: self.state.in_dropped + self.state.jitter.lock().dropped,
                in_skip: self.state.in_skip,
                out_count: self.state.out_count,
            },
        });
    }
}

// ---- Mix-group orchestrator ----
//
// The tick runs in this order — deliberately mirroring C++:
//
//   1. per-member inbound processing (decode, frame populate, bargein)
//   2. DTMF fan-out to peer relay queues
//   3. outbound mix (mix2 or mix_all)
//   4. write recordings               ← AFTER mix, matches C++
//   5. send queued DTMF outbound
//   6. drain events to sinks
//   7. idle check + close
//
// Putting recordings after the mix means the bytes on the wire this tick
// are already sent by the time we touch disk, and the frame cache is
// still valid (the mix encode path doesn't clear the inbound
// narrowband_8k cache).

/// One mixer tick — 20 ms worth of per-member processing followed by
/// the cross-member mix and post-mix housekeeping. Intentionally a thin
/// orchestrator so the phase order reads like a summary of the C++
/// `mux::tick` flow: configure → inbound → DTMF fanout → mix →
/// recordings/DTMF-out/events → idle sweep.
async fn mix_tick(members: &mut HashMap<ChannelId, Box<Member>>) {
    let n_alive = members.len();
    configure_peer_ilbc_pts(members, n_alive);

    let dtmf_broadcast = run_inbound_phase(members).await;
    broadcast_dtmf_to_peer_relays(members, &dtmf_broadcast);

    // mix2 is direct pass-through; mix_all does summed-minus-own. Both
    // use the destination's codec bundle for encoding.
    if n_alive == 2 {
        mix2_outbound(members).await;
    } else if n_alive >= 3 {
        mix_all_outbound(members).await;
    }

    run_post_mix_phase(members, n_alive).await;
    close_idle_members(members);
}

/// Per-member inbound work — delegates to `Member::process_inbound` for
/// each channel and collects any DTMF digits so the caller can fan them
/// out to the other legs' outbound DTMF queues.
async fn run_inbound_phase(
    members: &mut HashMap<ChannelId, Box<Member>>,
) -> Vec<(ChannelId, char)> {
    let mut dtmf_broadcast = Vec::new();
    for m in members.values_mut() {
        if let Some(digit) = m.process_inbound().await {
            dtmf_broadcast.push((m.state.id, digit));
        }
    }
    dtmf_broadcast
}

/// Post-mix per-member housekeeping. Order matches C++:
/// `writerecordings` → `senddtmf` → `endticktimer` (events drain here).
/// For N=2, stereo recorders need each member's peer samples — computed
/// once up-front so every member can read its sibling's narrowband
/// without reborrowing.
async fn run_post_mix_phase(
    members: &mut HashMap<ChannelId, Box<Member>>,
    n_alive: usize,
) {
    let peer_samples_by_id = compute_peer_samples_by_id(members, n_alive);

    // Clone the id list so we can look up each member's peer samples
    // from the sibling map while holding a `&mut` to the member.
    let ids: Vec<ChannelId> = members.keys().copied().collect();
    for id in ids {
        let peer_samples: Option<Vec<i16>> = if n_alive == 2 {
            // Always Some in N=2 — silence fallback when the peer had
            // no inbound this tick; never None (which would trigger
            // duplicate-mono in the stereo recorder).
            peer_samples_by_id.iter()
                .find_map(|(&pid, s)| if pid != id { Some(s.clone()) } else { None })
                .or_else(|| Some(vec![ 0i16; MIX_FRAME_SAMPLES ]))
        } else {
            None
        };
        let Some(m) = members.get_mut(&id) else { continue; };
        m.write_recordings(peer_samples.as_deref()).await;
        m.send_dtmf_outbound().await;
        m.drain_pending_events();
    }
}

/// Build the per-member "peer samples" map consumed by the N=2 stereo
/// recorder. Empty map for N≠2. Members with no inbound this tick get
/// silence so the other leg's stereo recorder never falls back to
/// duplicate-mono mid-call (which would mirror their own audio onto
/// the peer channel of the recording).
fn compute_peer_samples_by_id(
    members: &mut HashMap<ChannelId, Box<Member>>,
    n_alive: usize,
) -> HashMap<ChannelId, Vec<i16>> {
    if n_alive != 2 {
        return HashMap::new();
    }
    members.iter_mut()
        .map(|(&id, m)| {
            let samples = if m.inbound_this_tick {
                m.state.codecx.require_narrowband_8k()
                    .map(|s| s.to_vec())
                    .unwrap_or_else(|| vec![ 0i16; MIX_FRAME_SAMPLES ])
            } else {
                vec![ 0i16; MIX_FRAME_SAMPLES ]
            };
            (id, samples)
        })
        .collect()
}

/// For N=2, tell each member's codec bundle the peer's iLBC wire PT
/// (when dynamic, i.e. ≥96) so the bridge can decode/encode at the
/// right PT on both sides of an asymmetric-dynamic pair.
fn configure_peer_ilbc_pts(members: &mut HashMap<ChannelId, Box<Member>>, n_alive: usize) {
    if n_alive != 2 { return; }
    let pts: Vec<(ChannelId, u8)> = members.iter()
        .map(|(&id, m)| (id, m.state.remote_pt))
        .collect();
    for (&id, m) in members.iter_mut() {
        if let Some(&(_, peer_pt)) = pts.iter().find(|(pid, _)| *pid != id) {
            if peer_pt >= 96 {
                m.state.codecx.set_peer_ilbc_pt(peer_pt);
            }
        }
    }
}

/// Push each detected DTMF digit into every PEER's relay queue (not the
/// origin's).
fn broadcast_dtmf_to_peer_relays(
    members: &mut HashMap<ChannelId, Box<Member>>,
    digits: &[(ChannelId, char)],
) {
    for (origin, digit) in digits {
        let digit_str = digit.to_string();
        for (&id, m) in members.iter_mut() {
            if id == *origin { continue; }
            m.subs.dtmf_relay.enqueue(&digit_str);
        }
    }
}

/// Remove idle members from the mix and emit each one's Close event.
fn close_idle_members(members: &mut HashMap<ChannelId, Box<Member>>) {
    let idle_ids: Vec<ChannelId> = members.iter()
        .filter(|(_, m)| m.is_idle())
        .map(|(&id, _)| id)
        .collect();
    for id in idle_ids {
        if let Some(m) = members.remove(&id) {
            m.emit_close_event("idle");
        }
    }
}

// ---- helpers ----

fn copy_into_frame(dst: &mut [i16], src: &[i16]) {
    let n = src.len().min(dst.len());
    dst[..n].copy_from_slice(&src[..n]);
    for s in &mut dst[n..] { *s = 0; }
}

/// Feed every active recorder on this channel one tick's worth of audio.
///
/// For stereo recorders (`num_channels == 2`) the left channel is the
/// channel's own inbound (`samples`) and the right channel is the mix
/// peer's inbound (`peer_samples`, when present). Matches C++ call-
/// recording semantics where a mixed 2-leg call is written as a true
/// stereo file — left = this leg, right = far leg — so playback lets
/// you hear each party on a separate ear.
///
/// When `peer_samples` is `None` (Local mode, prebuffer flush, N≥3 mix
/// with no canonical peer), the right channel falls back to the same
/// samples as the left — a functional but less informative stereo.
///
/// If `peer_samples` is shorter than `samples`, missing positions are
/// zero-filled rather than truncating the left channel.
async fn feed_recorders(
    recorders: &mut Vec<Recorder>,
    samples: &[i16],
    peer_samples: Option<&[i16]>,
    channel_in_count: u64,
    pending_events: &mut Vec<Event>,
) {
    let mut i = 0;
    while i < recorders.len() {
        let rec = &mut recorders[i];
        let prev_state = rec.state();
        let frame: Vec<i16> = if rec.num_channels() == 2 {
            let mut inter = Vec::with_capacity(samples.len() * 2);
            match peer_samples {
                Some(peer) => {
                    for (idx, &s) in samples.iter().enumerate() {
                        inter.push(s);
                        inter.push(peer.get(idx).copied().unwrap_or(0));
                    }
                }
                None => {
                    // No peer available — duplicate-mono fallback.
                    for &s in samples { inter.push(s); inter.push(s); }
                }
            }
            inter
        } else {
            samples.to_vec()
        };
        let _ = rec.write_with_count(&frame, Some(channel_in_count)).await;
        let new_state = rec.state();
        let file_str = rec.file().to_string_lossy().into_owned();
        if prev_state == RecorderState::Pending && new_state == RecorderState::Active {
            pending_events.push(Event::Record {
                state: RecordState::Recording,
                reason: Some("abovepower".into()),
                file: Some(file_str.clone()),
                filesize: None,
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
            let size = rec.file_size();
            pending_events.push(Event::Record {
                state: RecordState::Finished,
                reason: Some(reason_str.into()),
                file: Some(file_str),
                filesize: Some(size),
            });
            recorders.remove(i);
            continue;
        }
        i += 1;
    }
}


async fn send_rtp(state: &mut ChannelState, pkt: &RtpPacket, remote: SocketAddr) {
    if let Some(ref mut ctx) = state.srtp_encrypt {
        if let Ok(encrypted) = ctx.encrypt_rtp(pkt.as_slice()) {
            if state.rtp_sock.send_to(&encrypted, remote).await.is_ok() {
                state.out_count += 1;
            }
        }
    } else if state.rtp_sock.send_to(pkt.as_slice(), remote).await.is_ok() {
        state.out_count += 1;
    }
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
    send_rtp(state, &pkt, remote).await;
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
    send_rtp(state, &pkt, remote).await;
}

// ---- command forwarding ----

async fn apply_forwarded(m: &mut Member, cmd: Command) {
    match cmd {
        Command::Direction(d) => { m.state.direction = d; }
        Command::Echo { enabled } => { m.state.echo = enabled; }
        Command::Dtmf { digits } => { m.subs.dtmf_send.enqueue(&digits); }
        Command::Remote { cfg, ack } => {
            m.state.set_remote_addr(cfg.addr);
            m.state.remote_pt = cfg.payload_type;
            if let Some(pt) = cfg.rfc2833_payload_type {
                m.state.rfc2833_pt = pt;
            }
            if let Some(pt) = cfg.ilbc_payload_type {
                m.state.codecx.set_local_ilbc_pt(pt);
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
                m.subs.bargein = None;
                m.events.post(Event::Play {
                    state: PlayState::End,
                    reason: Some("new".into()),
                });
            }
            // Bare `play` supersedes any pending playrecord recorder. No
            // Recording event was emitted yet, so no matching Finished is owed.
            m.subs.pending_recorder = None;
            m.subs.prebuffer.clear();
            m.subs.player = Some(Player::new(cfg));
            m.events.post(Event::Play {
                state: PlayState::Start,
                reason: Some("new".into()),
            });
            let _ = ack.send(());
        }
        Command::Record { cfg, ack } => {
            m.subs.pending_recorder = None;
            m.subs.prebuffer.clear();
            let file_str = cfg.file.to_string_lossy().into_owned();
            let is_gated = cfg.start_above_power.is_some();
            // If an existing recorder at the same path is paused, resume it
            // instead of replacing it. This preserves the recording file so
            // both segments end up in one WAV.
            if let Some(rec) = m.subs.recorders.iter_mut().find(|r| r.file() == cfg.file && r.state() == RecorderState::Paused) {
                rec.resume();
                m.events.post(Event::Record {
                    state: RecordState::Recording,
                    reason: None,
                    file: Some(file_str),
                    filesize: None,
                });
            } else {
            match Recorder::open(cfg.clone()).await {
                Ok(rec) => {
                    if let Some(idx) = m.subs.recorders.iter().position(|r| r.file() == rec.file()) {
                        let mut old = m.subs.recorders.remove(idx);
                        let size = old.file_size();
                        old.close(FinishReason::ChannelClosed);
                        m.events.post(Event::Record {
                            state: RecordState::Finished,
                            reason: Some("channelclosed".into()),
                            file: Some(file_str.clone()),
                            filesize: Some(size),
                        });
                    }
                    m.subs.recorders.push(rec);
                    if !is_gated {
                        m.events.post(Event::Record {
                            state: RecordState::Recording,
                            reason: None,
                            file: Some(file_str),
                            filesize: None,
                        });
                    }
                }
                Err(e) => {
                    m.events.post(Event::Record {
                        state: RecordState::Finished,
                        reason: Some(format!("open-failed: {e}")),
                        file: Some(file_str),
                        filesize: None,
                    });
                }
            }
            }
            let _ = ack.send(());
        }
        Command::CreateReadStream { id, cfg, sender } => {
            m.subs.readers.push(super::audio_reader::AudioReader::new(id, cfg, sender));
        }
        Command::DestroyReadStream { id } => {
            m.subs.readers.retain(|r| r.id() != id);
        }
        Command::CreateWriteStream { id, cfg, receiver } => {
            // Same supersession semantics as `Command::Play` in mix mode
            // — active player yields to the writer.
            if m.subs.player.is_some() {
                m.subs.player = None;
                m.subs.bargein = None;
                m.events.post(Event::Play {
                    state: PlayState::End,
                    reason: Some("new".into()),
                });
            }
            m.subs.pending_recorder = None;
            m.subs.prebuffer.clear();
            m.subs.writer = Some(super::audio_writer::AudioWriter::new(id, cfg, receiver));
        }
        Command::DestroyWriteStream { id } => {
            if let Some(w) = m.subs.writer.as_ref() {
                if w.id() == id { m.subs.writer = None; }
            }
        }
        Command::RecordFinish { file } => {
            if let Some(idx) = m.subs.recorders.iter().position(|r| r.file() == file) {
                let mut rec = m.subs.recorders.remove(idx);
                let file_str = rec.file().to_string_lossy().into_owned();
                let size = rec.file_size();
                rec.close(FinishReason::Requested);
                m.events.post(Event::Record {
                    state: RecordState::Finished,
                    reason: Some("requested".into()),
                    file: Some(file_str),
                    filesize: Some(size),
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
                m.subs.bargein = None;
                m.events.post(Event::Play {
                    state: PlayState::End,
                    reason: Some("new".into()),
                });
            }
            // Supersede any previously-queued pending recorder — matches the
            // local-mode handler. No Record event is owed since no file was
            // ever opened.
            m.subs.pending_recorder = None;
            m.subs.prebuffer.clear();

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
            // Queue the recorder — opened on play-end or barge-in via
            // `activate_pending_recorder`. Parity with local-mode handler.
            let file_str = cfg.recorder.file.to_string_lossy().into_owned();
            m.subs.pending_recorder = Some(super::actor::PendingRecorder {
                cfg: cfg.recorder,
                file_str,
            });
            let _ = ack.send(());
        }
        Command::EnterMix { .. } | Command::LeaveMix { .. } | Command::Close { .. } => {}
    }
}
