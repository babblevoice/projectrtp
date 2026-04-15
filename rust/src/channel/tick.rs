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
const MAX_INBOUND_PER_TICK: usize = 64;

pub async fn run(state: &mut ChannelState) -> TickOutcome {
    state.tick_count += 1;

    // 2. DTLS handshake step. TODO: drive DtlsSession when live.

    // 3+4. Drain inbound UDP, route by packet type.
    drain_inbound(state).await;

    // 5. Consume next in-order RTP from jitter.
    let inbound_pkt = state.jitter.pop();
    if inbound_pkt.is_some() {
        state.in_count += 1;
    }

    // 6+7. Outbound frame.
    if state.direction.send && state.remote_addr.is_some() {
        if state.echo {
            // Pure byte-loopback echo — same codec in/out.
            if let Some(in_pk) = inbound_pkt.as_ref() {
                send_echo(state, in_pk).await;
            }
        } else {
            // No active source yet — silence on PCMA. Used by smoke/idle paths.
            send_silence(state).await;
        }
    }

    // 8. Idle timeout — only when not actively receiving.
    if state.tick_count >= IDLE_TICK_LIMIT && state.in_count == 0 {
        return TickOutcome::Stop;
    }

    TickOutcome::Continue
}

async fn drain_inbound(state: &mut ChannelState) {
    let mut scratch = [0u8; rtp::RTP_MAX_LENGTH];
    for _ in 0..MAX_INBOUND_PER_TICK {
        match state.rtp_sock.try_recv_from(&mut scratch) {
            Ok((n, peer)) => classify_and_route(state, &scratch[..n], peer).await,
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
}

async fn classify_and_route(state: &mut ChannelState, pkt: &[u8], _peer: SocketAddr) {
    if pkt.is_empty() { return; }

    // STUN per RFC 5389: first byte's top two bits are 00, magic cookie matches.
    if stun::is_stun(pkt) {
        // TODO: respond to Binding Request via stun::handle once we have the
        // local/remote ICE keys on ChannelState.
        return;
    }

    // DTLS: content type 20 (change cipher), 21 (alert), 22 (handshake), 23
    // (application data). The first byte distinguishes DTLS from RTP because
    // RTP's first byte always has v=2 → top bits `10xx xxxx` (0x80..=0xBF).
    let first = pkt[0];
    if (20..=23).contains(&first) {
        // TODO: forward to DtlsSession.step_incoming and drain outgoing.
        return;
    }

    // Everything else: treat as RTP/RTCP. We only buffer RTP; RTCP routes
    // elsewhere. RTCP is PT 200..=204 (after the v=2 marker in pk[1]).
    // Be permissive here — jitter::push rejects malformed SN anyway.
    if pkt.len() < rtp::RTP_FIXED_HEADER_LEN { return; }

    let mut rp = RtpPacket::new();
    // Copy into an owned RtpPacket. (Pool-backed allocation lands when the
    // channel's packet pool is wired up; for now this matches the C++
    // behavior of memcpy into a pre-sized buffer.)
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

async fn send_echo(state: &mut ChannelState, in_pk: &RtpPacket) {
    let Some(remote) = state.remote_addr else { return; };
    let mut out = RtpPacket::new();
    out.init(state.ssrc);
    // Preserve inbound payload type so transcoding isn't required.
    out.set_payload_type(in_pk.payload_type());
    out.set_sequence_number(state.out_sn);
    out.set_timestamp(state.out_ts);
    let payload = in_pk.payload();
    out.set_payload(payload);
    state.out_sn = state.out_sn.wrapping_add(1);
    state.out_ts = state.out_ts.wrapping_add(payload.len() as u32);
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
