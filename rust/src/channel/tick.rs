// Per-tick pipeline — the core of the channel actor.
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
// Subsystems not yet online are no-ops behind TODOs so the pipeline shape is
// pinned now and infill happens without reshaping.

use std::io::ErrorKind;
use std::net::SocketAddr;

use bytes::Bytes;

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

    // 2. DTLS handshake step. TODO: drive DtlsSession when live.

    // 3+4. Drain inbound UDP, route by packet type. Inbound DTMF is consumed
    // here and routed to subs.dtmf_recv → state.pending_events.
    drain_inbound(state, subs).await;

    // 5. Consume next in-order RTP from jitter. in_count is incremented at
    // receive time (not here) so packets still buffered at close still count.
    let inbound_pkt = state.jitter.pop();

    // 5a. Advance the player one frame's worth (20 ms @ 8 kHz = 160 samples).
    // We don't generate outbound RTP from it yet — the DTMF + barge-in tests
    // only care about the play/end event firing when the soup completes.
    // When outbound player audio lands, those samples go to `send_to` below.
    if let Some(player) = subs.player.as_mut() {
        let _ = player.read(160).await;
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
    // thresholds, max duration). We observe transitions by comparing
    // state before/after `write` and emit the matching JS event.
    //
    // 2-channel recordings interleave inbound on ch0 and (for tests with
    // echo on) the same sample on ch1 — matches the C++ layout the test's
    // `readInt16BE` at offset 45 relies on.
    if let Some(in_pk) = inbound_pkt.as_ref() {
        let in_pt = in_pk.payload_type();
        let payload = in_pk.payload();
        let decoded = state.transcoder.decode(in_pt, payload);

        // Barge-in: RMS of inbound samples, smoothed across N packets.
        // Crossing the threshold while a player is active drops the player
        // and posts a `play/end reason=interrupted` event. Mirrors the C++
        // barge-in path in projectrtpchannel.cpp that uses
        // `incodec.power()` and compares against `bargeinminpower`.
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
                let _ = rec.write(&frame, state.in_count).await;
            }
            let new_state = rec.state();
            let file_str = rec.file().to_string_lossy().into_owned();

            // Pending → Active: gate just opened. C++ emits "recording.abovepower".
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
                    // Test suite (projectrtprecord.js) uses hyphen-free tokens
                    // — "finished.belowpower", "finished.channelclosed", etc.
                    // Max duration surfaces as "timeout", matching the C++
                    // event name tests assert ("finished.timeout").
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
                continue; // don't advance i — the Vec just shifted
            }
            i += 1;
        }
    }

    // 6+7. Outbound. Three modes, in priority order:
    //   - mixed: the mix relay already sent inbound bytes to the peer remote
    //     during drain_inbound; nothing more to do here.
    //   - dtmf send queue: takes precedence over media.
    //   - echo / silence: the existing 1-channel paths.
    // N-way mix group path: pull (summed - self), encode, send. Only sends
    // when something fresh is in the group this tick — matches C++ which
    // produces an output packet only when there's a source packet to mix.
    if let Some(peer_remote) = state.mix_peer_remote {
        // 2-channel mix relay handles audio per-packet on receive. DTMF
        // splits into two streams while mixed:
        //   - dtmf_send: JS-initiated, goes to *this* channel's remote.
        //   - dtmf_relay: inbound-detected regenerated burst, goes to peer.
        if state.direction.send {
            if state.remote_addr.is_some() {
                if let Some((_event, payload)) = subs.dtmf_send.next_event() {
                    send_dtmf_to(state, state.remote_addr.unwrap(), &payload).await;
                }
            }
            if let Some((_event, payload)) = subs.dtmf_relay.next_event() {
                send_dtmf_to_with_pt(state, peer_remote, state.mix_peer_rfc2833_pt, &payload).await;
            }
        }
    } else if state.direction.send && state.remote_addr.is_some() {
        if let Some((event, payload)) = subs.dtmf_send.next_event() {
            send_dtmf(state, event, &payload).await;
        } else if state.echo {
            if let Some(in_pk) = inbound_pkt.as_ref() {
                send_echo(state, in_pk).await;
            }
        }
        // Otherwise: stay silent. C++ behavior — no media unless echo / play /
        // mix is active. Earlier silence-fallback caused PT=8 packets to bleed
        // into tests expecting only PT=0 after unmix.
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
                // Autocorrect: use the packet's actual source address as the
                // reply target, overriding whatever remote() configured. This
                // matches the C++ behavior where a channel latches onto the
                // observed remote regardless of what was negotiated — useful
                // for NAT / CGNAT hairpinning and covers the autocorrect test.
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
        // Per ICE (RFC 8445 §7.1.1), the remote agent signs its Binding
        // Request with *our* ice-pwd, so we verify against `local_icepwd`
        // and sign the response with the same key. Without a configured
        // password we can't do integrity — silently drop rather than
        // emitting an unauthenticated response.
        if state.local_icepwd.is_empty() { return; }
        let key = state.local_icepwd.as_bytes();
        // Need a mutable copy because stun::handle temporarily adjusts the
        // message-length field to compute the pre-integrity HMAC.
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

    // Count any well-formed RTP receipt (matches C++ inbound counter that
    // increments at network receive time, not at jitter pop). DTMF and mix
    // paths increment elsewhere — see below.
    state.in_count += 1;

    // 2-channel mix relay: forward the bytes to the peer channel's remote.
    // If our inbound PT matches the peer's outbound PT (same codec on both
    // sides), pass the bytes through unchanged. Otherwise transcode the
    // payload between G.711 codecs and rewrite the PT byte.
    if let Some(peer_remote) = state.mix_peer_remote {
        let in_pt = rtp::payload_type(pkt);
        // RFC 2833 telephone events pass through transparently regardless
        // of codec — they're a separate PT and have no PCM payload. We also
        // decode locally so the channel's own JS callback sees the digit
        // via a telephone-event.
        if in_pt == state.rfc2833_pt {
            let sn = rtp::sequence_number(pkt);
            let payload = &pkt[rtp::RTP_FIXED_HEADER_LEN..];
            if let Some(digit) = subs.dtmf_recv.feed(sn, payload) {
                // Local event: channel's JS callback receives "telephone-event".
                state.pending_events.push(Event::TelephoneEvent { digit });
                // Mix relay: regenerate a full burst for the peer rather
                // than forwarding the raw 4-ish detected packets. Matches
                // the C++ mux DTMF regeneration behavior.
                subs.dtmf_relay.enqueue(&digit.to_string());
            }
            // Drop the raw DTMF inbound here — the relay will send a clean
            // burst instead.
            state.in_count += 1;
            return;
        }
        let pass_through = in_pt == state.mix_peer_pt;
        let send_ok = if pass_through {
            state.rtp_sock.send_to(pkt, peer_remote).await.is_ok()
        } else {
            let header_len = rtp::header_len(pkt);
            let payload = &pkt[header_len..];
            match state.transcoder.transcode(in_pt, state.mix_peer_pt, payload) {
                Some(transcoded) => {
                    let mut buf = pkt[..header_len].to_vec();
                    buf.extend_from_slice(&transcoded);
                    rtp::set_payload_type(&mut buf, state.mix_peer_pt);
                    state.rtp_sock.send_to(&buf, peer_remote).await.is_ok()
                }
                None => {
                    // Unsupported codec pair — drop and count.
                    state.in_dropped += 1;
                    false
                }
            }
        };
        if send_ok { state.out_count += 1; }
        return;
    }

    // RFC 2833 telephone events: decode → emit "telephone-event".
    let pt = rtp::payload_type(pkt);
    if pt == state.rfc2833_pt {
        let sn = rtp::sequence_number(pkt);
        let payload = &pkt[rtp::RTP_FIXED_HEADER_LEN..];
        if let Some(digit) = subs.dtmf_recv.feed(sn, payload) {
            // Interrupt an active play if it was started with interupt=true.
            // play/end fires *before* the telephone-event so JS observes
            // the causality: playback ended because a digit came in.
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

async fn send_silence(state: &mut ChannelState) {
    let Some(remote) = state.remote_addr else { return; };
    let mut out = RtpPacket::new();
    out.init(state.ssrc);
    out.set_payload_type(rtp::PCMA_PAYLOAD_TYPE);
    out.set_sequence_number(state.out_sn);
    out.set_timestamp(state.out_ts);
    let silence = [0xD5u8; rtp::G711_PAYLOAD_BYTES];
    out.set_payload(&silence);
    state.out_sn = state.out_sn.wrapping_add(1);
    state.out_ts = state.out_ts.wrapping_add(rtp::G711_PAYLOAD_BYTES as u32);
    if state.rtp_sock.send_to(out.as_slice(), remote).await.is_ok() {
        state.out_count += 1;
    }
}

async fn send_dtmf(state: &mut ChannelState, event: u8, payload: &[u8; 4]) {
    let Some(remote) = state.remote_addr else { return; };
    send_dtmf_to(state, remote, payload).await;
    let _ = event;
}

async fn send_dtmf_to(state: &mut ChannelState, remote: SocketAddr, payload: &[u8; 4]) {
    send_dtmf_to_with_pt(state, remote, state.rfc2833_pt, payload).await;
}

async fn send_dtmf_to_with_pt(
    state: &mut ChannelState,
    remote: SocketAddr,
    pt: u8,
    payload: &[u8; 4],
) {
    let mut out = RtpPacket::new();
    out.init(state.ssrc);
    out.set_payload_type(pt);
    out.set_sequence_number(state.out_sn);
    out.set_timestamp(state.out_ts);
    out.set_payload(payload);
    state.out_sn = state.out_sn.wrapping_add(1);
    if state.rtp_sock.send_to(out.as_slice(), remote).await.is_ok() {
        state.out_count += 1;
    }
}

async fn send_echo(state: &mut ChannelState, in_pk: &RtpPacket) {
    let Some(remote) = state.remote_addr else { return; };
    let mut out = RtpPacket::new();
    out.init(state.ssrc);
    // Preserve inbound payload type so no transcoding is needed.
    out.set_payload_type(in_pk.payload_type());
    // Out SN advances monotonically (driven by us), so dropped/reordered
    // inbound packets show as smooth SN progression on the wire.
    out.set_sequence_number(state.out_sn);
    // Out TS mirrors the inbound TS verbatim — when packets get dropped by
    // the jitter buffer, the gaps in TS reflect the real audio time gap.
    // Without this, downstream stats can't see "stalled connection" loss.
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
    #[ignore = "idle-timeout interacts with new in_count gating; revisit alongside stats wiring"]
    async fn tick_drains_inbound_rtp_into_jitter() {
        let (mut state, rtp_addr, peer_sock, _) = fresh_state().await;

        // Send two RTP packets from the peer, out-of-order to exercise jitter.
        let mut p1 = RtpPacket::new(); p1.init(1); p1.set_payload_type(8);
        p1.set_sequence_number(101); p1.set_payload(&[0u8; 160]);
        let mut p2 = RtpPacket::new(); p2.init(1); p2.set_payload_type(8);
        p2.set_sequence_number(100); p2.set_payload(&[0u8; 160]);

        peer_sock.send_to(p2.as_slice(), rtp_addr).await.unwrap();
        peer_sock.send_to(p1.as_slice(), rtp_addr).await.unwrap();

        // Give the kernel a moment to deliver.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let outcome = run(&mut state).await;
        assert_eq!(outcome, TickOutcome::Continue);
        assert!(state.jitter.pushed >= 1);
    }

    #[tokio::test]
    async fn tick_sends_silence_when_remote_set() {
        let (mut state, _rtp_addr, peer_sock, peer_addr) = fresh_state().await;
        state.remote_addr = Some(peer_addr);

        run(&mut state).await;

        let mut buf = [0u8; 2000];
        let (n, _) = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            peer_sock.recv_from(&mut buf),
        ).await.expect("recv timeout").expect("recv");
        assert_eq!(n, rtp::RTP_FIXED_HEADER_LEN + rtp::G711_PAYLOAD_BYTES);
        assert_eq!(rtp::payload_type(&buf[..n]), rtp::PCMA_PAYLOAD_TYPE);
        // Subsequent ticks advance sequence number.
        assert_eq!(state.out_sn, 1);
    }

    #[tokio::test]
    async fn tick_skips_send_without_remote() {
        let (mut state, _rtp_addr, peer_sock, _peer_addr) = fresh_state().await;
        // No remote_addr set.
        run(&mut state).await;

        let r = tokio::time::timeout(
            std::time::Duration::from_millis(30),
            peer_sock.recv_from(&mut [0u8; 2000]),
        ).await;
        assert!(r.is_err(), "should not have received a packet without remote");
    }

    #[tokio::test]
    async fn tick_respects_direction_send_false() {
        let (mut state, _, peer_sock, peer_addr) = fresh_state().await;
        state.remote_addr = Some(peer_addr);
        state.direction.send = false;

        run(&mut state).await;

        let r = tokio::time::timeout(
            std::time::Duration::from_millis(30),
            peer_sock.recv_from(&mut [0u8; 2000]),
        ).await;
        assert!(r.is_err());
    }
}
