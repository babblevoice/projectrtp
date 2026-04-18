// Per-tick pipeline — the core of the channel actor in Local mode.
//
// The recv_loop task reads the socket continuously and pushes RTP/DTMF
// packets to the jitter buffer. This tick pops one per 20ms cycle,
// decodes, runs subsystems (recorder, player, barge-in), and sends
// outbound (echo, player, DTMF, silence).

use std::net::SocketAddr;
use std::sync::atomic::Ordering;

use super::actor::{Event, Subsystems};
use super::rtp::{self, RtpPacket};
use super::state::ChannelState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    Continue,
    Handshaking,
    Stop,
}

const IDLE_TICK_LIMIT: u64 = 50 * 20;

pub async fn run(state: &mut ChannelState, subs: &mut Subsystems) -> TickOutcome {
    state.tick_count += 1;

    // Check if DTLS handshake completed since last tick.
    poll_dtls_handshake(state);

    // Pop from jitter (filled continuously by recv_loop).
    let mut inbound_pkt = state.jitter.lock().pop();
    if inbound_pkt.is_some() {
        state.ticks_without_rtp = 0;
    } else {
        state.ticks_without_rtp += 1;
    }

    // SRTP decrypt if active — strip auth tag and decrypt payload in-place.
    if let (Some(ref mut pk), Some(ref mut ctx)) = (&mut inbound_pkt, &mut state.srtp_decrypt) {
        match ctx.decrypt_rtp(pk.as_slice()) {
            Ok(decrypted) => {
                let n = decrypted.len().min(pk.buf.len());
                pk.buf[..n].copy_from_slice(&decrypted[..n]);
                pk.len = n;
            }
            Err(_) => { inbound_pkt = None; }
        }
    }

    // DTMF classification at pop time — recv_loop pushes all RTP/DTMF
    // into jitter; we check PT here since we need access to Subsystems.
    if let Some(ref pk) = inbound_pkt {
        let pt = pk.payload_type();
        if pt == state.rfc2833_pt {
            let sn = pk.sequence_number();
            let payload = &pk.as_slice()[rtp::RTP_FIXED_HEADER_LEN..pk.len()];
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
            // DTMF consumed — don't process as audio.
            return TickOutcome::Continue;
        }
    }

    // Player.
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

    // Recorder + barge-in.
    if let Some(in_pk) = inbound_pkt.as_ref() {
        let in_pt = in_pk.payload_type();
        let payload = in_pk.payload();
        let decoded = state.transcoder.decode(in_pt, payload);

        if let (Some(samples), Some(bi)) = (decoded.as_ref(), subs.bargein.as_mut()) {
            if subs.player.is_some() && state.in_count.load(Ordering::Relaxed) >= 100 {
                let mut sum_sq: u64 = 0;
                for s in samples { let v = *s as i64; sum_sq += (v * v) as u64; }
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

    // Outbound.
    let remote = state.get_remote_addr();
    if state.direction.send && remote.is_some() {
        let remote = remote.unwrap();
        if let Some((event, payload)) = subs.dtmf_send.next_event() {
            send_dtmf(state, event, &payload, remote).await;
        } else if let Some(samples) = player_frame.as_ref() {
            send_player_frame(state, samples, remote).await;
        } else if state.echo {
            if let Some(in_pk) = inbound_pkt.as_ref() {
                send_echo(state, in_pk, remote).await;
            }
        }
    }

    // Idle timeout — matches C++ tickswithnortpcount: after 20s of no
    // inbound RTP (with remote confirmed), close the channel.
    if state.remote_confirmed && state.ticks_without_rtp >= IDLE_TICK_LIMIT {
        return TickOutcome::Stop;
    }

    TickOutcome::Continue
}

fn poll_dtls_handshake(state: &mut ChannelState) {
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
    let Some(payload) = state.transcoder.encode(state.remote_pt, samples) else { return; };
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
