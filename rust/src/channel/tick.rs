// Per-tick pipeline — the core of the channel actor in Local mode.
//
// The recv_loop task reads the socket continuously and pushes RTP/DTMF
// packets to the jitter buffer. This tick pops one per 20ms cycle,
// decodes, runs subsystems (recorder, player, barge-in), and sends
// outbound (echo, player, DTMF, silence).
//
// `run` is intentionally a thin orchestrator — one phase per call — so
// the shape of a tick is legible without paging through 300 lines. Each
// phase is a named helper below. The order matches the C++ addon's
// handletick flow:
//
//   1.  housekeeping  : tick_count, DTLS handshake result poll
//   2.  inbound       : pop from jitter, SRTP-decrypt
//   3.  DTMF classify : rfc2833 packets short-circuit here
//   4.  player        : read next 160-sample frame if playing
//   5.  decode        : wire bytes → narrowband 8 kHz linear
//   6.  barge-in      : interrupt player on loud inbound
//   7.  pre-buffer    : capture audio under playrecord's prompt
//   8.  recorder/readers : write to disk / push to JS readers
//   9.  outbound      : send DTMF / player / echo / nothing
//  10.  idle check    : multi-tier timeouts, matches C++ checkidlerecv

use std::net::SocketAddr;
use std::sync::atomic::Ordering;

use super::actor::{activate_pending_recorder, Event, Subsystems, PREBUFFER_CAPACITY_SAMPLES};
use super::rtp::{self, RtpPacket};
use super::state::ChannelState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    Continue,
    #[allow(dead_code)]
    Handshaking,
    Stop,
}

pub const IDLE_TICK_LIMIT: u64 = 50 * 20;          // 20s — soft idle (no RTP with remote confirmed)
pub const HARD_TIMEOUT_NO_REMOTE: u64 = 50 * 60 * 60;     // 1hr — remote() never called
pub const HARD_TIMEOUT_NO_RECV: u64 = 50 * 60 * 60 * 2;   // 2hr — on hold (recv=false)

// 20 ms at 8 kHz — the canonical narrowband frame length used for
// recorder/reader frames when the inbound side is silent.
const FRAME_SAMPLES: usize = 160;

pub async fn run(state: &mut ChannelState, subs: &mut Subsystems) -> TickOutcome {
    state.tick_count += 1;
    poll_dtls_handshake(state);

    let inbound_pkt = pop_and_decrypt_inbound(state);
    if inbound_pkt.is_some() {
        state.ticks_without_rtp = 0;
    } else {
        state.ticks_without_rtp += 1;
    }

    if let Some(ref pk) = inbound_pkt {
        if classify_dtmf_inbound(state, subs, pk).await {
            // DTMF consumed the packet; skip audio processing this tick.
            return TickOutcome::Continue;
        }
    }

    let player_frame = tick_player(state, subs).await;
    let writer_frame = tick_writer(state, subs);
    // One slot, two possible sources — writer wins when both are set
    // (the Create handler also nixes any active player so this is
    // belt-and-braces).
    let out_frame = writer_frame.as_deref().or(player_frame.as_deref());

    let decoded = decode_inbound(state, inbound_pkt.as_ref());
    run_bargein(state, subs, decoded.as_deref()).await;
    accumulate_prebuffer(subs, decoded.as_deref());
    feed_recorders_and_readers(state, subs, decoded.as_deref(), out_frame).await;

    send_outbound(state, subs, out_frame, inbound_pkt.as_ref()).await;

    check_idle_timeout(state)
}

// ---- phase helpers --------------------------------------------------------

/// Pop one packet from the jitter buffer and SRTP-decrypt in place if an
/// inbound SRTP context is active. Returns `None` when the jitter is
/// empty *or* when decryption fails (bad MAC / replay) — in both cases
/// the tick treats this as "no inbound this cycle".
fn pop_and_decrypt_inbound(state: &mut ChannelState) -> Option<RtpPacket> {
    let mut pkt = state.jitter.lock().pop()?;
    if let Some(ref mut ctx) = state.srtp_decrypt {
        match ctx.decrypt_rtp(pkt.as_slice()) {
            Ok(decrypted) => {
                let n = decrypted.len().min(pkt.buf.len());
                pkt.buf[..n].copy_from_slice(&decrypted[..n]);
                pkt.len = n;
            }
            Err(_) => return None,
        }
    }
    Some(pkt)
}

