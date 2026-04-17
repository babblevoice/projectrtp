// Per-tick pipeline — the core of the channel actor in Local mode.
//
// Mirrors the order of C++ `projectrtpchannel::handletick()` (channel.cpp:558).
// Each call is one 20 ms step:
//
//   1. Close-requested check — handled by the actor loop, not here.
//   2. DTLS handshake step (bail early if still negotiating).
//   3. Drain inbound UDP socket: classify STUN / DTLS / RTP.
//   4. For RTP: feed jitter buffer.
//   5. Peek jitter; decode; hand samples to recorders; DTMF detect.
//   6. Generate outbound frame: echo / player / silence.
//   7. Encode → SRTP-protect → UDP send.
//   8. Bookkeeping: tick_count, idle timeout, counters.
//
// Mix-group behaviour is owned by `channel/mixer.rs`: when a channel joins a
// mix, its state + subs migrate into the mixer actor which runs its own tick
// loop. Only Local (unmixed) channels end up in this file.

use std::io::ErrorKind;
use std::net::SocketAddr;

use super::actor::{Event, Subsystems};
use super::rtp::{self, RtpPacket};
use super::state::ChannelState;
use crate::stun;

/// Outcome of a tick — used by the actor to decide whether to keep running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    /// Continue ticking.
    Continue,
    /// DTLS is mid-handshake; skipped the media pipeline this tick.
    Handshaking,
    /// Idle-timeout reached or fatal error — actor should close.
    Stop,
}

const IDLE_TICK_LIMIT: u64 = 50 * 20; // 20 s @ 20 ms/tick, matches C++ line 528.
/// Matches the C++ handletick behavior of consuming roughly one RTP source
/// packet per tick. A burst that arrives inside a single 20 ms window fills
/// the OS socket buffer; excess packets are dropped by the kernel. The
/// "stalled connection" test depends on this drop behavior being visible as
/// TS gaps in the echoed stream.
const MAX_INBOUND_PER_TICK: usize = 2;

pub async fn run(state: &mut ChannelState, subs: &mut Subsystems) -> TickOutcome {
    state.tick_count += 1;

    // 3+4. Drain inbound UDP, route by packet type. Inbound DTMF is consumed
    // here and routed to subs.dtmf_recv → state.pending_events.
    drain_inbound(state, subs).await;

    // 5. Consume next in-order RTP from jitter. in_count is incremented at
    // receive time (not here) so packets still buffered at close still count.
    let inbound_pkt = state.jitter.pop();

    // 5a. Advance the player one frame's worth (20 ms @ 8 kHz = 160 samples).
    let mut player_frame: Option<Vec<i16>> = None;
    if let Some(player) = subs.player.as_mut() {
        let frame = player.read(160).await;
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
        }
    }

    // 5b. Recorder: feed each active recorder the popped packet's decoded
    // samples. Each recorder handles its own state transitions (gate, finish
    // thresholds, max duration).
    if let Some(in_pk) = inbound_pkt.as_ref() {
        let in_pt = in_pk.payload_type();
        let payload = in_pk.payload();
        let decoded = state.transcoder.decode(in_pt, payload);

        // Barge-in: RMS of inbound samples, smoothed across N packets.
        if let (Some(samples), Some(bi)) = (decoded.as_ref(), subs.bargein.as_mut()) {
            if subs.player.is_some() && state.in_count >= 100 {
                let mut sum_sq: u64 = 0;
                for s in samples {
                    let v = *s as i64;
                    sum_sq += (v * v) as u64;
                }
                let rms = if samples.is_empty() { 0 }
                          else { ((sum_sq / samples.len() as u64) as f64).sqrt() as i32 };
                let smoothed = bi.power_ma.execute(rms.min(i16::MAX as i32) as i16) as i32;
                if smoothed > bi.power_threshold {
                    subs.player = None;
                    subs.bargein = None;
                    state.pending_events.push(Event::Play {
                        state: super::actor::PlayState::End,
                        reason: Some("interrupted".into()),
                    });
                }
            }
        }
        // We iterate by index so we can remove finished recorders without
        // holding a borrow across the emit.
        let mut i = 0;
        while i < subs.recorders.len() {
            let rec = &mut subs.recorders[i];
            let prev_state = rec.state();
            if let Some(samples) = &decoded {
                let frame = if rec.num_channels() == 2 {
                    let mut inter = Vec::with_capacity(samples.len() * 2);
                    for s in samples { inter.push(*s); inter.push(*s); }
                    inter
                } else {
                    samples.clone()
                };
                let _ = rec.write(&frame).await;
            }
            let new_state = rec.state();
            let file_str = rec.file().to_string_lossy().into_owned();

            if prev_state == super::recorder::RecorderState::Pending
                && new_state == super::recorder::RecorderState::Active
            {
                state.pending_events.push(Event::Record {
                    state: super::actor::RecordState::Recording,
                    reason: Some("abovepower".into()),
                    file: Some(file_str.clone()),
                });
            }

            if rec.is_finished() {
                let reason = rec.finish_reason().cloned();
                let reason_str = match reason {
                    Some(super::recorder::FinishReason::Completed) => "completed",
                    Some(super::recorder::FinishReason::MaxDurationReached) => "timeout",
                    Some(super::recorder::FinishReason::BelowPowerThreshold) => "belowpower",
                    Some(super::recorder::FinishReason::ChannelClosed) => "channelclosed",
                    Some(super::recorder::FinishReason::Requested) => "requested",
                    None => "completed",
                };
                state.pending_events.push(Event::Record {
                    state: super::actor::RecordState::Finished,
                    reason: Some(reason_str.into()),
                    file: Some(file_str),
                });
                subs.recorders.remove(i);
                continue;
            }
            i += 1;
        }
    }

    // 6+7. Outbound. Priority: dtmf → player → echo → silence.
    if state.direction.send && state.remote_addr.is_some() {
        if let Some((event, payload)) = subs.dtmf_send.next_event() {
            send_dtmf(state, event, &payload).await;
        } else if let Some(samples) = player_frame.as_ref() {
            send_player_frame(state, samples).await;
        } else if state.echo {
            if let Some(in_pk) = inbound_pkt.as_ref() {
                send_echo(state, in_pk).await;
            }
        }
        // Otherwise: stay silent.
    }

    // 8. Idle timeout — only when not actively receiving.
    if state.tick_count >= IDLE_TICK_LIMIT && state.in_count == 0 {
        return TickOutcome::Stop;
    }

    TickOutcome::Continue
}

