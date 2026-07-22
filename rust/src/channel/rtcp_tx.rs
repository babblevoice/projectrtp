// Periodic RTCP transmission — the send counterpart to `rtcp_loop.rs`.
//
// `maybe_send_rtcp` is called once per member per tick from *both* tick loops
// (Local `tick::run` and Mixed `Member` post-mix housekeeping). On the report
// interval it assembles a compound datagram — an SR when we've sent audio,
// otherwise an RR, always with an SDES/CNAME — and sends it on the P+1 control
// socket to the derived RTCP remote.

use std::net::SocketAddr;
use std::time::Instant;

use super::rtcp::{self, ReportBlock, SenderInfo};
use super::state::ChannelState;

/// Ticks between periodic reports: 250 × 20 ms ≈ 5 s. This is a fixed interval;
/// full RFC 3550 §6.2 randomised/bandwidth-scaled report timing is a deliberate
/// Tier 1 simplification.
pub const RTCP_INTERVAL_TICKS: u64 = 250;

/// Emit a periodic SR/RR + SDES on the report interval. No-op off-interval or
/// until a remote address is known.
pub async fn maybe_send_rtcp(state: &mut ChannelState) {
    if !state.tick_count.is_multiple_of(RTCP_INTERVAL_TICKS) {
        return;
    }

    // Tier 1 emits plain RTP/AVP RTCP only. On an SRTP/DTLS channel that would
    // be unencrypted (non-compliant SRTCP) and would leak SSRC/CNAME/counts in
    // the clear, so stay silent until the Tier 2 SRTCP path lands.
    if state.srtp_encrypt.is_some() {
        return;
    }

    // Derive the RTCP remote: the RTP peer's IP with port + 1 (symmetric RTCP
    // without mux). Skip entirely until the RTP peer is known.
    let Some(rtp_remote) = state.get_remote_addr() else {
        return;
    };
    let rtcp_remote = SocketAddr::new(rtp_remote.ip(), rtp_remote.port().wrapping_add(1));

    let now = Instant::now();
    let (ntp_sec, ntp_frac) = rtcp::ntp_now();

    // Reception report about the peer, if we've latched its stream yet.
    let report = state.rx_stats.lock().report_block(now);
    let reports: &[ReportBlock] = match &report {
        Some(rb) => std::slice::from_ref(rb),
        None => &[],
    };

    // SR once we've sent any audio, otherwise RR.
    let compound = if state.out_count > 0 {
        let info = SenderInfo {
            ntp_sec,
            ntp_frac,
            rtp_ts: state.out_ts,
            packet_count: state.out_count as u32,
            octet_count: state.out_octets as u32,
        };
        rtcp::build_compound(state.ssrc, Some(&info), reports, Some(&state.cname), false)
    } else {
        rtcp::build_compound(state.ssrc, None, reports, Some(&state.cname), false)
    };

    let _ = state.rtcp_sock.send_to(&compound, rtcp_remote).await;
}