/// Inspect `pk`'s payload type; if it's the rfc2833 DTMF PT, feed the
/// receiver, fire `telephone-event`, and if the player is interrupt-enabled
/// end it and activate any pending recorder. Returns `true` when the packet
/// was consumed as DTMF (audio processing should be skipped).
async fn classify_dtmf_inbound(
    state: &mut ChannelState,
    subs: &mut Subsystems,
    pk: &RtpPacket,
) -> bool {
    if pk.payload_type() != state.rfc2833_pt { return false; }
    let sn = pk.sequence_number();
    let ts = pk.timestamp();
    let payload = &pk.as_slice()[rtp::RTP_FIXED_HEADER_LEN..pk.len()];
    if let Some(digit) = subs.dtmf_recv.feed(sn, ts, payload) {
        let mut activate_recorder = false;
        if let Some(p) = subs.player.as_ref() {
            if p.interrupts() {
                subs.player = None;
                subs.bargein = None;
                state.pending_events.push(Event::Play {
                    state: super::actor::PlayState::End,
                    reason: Some("telephone-event".into()),
                });
                activate_recorder = true;
            }
        }
        state.pending_events.push(Event::TelephoneEvent { digit });
        if activate_recorder {
            let _ = activate_pending_recorder(subs, &mut state.pending_events).await;
        }
    }
    true
}

/// Read the next 160 samples from the active player, if any. If the
/// player finishes this tick, tear it down, emit `play/end`, and activate
/// any recorder queued by a prior `playrecord`. Returns the frame so
/// downstream phases (recorder write, outbound send) can use it.
async fn tick_player(state: &mut ChannelState, subs: &mut Subsystems) -> Option<Vec<i16>> {
    let mut player_frame: Option<Vec<i16>> = None;
    let mut player_just_ended = false;
    if let Some(player) = subs.player.as_mut() {
        let frame = player.read(FRAME_SAMPLES).await;
        if !frame.samples.is_empty() {
            player_frame = Some(frame.samples);
        }
        if player.is_finished() {
            subs.player = None;
            subs.bargein = None;
            state.pending_events.push(Event::Play {
                state: super::actor::PlayState::End,
                reason: Some("completed".into()),
            });
            player_just_ended = true;
        }
    }
    if player_just_ended {
        // Ordered after the `play/end` event so downstream sees play-end
        // then recording-start in the right sequence.
        let _ = activate_pending_recorder(subs, &mut state.pending_events).await;
    }
    player_frame
}

/// Pull the next 20 ms frame from an active write stream. Sibling of
/// `tick_player` — same role (outbound source) but fed from a JS
/// `Writable` rather than a WAV file. Returns None on underrun (tick
/// will emit silence) and tears the writer down when the JS side has
/// ended AND we've drained the last partial frame.
fn tick_writer(state: &mut ChannelState, subs: &mut Subsystems) -> Option<Vec<i16>> {
    let frame = {
        let w = subs.writer.as_mut()?;
        w.next_frame_8k()
    };
    // Take-out check: if the writer reports drained+ended, retire it and
    // emit a `play/end` so the JS event sink sees symmetry with `play`.
    let drained = subs.writer.as_ref().is_some_and(|w| w.is_drained_and_ended());
    if drained {
        subs.writer = None;
        state.pending_events.push(Event::Play {
            state: super::actor::PlayState::End,
            reason: Some("completed".into()),
        });
    }
    frame
}

/// Feed the wire bytes into `codecx` and pull the narrowband 8 kHz linear
/// representation. Returns `None` when there's no inbound packet this tick.
/// The result is owned so multiple downstream phases can share it without
/// holding a `&mut codecx` borrow.
fn decode_inbound(state: &mut ChannelState, pkt: Option<&RtpPacket>) -> Option<Vec<i16>> {
    let pk = pkt?;
    state.codecx.feed_wire(pk.payload_type(), pk.payload());
    state.codecx.require_narrowband_8k().map(|s| s.to_vec())
}

/// Smoothed-RMS barge-in check. Only runs when a player is active *and*
/// we've seen enough inbound packets for the moving average to have
/// converged (100 packets ≈ 2 s — same constant the recorder uses).
async fn run_bargein(
    state: &mut ChannelState,
    subs: &mut Subsystems,
    decoded: Option<&[i16]>,
) {
    let Some(samples) = decoded else { return; };
    let Some(bi) = subs.bargein.as_mut() else { return; };
    if subs.player.is_none() { return; }
    if state.in_count.load(Ordering::Relaxed) < 100 { return; }

    let mut sum_sq: u64 = 0;
    for s in samples { let v = *s as i64; sum_sq += (v * v) as u64; }
    let rms = if samples.is_empty() { 0 }
              else { ((sum_sq / samples.len() as u64) as f64).sqrt() as i32 };
    let smoothed = bi.power_ma.execute(rms.min(i16::MAX as i32) as i16) as i32;
    if smoothed <= bi.power_threshold { return; }

    subs.player = None;
    subs.bargein = None;
    state.pending_events.push(Event::Play {
        state: super::actor::PlayState::End,
        reason: Some("interrupted".into()),
    });
    let _ = activate_pending_recorder(subs, &mut state.pending_events).await;
}