async fn drain_inbound(state: &mut ChannelState, subs: &mut Subsystems) {
    let mut scratch = [0u8; rtp::RTP_MAX_LENGTH];
    for _ in 0..MAX_INBOUND_PER_TICK {
        match state.rtp_sock.try_recv_from(&mut scratch) {
            Ok((n, peer)) => {
                // Autocorrect: latch onto the observed remote (C++ NAT-
                // hairpin behaviour).
                state.remote_addr = Some(peer);
                classify_and_route(state, subs, &scratch[..n], peer).await;
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
}

async fn classify_and_route(
    state: &mut ChannelState,
    subs: &mut Subsystems,
    pkt: &[u8],
    peer: SocketAddr,
) {
    if pkt.is_empty() { return; }

    if stun::is_stun(pkt) {
        if state.local_icepwd.is_empty() { return; }
        let key = state.local_icepwd.as_bytes();
        let mut req = pkt.to_vec();
        let mut resp = [0u8; rtp::RTP_MAX_LENGTH];
        let n = stun::handle(&mut req, &mut resp, peer, key, key);
        if n > 0 {
            let _ = state.rtp_sock.send_to(&resp[..n], peer).await;
        }
        return;
    }

    let first = pkt[0];
    if (20..=23).contains(&first) {
        // TODO: forward to DtlsSession.step_incoming and drain outgoing.
        return;
    }

    if pkt.len() < rtp::RTP_FIXED_HEADER_LEN { return; }

    state.in_count += 1;

    // RFC 2833 telephone events: decode → emit "telephone-event".
    let pt = rtp::payload_type(pkt);
    if pt == state.rfc2833_pt {
        let sn = rtp::sequence_number(pkt);
        let payload = &pkt[rtp::RTP_FIXED_HEADER_LEN..];
        if let Some(digit) = subs.dtmf_recv.feed(sn, payload) {
            if let Some(p) = subs.player.as_ref() {
                if p.interrupts() {
                    subs.player = None;
                    state.pending_events.push(Event::Play {
                        state: super::actor::PlayState::End,
                        reason: Some("telephone-event".into()),
                    });
                }
            }
            state.pending_events.push(Event::TelephoneEvent { digit });
        }
        return;
    }

    let mut rp = RtpPacket::new();
    rp.as_mut_slice_for_fill(pkt.len()).copy_from_slice(pkt);
    state.jitter.push(rp);
}

async fn send_dtmf(state: &mut ChannelState, _event: u8, payload: &[u8; 4]) {
    let Some(remote) = state.remote_addr else { return; };
    let mut out = RtpPacket::new();
    out.init(state.ssrc);
    out.set_payload_type(state.rfc2833_pt);
    out.set_sequence_number(state.out_sn);
    out.set_timestamp(state.out_ts);
    out.set_payload(payload);
    state.out_sn = state.out_sn.wrapping_add(1);
    if state.rtp_sock.send_to(out.as_slice(), remote).await.is_ok() {
        state.out_count += 1;
    }
}

/// Encode a player frame to the remote codec and send it as one RTP packet.
async fn send_player_frame(state: &mut ChannelState, samples: &[i16]) {
    let Some(remote) = state.remote_addr else { return; };
    let Some(payload) = state.transcoder.encode(state.remote_pt, samples) else { return; };
    let mut out = RtpPacket::new();
    out.init(state.ssrc);
    out.set_payload_type(state.remote_pt);
    out.set_sequence_number(state.out_sn);
    out.set_timestamp(state.out_ts);
    out.set_payload(&payload);
    state.out_sn = state.out_sn.wrapping_add(1);
    state.out_ts = state.out_ts.wrapping_add(samples.len() as u32);
    if state.rtp_sock.send_to(out.as_slice(), remote).await.is_ok() {
        state.out_count += 1;
    }
}

async fn send_echo(state: &mut ChannelState, in_pk: &RtpPacket) {
    let Some(remote) = state.remote_addr else { return; };
    let mut out = RtpPacket::new();
    out.init(state.ssrc);
    out.set_payload_type(in_pk.payload_type());
    out.set_sequence_number(state.out_sn);
    out.set_timestamp(in_pk.timestamp());
    let payload = in_pk.payload();
    out.set_payload(payload);
    state.out_sn = state.out_sn.wrapping_add(1);
    if state.rtp_sock.send_to(out.as_slice(), remote).await.is_ok() {
        state.out_count += 1;
    }
}

// Small extension on RtpPacket used above — lets tick.rs copy a full datagram
// into the packet buffer and set the live length in one call without poking
// RtpPacket's internals.
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
    use crate::channel::jitter::JitterBuffer;
    use crate::channel::state::ChannelState;
    use tokio::net::UdpSocket;

    async fn socket_on_loopback() -> (UdpSocket, SocketAddr) {
        let s = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let a = s.local_addr().unwrap();
        (s, a)
    }

    async fn fresh_state() -> (ChannelState, SocketAddr, UdpSocket, SocketAddr) {
        let (rtp_sock, rtp_addr) = socket_on_loopback().await;
        let (rtcp_sock, _) = socket_on_loopback().await;
        let (peer_sock, peer_addr) = socket_on_loopback().await;
        let mut state = ChannelState::new(1, rtp_addr, rtp_sock, rtcp_sock, 0xC0FFEE);
        state.jitter = JitterBuffer::new(32, 2);
        state.direction = Direction { send: true, recv: true };
        (state, rtp_addr, peer_sock, peer_addr)
    }

    #[tokio::test]
    async fn tick_skips_send_without_remote() {
        let (mut state, _rtp_addr, peer_sock, _peer_addr) = fresh_state().await;
        let mut subs = crate::channel::actor::Subsystems::default();
        run(&mut state, &mut subs).await;

        let r = tokio::time::timeout(
            std::time::Duration::from_millis(30),
            peer_sock.recv_from(&mut [0u8; 2000]),
        ).await;
        assert!(r.is_err(), "should not have received a packet without remote");
    }

    #[tokio::test]
    async fn tick_respects_direction_send_false() {
        let (mut state, _, peer_sock, peer_addr) = fresh_state().await;
        let mut subs = crate::channel::actor::Subsystems::default();
        state.remote_addr = Some(peer_addr);
        state.direction.send = false;

        run(&mut state, &mut subs).await;

        let r = tokio::time::timeout(
            std::time::Duration::from_millis(30),
            peer_sock.recv_from(&mut [0u8; 2000]),
        ).await;
        assert!(r.is_err());
    }
}
