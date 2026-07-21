// Periodic RTCP transmission — the send counterpart to `rtcp_loop.rs`.
//
// `maybe_send_rtcp` is called once per member per tick from *both* tick loops
// (Local `tick::run` and Mixed `Member` post-mix housekeeping). On the report
// interval it assembles a compound datagram — an SR when we've sent audio,
// otherwise an RR, always with an SDES/CNAME — and sends it on the P+1 control
// socket to the derived RTCP remote. On a secure (DTLS-SRTP) channel the
// compound is protected as SRTCP first.
//
// Report timing is randomised per RFC 3550 §6.3.1 so reports from many
// channels don't synchronise. Full §6.2 multiparty reconsideration and
// bandwidth scaling are deliberately out of scope: projectrtp is strictly
// point-to-point (one local SSRC, one latched remote), so the member/sender
// and bandwidth terms stay below Tmin and Tmin is the effective interval — the
// part that matters here is the randomisation, which we do implement.

use std::net::SocketAddr;
use std::time::Instant;

use super::rtcp::{self, ReportBlock, SenderInfo};
use super::state::ChannelState;

/// Minimum report interval (RFC 3550 Tmin): 250 × 20 ms ≈ 5 s.
pub const RTCP_MIN_INTERVAL_TICKS: u64 = 250;

/// RFC 3550 §6.3.1 interval-compensation divisor (e/2 - ... ≈ 1.21828).
const RTCP_COMPENSATION: f64 = 1.21828;

/// Emit a periodic SR/RR + SDES when the (randomised) report interval elapses.
/// No-op off-interval or until a remote address is known.
pub async fn maybe_send_rtcp(state: &mut ChannelState) {
    // First call for the channel: schedule the initial report at *half* the
    // interval (§6.3.1 initial reconsideration) and return without sending.
    if state.rtcp_next_tick == 0 {
        let half = (next_interval_ticks(&mut state.rtcp_rng) / 2).max(1);
        state.rtcp_next_tick = state.tick_count + half;
        return;
    }
    if state.tick_count < state.rtcp_next_tick {
        return;
    }
    // Reschedule now, with fresh jitter, whether or not this report actually
    // goes out — otherwise, while we wait for a remote to be latched, the gate
    // would pass every tick and busy-build reports.
    state.rtcp_next_tick = state.tick_count + next_interval_ticks(&mut state.rtcp_rng);

    let Some(rtcp_remote) = rtcp_remote_addr(state) else {
        return;
    };
    let compound = build_report(state, false);
    send_compound(state, &compound, rtcp_remote).await;
}

/// Send a single BYE compound on channel close so the peer learns the stream
/// has ended now rather than via RTP timeout. Best-effort — a dropped BYE just
/// falls back to the peer's own idle teardown. No-op if no remote is known.
pub async fn send_bye(state: &mut ChannelState) {
    let Some(rtcp_remote) = rtcp_remote_addr(state) else {
        return;
    };
    let compound = build_report(state, true);
    send_compound(state, &compound, rtcp_remote).await;
}

/// The RTCP destination. Under rtcp-mux (RFC 5761) RTCP shares the RTP
/// 5-tuple, so it is the RTP peer itself; otherwise it is the RTP peer's IP
/// with port + 1 (symmetric RTCP on the separate control port).
fn rtcp_remote_addr(state: &ChannelState) -> Option<SocketAddr> {
    let r = state.get_remote_addr()?;
    if state.rtcpmux {
        Some(r)
    } else {
        Some(SocketAddr::new(r.ip(), r.port().wrapping_add(1)))
    }
}

/// Build a compound report: SR when we've sent audio, otherwise RR; always
/// with an SDES/CNAME and a reception report about the peer if we've latched
/// its stream. `bye` appends a BYE sub-packet.
fn build_report(state: &ChannelState, bye: bool) -> Vec<u8> {
    let now = Instant::now();
    let (ntp_sec, ntp_frac) = rtcp::ntp_now();

    let report = state.rx_stats.lock().report_block(now);
    let reports: &[ReportBlock] = match &report {
        Some(rb) => std::slice::from_ref(rb),
        None => &[],
    };

    if state.out_count > 0 {
        let info = SenderInfo {
            ntp_sec,
            ntp_frac,
            rtp_ts: state.out_ts,
            packet_count: state.out_count as u32,
            octet_count: state.out_octets as u32,
        };
        rtcp::build_compound(state.ssrc, Some(&info), reports, Some(&state.cname), bye)
    } else {
        rtcp::build_compound(state.ssrc, None, reports, Some(&state.cname), bye)
    }
}

/// Send a compound to `remote`, protecting it as SRTCP first on a secure
/// channel (same gate/context as the RTP send path). Under rtcp-mux the
/// datagram goes out on the RTP socket (shared 5-tuple), otherwise on the P+1
/// control socket.
async fn send_compound(state: &mut ChannelState, compound: &[u8], remote: SocketAddr) {
    // Clone the Arc up front so the socket borrow doesn't collide with the
    // mutable `state.srtp_encrypt` borrow below.
    let sock = if state.rtcpmux {
        state.rtp_sock.clone()
    } else {
        state.rtcp_sock.clone()
    };
    if let Some(ref mut ctx) = state.srtp_encrypt {
        if let Ok(protected) = ctx.encrypt_rtcp(compound) {
            let _ = sock.send_to(&protected, remote).await;
        }
    } else {
        let _ = sock.send_to(compound, remote).await;
    }
}

/// A randomised report interval in ticks: Tmin scaled uniformly over
/// [0.5, 1.5) then divided by the §6.3.1 compensation factor.
fn next_interval_ticks(rng: &mut u64) -> u64 {
    let scale = 0.5 + next_unit(rng); // [0.5, 1.5)
    let ticks = (RTCP_MIN_INTERVAL_TICKS as f64 * scale / RTCP_COMPENSATION) as u64;
    ticks.max(1)
}

/// xorshift64* → uniform f64 in [0, 1). Same RNG family as
/// `facade::rand_icepwd`; no `rand` crate dependency. `rng` is seeded non-zero
/// at channel construction.
fn next_unit(rng: &mut u64) -> f64 {
    let mut x = *rng;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *rng = x;
    let v = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
    (v >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_stays_within_rfc_bounds() {
        let mut rng = 0x1234_5678_9abc_def0u64;
        let lo = (RTCP_MIN_INTERVAL_TICKS as f64 * 0.5 / RTCP_COMPENSATION) as u64;
        let hi = (RTCP_MIN_INTERVAL_TICKS as f64 * 1.5 / RTCP_COMPENSATION) as u64;
        for _ in 0..100_000 {
            let t = next_interval_ticks(&mut rng);
            assert!(t >= lo && t <= hi, "interval {t} out of [{lo},{hi}]");
        }
    }

    #[test]
    fn unit_is_in_unit_interval() {
        let mut rng = 0x0fed_cba9_8765_4321u64;
        for _ in 0..100_000 {
            let u = next_unit(&mut rng);
            assert!((0.0..1.0).contains(&u), "unit {u} out of [0,1)");
        }
    }
}