/// During the prompt phase of a `playrecord` (player active *and*
/// recorder pending), accumulate inbound samples into the pre-buffer so
/// they can be flushed into the recorder on activation — captures the
/// caller's speech that started before the prompt finished.
fn accumulate_prebuffer(subs: &mut Subsystems, decoded: Option<&[i16]>) {
    if subs.player.is_none() || subs.pending_recorder.is_none() { return; }
    let Some(samples) = decoded else { return; };

    let total = subs.prebuffer.len() + samples.len();
    if total > PREBUFFER_CAPACITY_SAMPLES {
        let drop_n = total - PREBUFFER_CAPACITY_SAMPLES;
        for _ in 0..drop_n.min(subs.prebuffer.len()) {
            subs.prebuffer.pop_front();
        }
    }
    for s in samples { subs.prebuffer.push_back(*s); }
}

/// Resolve inbound / outbound sample slices for the tick (same L/R
/// convention C++ uses for the WAV recorder), then feed every recorder
/// and every audio reader. Garbage-collect readers whose JS consumer has
/// gone away.
///
/// Recorder vs reader semantics:
///   - **Recorder**: writes bytes only when there's *actual* audio
///     (inbound or player). Idle ticks are no-ops, so a channel that's
///     just holding open doesn't inflate the WAV file. Matches C++.
///   - **Reader**: fed every tick, including silence, so STT / caption
///     consumers get a continuous 20 ms stream — their FFT / VAD / ASR
///     pipelines rely on steady framing.
async fn feed_recorders_and_readers(
    state: &mut ChannelState,
    subs: &mut Subsystems,
    decoded: Option<&[i16]>,
    player_frame: Option<&[i16]>,
) {
    // Matches C++ `projectrtpsoundfile.cpp:760` — incodec = inbound
    // decoded, outcodec = player → else echo → else silence. Mono
    // recorders sum both sides; stereo interleaves L=in, R=out.
    let silence = [0i16; FRAME_SAMPLES];
    let in_s: &[i16] = decoded.unwrap_or(&silence);
    let out_s: &[i16] = if let Some(pf) = player_frame {
        pf
    } else if state.echo {
        in_s
    } else {
        &silence
    };

    // Recorder writes only when there's real audio. `state.echo` without
    // inbound produces no samples, so it doesn't qualify.
    let has_recordable_audio = decoded.is_some() || player_frame.is_some();
    if has_recordable_audio {
        write_recorder_frames(state, subs, in_s, out_s).await;
    }
    feed_readers(state, subs, in_s, out_s);
}

async fn write_recorder_frames(
    state: &mut ChannelState,
    subs: &mut Subsystems,
    in_s: &[i16],
    out_s: &[i16],
) {
    let chan_in_count = state.in_count.load(Ordering::Relaxed);
    let len = in_s.len().max(out_s.len());

    let mut i = 0;
    while i < subs.recorders.len() {
        let rec = &mut subs.recorders[i];
        let prev_state = rec.state();
        let frame = build_recorder_frame(rec.num_channels(), in_s, out_s, len);
        // Power calc runs on the inbound narrowband only — matches C++
        // `codecx::power()` which operates on the 160-sample mono slice
        // rather than the interleaved stereo frame.
        let _ = rec.write_frame(&frame, in_s, Some(chan_in_count)).await;

        let new_state = rec.state();
        let file_str = rec.file().to_string_lossy().into_owned();
        if prev_state == super::recorder::RecorderState::Pending
            && new_state == super::recorder::RecorderState::Active
        {
            state.pending_events.push(Event::Record {
                state: super::actor::RecordState::Recording,
                reason: Some("abovepower".into()),
                file: Some(file_str.clone()),
                filesize: None,
            });
        }
        if rec.is_finished() {
            let reason_str = match rec.finish_reason() {
                Some(super::recorder::FinishReason::Completed) => "completed",
                Some(super::recorder::FinishReason::MaxDurationReached) => "timeout",
                Some(super::recorder::FinishReason::BelowPowerThreshold) => "belowpower",
                Some(super::recorder::FinishReason::ChannelClosed) => "channelclosed",
                Some(super::recorder::FinishReason::Requested) => "requested",
                None => "completed",
            };
            let size = rec.file_size();
            state.pending_events.push(Event::Record {
                state: super::actor::RecordState::Finished,
                reason: Some(reason_str.into()),
                file: Some(file_str),
                filesize: Some(size),
            });
            subs.recorders.remove(i);
            continue;
        }
        i += 1;
    }
}

