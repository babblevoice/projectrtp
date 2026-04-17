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

use super::actor::{Event, EventSink, PlayState, RecordState, Subsystems, TICK_MS};
use super::commands::{ChannelId, Command};
use super::player::Player;
use super::recorder::{FinishReason, Recorder, RecorderState};
use super::rtp::{self, RtpPacket};
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

// ---- mix tick: the unified per-tick pipeline ----

async fn mix_tick(members: &mut HashMap<ChannelId, Box<Member>>) {
    let n_alive = members.len();

    // Cross-configure iLBC dynamic PTs for N=2 so each member's transcoder
    // recognises the peer's wire PT for iLBC encode/decode.
    if n_alive == 2 {
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

    // No socket drain — the recv_loop continuously reads and pushes
    // RTP/DTMF into jitter. DTMF is classified at pop time (Phase 2).
    let mut dtmf_broadcast: Vec<(ChannelId, char)> = Vec::new();
    for m in members.values_mut() {
        m.state.tick_count += 1;
    }

    // ----- Phase 2: pop jitter, decode, cache frame, recorder, player, barge-in. -----
    for m in members.values_mut() {
        for s in m.frame.iter_mut() { *s = 0; }
        m.frame_present = false;
        m.inbound_this_tick = false;

        let popped = m.state.jitter.lock().pop();

        // DTMF at pop time — recv_loop pushes all RTP/DTMF into jitter.
        if let Some(ref pk) = popped {
            let pt = pk.payload_type();
            if pt == m.state.rfc2833_pt {
                let sn = pk.sequence_number();
                let payload = &pk.as_slice()[super::rtp::RTP_FIXED_HEADER_LEN..pk.len()];
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
                    // Broadcast to other members.
                    dtmf_broadcast.push((m.state.id, digit));
                }
                continue; // DTMF consumed — skip audio processing for this member.
            }
        }

        // Player: advance one frame. Player output becomes this member's
        // mix contribution (its "voice") when no inbound is present.
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

        // Decode inbound once — shared by recorder, barge-in, and frame.
        let inbound_samples: Option<Vec<i16>> = popped.as_ref().and_then(|in_pk| {
            m.state.transcoder.decode(in_pk.payload_type(), in_pk.payload())
        });

        // Populate mix frame: inbound wins over player for the contribution
        // (player audio is "our voice", inbound is what the far end sent —
        // in a relay scenario the far end's audio should be what gets mixed).
        if m.state.direction.recv {
            if let Some(samples) = inbound_samples.as_ref() {
                copy_into_frame(&mut m.frame, samples);
                m.frame_present = true;
                m.inbound_this_tick = true;
            } else if let Some(samples) = player_frame.as_ref() {
                copy_into_frame(&mut m.frame, samples);
                m.frame_present = true;
            }
        }

        // Recorder: feed inbound samples (wire audio, not mix output).
        if let Some(samples) = inbound_samples.as_ref() {
            feed_recorders(&mut m.subs.recorders, samples, &mut m.state.pending_events).await;
        }

        // Barge-in on inbound power.
        if let (Some(samples), Some(bi)) = (inbound_samples.as_ref(), m.subs.bargein.as_mut()) {
            if m.subs.player.is_some() && m.state.in_count.load(Ordering::Relaxed) >= 100 {
                let mut sum_sq: u64 = 0;
                for s in samples {
                    let v = *s as i64;
                    sum_sq += (v * v) as u64;
                }
                let rms = if samples.is_empty() { 0 }
                          else { ((sum_sq / samples.len() as u64) as f64).sqrt() as i32 };
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

    // Fan out detected DTMF digits to peers' relay queues.
    for (origin, digit) in &dtmf_broadcast {
        let digit_str = digit.to_string();
        for (&id, m) in members.iter_mut() {
            if id == *origin { continue; }
            m.subs.dtmf_relay.enqueue(&digit_str);
        }
    }

    // ----- Phase 3: outbound. -----
    // Build summed frame (used by both N=2 and N≥3 — for N=2 it's the same
    // as the single peer's frame, but computing it uniformly simplifies the
    // code and makes 2→3 transitions seamless).
    let mut summed = vec![0i32; MIX_FRAME_SAMPLES];
    for m in members.values() {
        if !m.state.direction.recv { continue; }
        if !m.frame_present { continue; }
        for (slot, &s) in summed.iter_mut().zip(m.frame.iter()) {
            *slot = slot.saturating_add(s as i32);
        }
    }

    if n_alive == 2 {
        // N=2: each member's output = peer's frame (= summed - own).
        // Send via the PEER's socket so the far end's autocorrect latches
        // onto the peer's port (matching C++ `dstchan->writepacket()`).
        // Two passes: first encode, then send via peer socket.
        let peer_info: Vec<(ChannelId, ChannelId, Option<SocketAddr>, u8)> = {
            let ids: Vec<ChannelId> = members.keys().copied().collect();
            ids.iter().map(|&id| {
                let peer_id = ids.iter().find(|&&k| k != id).copied().unwrap_or(id);
                let peer = members.get(&peer_id).unwrap();
                (id, peer_id, peer.state.get_remote_addr(), peer.state.remote_pt)
            }).collect()
        };

        let mut outgoing: Vec<(ChannelId, Vec<u8>, SocketAddr)> = Vec::new();
        for &(id, dest_id, peer_remote, peer_pt) in &peer_info {
            let m = members.get_mut(&id).unwrap();
            if !m.state.direction.send { continue; }
            if !m.frame_present { continue; }
            let Some(dest_addr) = peer_remote else { continue; };

            if let Some(payload) = m.state.transcoder.encode(peer_pt, &m.frame) {
                let mut pkt = RtpPacket::new();
                pkt.init(m.state.ssrc);
                pkt.set_payload_type(peer_pt);
                pkt.set_sequence_number(m.state.out_sn);
                pkt.set_timestamp(m.state.out_ts);
                pkt.set_payload(&payload);
                m.state.out_sn = m.state.out_sn.wrapping_add(1);
                m.state.out_ts = m.state.out_ts.wrapping_add(MIX_FRAME_SAMPLES as u32);
                outgoing.push((dest_id, pkt.as_slice().to_vec(), dest_addr));
            }
        }
        // Send via destination member's socket.
        for (dest_id, data, dest_addr) in &outgoing {
            if let Some(dest) = members.get(dest_id) {
                let _ = dest.state.rtp_sock.send_to(data, *dest_addr).await;
            }
        }
        // Count on source members.
        for &(id, _, _, _) in &peer_info {
            if outgoing.iter().any(|(did, _, _)| members.get(did).map(|m| m.state.id) == Some(id).map(|_| members.get(&id).unwrap().state.id).and_then(|_| None).or(Some(0))) {
                // simplified: just count based on outgoing
            }
        }
        // Simpler count: each source that contributed gets +1
        for (dest_id, _, _) in outgoing {
            // Find the source (the other member)
            let src_id = peer_info.iter().find(|(_, did, _, _)| *did == dest_id).map(|(sid, _, _, _)| *sid);
            if let Some(sid) = src_id {
                if let Some(src) = members.get_mut(&sid) {
                    src.state.out_count += 1;
                }
            }
        }
    } else {
        // N≥3: summed - own, encode to own codec, send to own remote.
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

            if let Some(payload) = m.state.transcoder.encode(m.state.remote_pt, &out_samples) {
                send_encoded(&mut m.state, remote, &payload).await;
            }
        }
    }

    // DTMF: send to own remote for all mix sizes. Audio and DTMF coexist.
    for m in members.values_mut() {
        if !m.state.direction.send { continue; }
        let Some(remote) = m.state.get_remote_addr() else { continue; };
        if let Some((_ev, payload)) = m.subs.dtmf_send.next_event() {
            let pt = m.state.rfc2833_pt;
            send_dtmf_to_remote(&mut m.state, remote, pt, &payload).await;
        }
        if let Some((_ev, payload)) = m.subs.dtmf_relay.next_event() {
            let pt = m.state.rfc2833_pt;
            send_dtmf_to_remote(&mut m.state, remote, pt, &payload).await;
        }
    }

    // ----- Phase 5: drain pending events. -----
    for m in members.values_mut() {
        for ev in m.state.pending_events.drain(..) {
            m.events.post(ev);
        }
    }
}

// ---- helpers ----

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
        Command::EnterMix { .. } | Command::LeaveMix { .. } | Command::Close { .. } => {}
    }
}