/// Build one WAV frame — mono = saturated sum, stereo = interleaved L=in R=out.
fn build_recorder_frame(num_channels: u16, in_s: &[i16], out_s: &[i16], len: usize) -> Vec<i16> {
    if num_channels == 2 {
        let mut v = Vec::with_capacity(len * 2);
        for j in 0..len {
            v.push(in_s.get(j).copied().unwrap_or(0));
            v.push(out_s.get(j).copied().unwrap_or(0));
        }
        v
    } else {
        (0..len).map(|j| {
            let a = in_s.get(j).copied().unwrap_or(0) as i32;
            let b = out_s.get(j).copied().unwrap_or(0) as i32;
            (a + b).clamp(i16::MIN as i32, i16::MAX as i32) as i16
        }).collect()
    }
}

fn feed_readers(
    state: &mut ChannelState,
    subs: &mut Subsystems,
    in_s: &[i16],
    out_s: &[i16],
) {
    for reader in subs.readers.iter_mut() {
        reader.feed(&mut state.codecx, Some(in_s), Some(out_s));
    }
    subs.readers.retain(|r| !r.is_closed());
}

/// Priority: DTMF outbound → player frame → echo → nothing. Matches C++
/// `postreadcb` ordering — DTMF wins because the RFC-2833 burst has to
/// ship in 20 ms slots; player beats echo because an explicit prompt
/// always overrides reflection; echo only fires on the tick an inbound
/// packet arrives so it stays in lock-step with the source.
async fn send_outbound(
    state: &mut ChannelState,
    subs: &mut Subsystems,
    player_frame: Option<&[i16]>,
    inbound_pkt: Option<&RtpPacket>,
) {
    if !state.direction.send { return; }
    let Some(remote) = state.get_remote_addr() else { return; };

    if let Some((event, payload)) = subs.dtmf_send.next_event() {
        send_dtmf(state, event, &payload, remote).await;
    } else if let Some(samples) = player_frame {
        send_player_frame(state, samples, remote).await;
    } else if state.echo {
        if let Some(pk) = inbound_pkt {
            send_echo(state, pk, remote).await;
        }
    }
}

/// Multi-tier idle check — matches C++ `checkidlerecv` in
/// `projectrtpchannel.cpp`. Three regimes:
///   * recv=true + remote_confirmed : soft 20 s ceiling on no-RTP
///   * recv=true + not confirmed    : hard 1 h zombie timeout
///   * recv=false (on hold)         : hard 2 h timeout
fn check_idle_timeout(state: &ChannelState) -> TickOutcome {
    if state.direction.recv {
        if state.remote_confirmed {
            if state.ticks_without_rtp >= IDLE_TICK_LIMIT {
                return TickOutcome::Stop;
            }
        } else if state.tick_count >= HARD_TIMEOUT_NO_REMOTE {
            return TickOutcome::Stop;
        }
    } else if state.tick_count >= HARD_TIMEOUT_NO_RECV {
        return TickOutcome::Stop;
    }
    TickOutcome::Continue
}

// ---- DTLS → SRTP plumbing -------------------------------------------------

/// Pick up the DTLS handshake result (if it's arrived) and build the
/// inbound / outbound SRTP contexts. Non-blocking `try_recv` — cheap to
/// call every tick. Must be called from whichever tick loop owns the
/// channel (Local `tick::run`, Mixer `mix_tick`) or the SRTP contexts
/// never get built and audio is silent after handshake.
pub(crate) fn poll_dtls_handshake(state: &mut ChannelState) {
    if let Some(rx) = state.dtls_result_rx.as_mut() {
        match rx.try_recv() {
            Ok(Some(result)) => {
                let keys = super::dtls_session::split_keying_material(
                    &result.keying_material, result.profile, result.is_client,
                );
                let (our_key, our_salt) = if keys.local_is_server {
                    (&keys.server_write_key, &keys.server_write_salt)
                } else {
                    (&keys.client_write_key, &keys.client_write_salt)
                };
                let (their_key, their_salt) = if keys.local_is_server {
                    (&keys.client_write_key, &keys.client_write_salt)
                } else {
                    (&keys.server_write_key, &keys.server_write_salt)
                };
                if let Ok(enc) = webrtc_srtp::context::Context::new(
                    our_key, our_salt, keys.profile, None, None,
                ) { state.srtp_encrypt = Some(enc); }
                if let Ok(dec) = webrtc_srtp::context::Context::new(
                    their_key, their_salt, keys.profile, None, None,
                ) { state.srtp_decrypt = Some(dec); }
                state.srtp_keys = Some(keys);
                state.dtls_result_rx = None;
            }
            Ok(None) => { state.dtls_result_rx = None; }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                state.dtls_result_rx = None;
            }
        }
    }
}

// ---- outbound send primitives --------------------------------------------

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

async fn send_dtmf(state: &mut ChannelState, _event: u8, payload: &[u8; 4], remote: SocketAddr) {
    let mut out = RtpPacket::new();
    out.init(state.ssrc);
    out.set_payload_type(state.rfc2833_pt);
    out.set_sequence_number(state.out_sn);
    out.set_timestamp(state.out_ts);
    out.set_payload(payload);
    state.out_sn = state.out_sn.wrapping_add(1);
    send_rtp(state, &out, remote).await;
}

async fn send_player_frame(state: &mut ChannelState, samples: &[i16], remote: SocketAddr) {
    // Feed linear samples into the codec bundle, lazily produce the
    // wire encoding for this channel's remote PT. The encoder state
    // (G.722 predictor, iLBC LP, etc.) lives on the bundle and
    // persists across ticks for a coherent outbound stream.
    state.codecx.feed_linear_8k(samples);
    let pt = state.remote_pt;
    let Some(payload) = state.codecx.require_wire_as(pt).map(|b| b.to_vec()) else { return; };
    let mut out = RtpPacket::new();
    out.init(state.ssrc);
    out.set_payload_type(state.remote_pt);
    out.set_sequence_number(state.out_sn);
    out.set_timestamp(state.out_ts);
    out.set_payload(&payload);
    state.out_sn = state.out_sn.wrapping_add(1);
    state.out_ts = state.out_ts.wrapping_add(samples.len() as u32);
    send_rtp(state, &out, remote).await;
}

async fn send_echo(state: &mut ChannelState, in_pk: &RtpPacket, remote: SocketAddr) {
    let mut out = RtpPacket::new();
    out.init(state.ssrc);
    out.set_payload_type(in_pk.payload_type());
    out.set_sequence_number(state.out_sn);
    out.set_timestamp(in_pk.timestamp());
    out.set_payload(in_pk.payload());
    state.out_sn = state.out_sn.wrapping_add(1);
    send_rtp(state, &out, remote).await;
}

impl RtpPacket {
    pub fn as_mut_slice_for_fill(&mut self, n: usize) -> &mut [u8] {
        debug_assert!(n <= self.buf.capacity());
        self.len = n;
        &mut self.buf[..n]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::commands::Direction;
    use crate::channel::state::ChannelState;
    use tokio::net::UdpSocket;

    async fn fresh_state() -> (ChannelState, UdpSocket, SocketAddr) {
        let rtp_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let rtp_addr = rtp_sock.local_addr().unwrap();
        let rtcp_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer_addr = peer_sock.local_addr().unwrap();
        let mut state = ChannelState::new(1, rtp_addr, rtp_sock, rtcp_sock, 0xC0FFEE);
        state.direction = Direction { send: true, recv: true };
        (state, peer_sock, peer_addr)
    }

    #[tokio::test]
    async fn tick_skips_send_without_remote() {
        let (mut state, peer_sock, _peer_addr) = fresh_state().await;
        let mut subs = crate::channel::actor::Subsystems::default();
        run(&mut state, &mut subs).await;

        let r = tokio::time::timeout(
            std::time::Duration::from_millis(30),
            peer_sock.recv_from(&mut [0u8; 2000]),
        ).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn tick_respects_direction_send_false() {
        let (mut state, peer_sock, peer_addr) = fresh_state().await;
        let mut subs = crate::channel::actor::Subsystems::default();
        state.set_remote_addr(peer_addr);
        state.direction.send = false;

        run(&mut state, &mut subs).await;

        let r = tokio::time::timeout(
            std::time::Duration::from_millis(30),
            peer_sock.recv_from(&mut [0u8; 2000]),
        ).await;
        assert!(r.is_err());
    }
}
