// RTCP packet codec — RFC 3550 §6.
//
// Tier 1: plain RTP/AVP RTCP over the separate control port (RTP port + 1),
// unencrypted. This module is pure: byte-slice encode/decode plus small POD
// structs, no async and no channel state, so it is trivially unit-testable
// (mirroring the style of `rtp.rs`).
//
// Common header (RFC 3550 §6.1):
//
//   0                   1                   2                   3
//   0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//  |V=2|P|  RC/SC  |      PT       |             length            |
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//
// `length` is the packet size in 32-bit words minus one. A "compound" RTCP
// datagram is one or more of these sub-packets concatenated; RFC 3550 §6.1
// requires the first to be an SR or RR and (for a full implementation) an SDES
// CNAME to follow.
//

use std::time::{SystemTime, UNIX_EPOCH};

pub const RTCP_VERSION: u8 = 2;

// Packet types (RFC 3550 §12.1).
pub const PT_SR: u8 = 200;
pub const PT_RR: u8 = 201;
pub const PT_SDES: u8 = 202;
pub const PT_BYE: u8 = 203;
/// Defined for completeness; parser skips APP, live path never emits it.
#[allow(dead_code)]
pub const PT_APP: u8 = 204;

// SDES item types (RFC 3550 §6.5). Only CNAME is mandatory.
pub const SDES_CNAME: u8 = 1;

/// Minimum size of any RTCP sub-packet: the 4-byte common header.
const RTCP_HEADER_LEN: usize = 4;
/// Sender-info block that follows the SSRC in an SR (RFC 3550 §6.4.1).
const SENDER_INFO_LEN: usize = 20;
/// Fixed size of one report block (RFC 3550 §6.4.1).
const REPORT_BLOCK_LEN: usize = 24;
/// A compound packet holds at most 31 report blocks per SR/RR (5-bit RC).
const MAX_REPORT_COUNT: usize = 31;

/// Sender-info block carried in a Sender Report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SenderInfo {
    pub ntp_sec: u32,
    pub ntp_frac: u32,
    pub rtp_ts: u32,
    pub packet_count: u32,
    pub octet_count: u32,
}

/// One reception report block (RFC 3550 §6.4.1). `cumulative_lost` is a signed
/// 24-bit quantity on the wire; we keep the decoded value here as `i32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReportBlock {
    pub ssrc: u32,
    pub fraction_lost: u8,
    pub cumulative_lost: i32,
    pub ext_highest_seq: u32,
    pub jitter: u32,
    pub lsr: u32,
    pub dlsr: u32,
}

/// A decoded sub-packet of interest. SDES/APP and unknown types are skipped
/// during parsing rather than surfaced (Tier 1 only consumes SR/RR/BYE).
#[derive(Debug, Clone, PartialEq)]
pub enum RtcpItem {
    SenderReport {
        ssrc: u32,
        info: SenderInfo,
        reports: Vec<ReportBlock>,
    },
    ReceiverReport {
        ssrc: u32,
        reports: Vec<ReportBlock>,
    },
    Bye {
        ssrcs: Vec<u32>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Buffer shorter than a single common header.
    TooShort,
    /// Version field of the first sub-packet was not 2.
    BadVersion,
    /// A sub-packet's declared length ran past the end of the buffer.
    BadLength,
}

// ---------- NTP helpers (RFC 3550 §4) ----------

/// Seconds between the NTP epoch (1900-01-01) and the Unix epoch (1970-01-01).
const NTP_UNIX_OFFSET_SECS: u64 = 2_208_988_800;

/// Convert a `SystemTime` to a 64-bit NTP timestamp (seconds, fraction).
/// Split out from `ntp_now` so tests are deterministic.
pub fn ntp_from_system_time(t: SystemTime) -> (u32, u32) {
    let dur = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs().wrapping_add(NTP_UNIX_OFFSET_SECS);
    // Fraction: nanoseconds scaled into the top 32 bits of a second.
    let frac = ((dur.subsec_nanos() as u64) << 32) / 1_000_000_000;
    (secs as u32, frac as u32)
}

/// Current wall-clock as an NTP timestamp. The crate may read wall-clock freely
/// (the `Date.now()` ban applies only to workflow scripts, not to Rust).
pub fn ntp_now() -> (u32, u32) {
    ntp_from_system_time(SystemTime::now())
}

/// The middle 32 bits of a 64-bit NTP timestamp, as carried in an SR and echoed
/// back in a report block's LSR field (RFC 3550 §6.4.1).
pub fn ntp_middle_32(ntp_sec: u32, ntp_frac: u32) -> u32 {
    ((ntp_sec & 0x0000_FFFF) << 16) | (ntp_frac >> 16)
}

// ---------- encode ----------

#[inline]
fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}
#[inline]
fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// Write the 4-byte common header. `length_words` is the sub-packet size in
/// 32-bit words *minus one* (filled in by the caller once known).
fn push_header(out: &mut Vec<u8>, count: u8, pt: u8, length_words: u16) {
    out.push((RTCP_VERSION << 6) | (count & 0x1F));
    out.push(pt);
    push_u16(out, length_words);
}

fn push_report_block(out: &mut Vec<u8>, r: &ReportBlock) {
    push_u32(out, r.ssrc);
    // fraction lost (8 bits) then cumulative lost (signed 24 bits).
    let cum = (r.cumulative_lost as u32) & 0x00FF_FFFF;
    out.push(r.fraction_lost);
    out.push((cum >> 16) as u8);
    out.push((cum >> 8) as u8);
    out.push(cum as u8);
    push_u32(out, r.ext_highest_seq);
    push_u32(out, r.jitter);
    push_u32(out, r.lsr);
    push_u32(out, r.dlsr);
}

/// Append an SR (`info = Some`) or RR (`info = None`) sub-packet.
fn push_report_packet(
    out: &mut Vec<u8>,
    sender_ssrc: u32,
    info: Option<&SenderInfo>,
    reports: &[ReportBlock],
) {
    let rc = reports.len().min(MAX_REPORT_COUNT);
    let header_at = out.len();
    let (pt, body_len) = match info {
        Some(_) => (PT_SR, 4 + SENDER_INFO_LEN + rc * REPORT_BLOCK_LEN),
        None => (PT_RR, 4 + rc * REPORT_BLOCK_LEN),
    };
    // length = total words - 1 = (header + body) / 4 - 1.
    let length_words = ((RTCP_HEADER_LEN + body_len) / 4 - 1) as u16;
    push_header(out, rc as u8, pt, length_words);
    push_u32(out, sender_ssrc);
    if let Some(si) = info {
        push_u32(out, si.ntp_sec);
        push_u32(out, si.ntp_frac);
        push_u32(out, si.rtp_ts);
        push_u32(out, si.packet_count);
        push_u32(out, si.octet_count);
    }
    for r in reports.iter().take(rc) {
        push_report_block(out, r);
    }
    debug_assert_eq!(out.len() - header_at, RTCP_HEADER_LEN + body_len);
}

/// Append an SDES packet carrying a single CNAME item for `ssrc`.
fn push_sdes_cname(out: &mut Vec<u8>, ssrc: u32, cname: &str) {
    let header_at = out.len();
    push_header(out, 1, PT_SDES, 0); // length patched below
    let body_at = out.len();
    push_u32(out, ssrc);
    // CNAME item: type, length, text (RFC 3550 §6.5).
    let text = cname.as_bytes();
    let text_len = text.len().min(255);
    out.push(SDES_CNAME);
    out.push(text_len as u8);
    out.extend_from_slice(&text[..text_len]);
    // Null item terminates the list; then pad the chunk to a 32-bit boundary.
    out.push(0);
    while !(out.len() - body_at).is_multiple_of(4) {
        out.push(0);
    }
    let length_words = ((out.len() - header_at) / 4 - 1) as u16;
    out[header_at + 2..header_at + 4].copy_from_slice(&length_words.to_be_bytes());
}

/// Append a BYE packet naming a single source.
fn push_bye(out: &mut Vec<u8>, ssrc: u32) {
    push_header(out, 1, PT_BYE, 1); // header + one SSRC = 2 words → length 1
    push_u32(out, ssrc);
}

/// Assemble a compound RTCP datagram: an SR (when `sender_info` is `Some`) or
/// RR, followed by an SDES/CNAME, optionally followed by a BYE. Returns the
/// wire bytes ready to send on the control socket.
pub fn build_compound(
    sender_ssrc: u32,
    sender_info: Option<&SenderInfo>,
    reports: &[ReportBlock],
    cname: Option<&str>,
    bye: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);
    push_report_packet(&mut out, sender_ssrc, sender_info, reports);
    if let Some(name) = cname {
        push_sdes_cname(&mut out, sender_ssrc, name);
    }
    if bye {
        push_bye(&mut out, sender_ssrc);
    }
    out
}

// ---------- decode ----------

#[inline]
fn read_u16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}
#[inline]
fn read_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Sign-extend a 24-bit big-endian value into `i32` (cumulative lost).
fn read_i24(b: &[u8]) -> i32 {
    let raw = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
    if raw & 0x0080_0000 != 0 {
        (raw | 0xFF00_0000) as i32
    } else {
        raw as i32
    }
}

fn parse_report_block(b: &[u8]) -> ReportBlock {
    ReportBlock {
        ssrc: read_u32(&b[0..4]),
        fraction_lost: b[4],
        cumulative_lost: read_i24(&b[5..8]),
        ext_highest_seq: read_u32(&b[8..12]),
        jitter: read_u32(&b[12..16]),
        lsr: read_u32(&b[16..20]),
        dlsr: read_u32(&b[20..24]),
    }
}

/// Parse a compound RTCP datagram into the sub-packets we care about (SR, RR,
/// BYE). SDES, APP, and unknown types are skipped. A malformed header or an
/// out-of-range length aborts parsing with an error.
pub fn parse(buf: &[u8]) -> Result<Vec<RtcpItem>, ParseError> {
    if buf.len() < RTCP_HEADER_LEN {
        return Err(ParseError::TooShort);
    }
    if (buf[0] >> 6) != RTCP_VERSION {
        return Err(ParseError::BadVersion);
    }

    let mut items = Vec::new();
    let mut off = 0usize;
    while off + RTCP_HEADER_LEN <= buf.len() {
        let count = (buf[off] & 0x1F) as usize;
        let pt = buf[off + 1];
        let length_words = read_u16(&buf[off + 2..off + 4]) as usize;
        let packet_len = (length_words + 1) * 4;
        if off + packet_len > buf.len() {
            return Err(ParseError::BadLength);
        }
        let body = &buf[off..off + packet_len];

        match pt {
            PT_SR => {
                if let Some(item) = parse_report(body, count, true) {
                    items.push(item);
                }
            }
            PT_RR => {
                if let Some(item) = parse_report(body, count, false) {
                    items.push(item);
                }
            }
            PT_BYE => {
                let mut ssrcs = Vec::with_capacity(count);
                for i in 0..count {
                    let at = RTCP_HEADER_LEN + i * 4;
                    if at + 4 <= body.len() {
                        ssrcs.push(read_u32(&body[at..at + 4]));
                    }
                }
                items.push(RtcpItem::Bye { ssrcs });
            }
            _ => { /* SDES / APP / unknown — skip */ }
        }

        off += packet_len;
    }
    Ok(items)
}

/// Parse an SR (`is_sr = true`) or RR body into an `RtcpItem`. Returns `None`
/// if the body is too short for its declared fields.
fn parse_report(body: &[u8], count: usize, is_sr: bool) -> Option<RtcpItem> {
    let mut at = RTCP_HEADER_LEN;
    if at + 4 > body.len() {
        return None;
    }
    let ssrc = read_u32(&body[at..at + 4]);
    at += 4;

    let info = if is_sr {
        if at + SENDER_INFO_LEN > body.len() {
            return None;
        }
        let si = SenderInfo {
            ntp_sec: read_u32(&body[at..at + 4]),
            ntp_frac: read_u32(&body[at + 4..at + 8]),
            rtp_ts: read_u32(&body[at + 8..at + 12]),
            packet_count: read_u32(&body[at + 12..at + 16]),
            octet_count: read_u32(&body[at + 16..at + 20]),
        };
        at += SENDER_INFO_LEN;
        Some(si)
    } else {
        None
    };

    let mut reports = Vec::with_capacity(count);
    for _ in 0..count {
        if at + REPORT_BLOCK_LEN > body.len() {
            break;
        }
        reports.push(parse_report_block(&body[at..at + REPORT_BLOCK_LEN]));
        at += REPORT_BLOCK_LEN;
    }

    Some(match info {
        Some(info) => RtcpItem::SenderReport {
            ssrc,
            info,
            reports,
        },
        None => RtcpItem::ReceiverReport { ssrc, reports },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report(ssrc: u32) -> ReportBlock {
        ReportBlock {
            ssrc,
            fraction_lost: 12,
            cumulative_lost: -5,
            ext_highest_seq: 0x0001_2345,
            jitter: 4321,
            lsr: 0xAABB_CCDD,
            dlsr: 0x0000_1000,
        }
    }

    fn sample_sender_info() -> SenderInfo {
        SenderInfo {
            ntp_sec: 0xE0E1_E2E3,
            ntp_frac: 0x1122_3344,
            rtp_ts: 0x0002_0000,
            packet_count: 500,
            octet_count: 80_000,
        }
    }

    #[test]
    fn compound_is_32bit_aligned() {
        let pkt = build_compound(
            0x1234_5678,
            Some(&sample_sender_info()),
            &[sample_report(0xDEAD_BEEF)],
            Some("host@example.com"),
            false,
        );
        assert_eq!(pkt.len() % 4, 0, "compound must be word-aligned");
    }

    #[test]
    fn sr_roundtrip() {
        let info = sample_sender_info();
        let r = sample_report(0xDEAD_BEEF);
        let pkt = build_compound(0x1234_5678, Some(&info), &[r], Some("a@b"), false);

        let items = parse(&pkt).expect("parse");
        assert_eq!(items.len(), 1, "SDES is skipped, only SR surfaces");
        match &items[0] {
            RtcpItem::SenderReport {
                ssrc,
                info: got_info,
                reports,
            } => {
                assert_eq!(*ssrc, 0x1234_5678);
                assert_eq!(*got_info, info);
                assert_eq!(reports.len(), 1);
                assert_eq!(reports[0], r);
            }
            other => panic!("expected SR, got {other:?}"),
        }
    }

    #[test]
    fn rr_roundtrip() {
        let r0 = sample_report(0x1111_1111);
        let r1 = sample_report(0x2222_2222);
        let pkt = build_compound(0xABCD_0000, None, &[r0, r1], Some("cn"), false);

        let items = parse(&pkt).expect("parse");
        match &items[0] {
            RtcpItem::ReceiverReport { ssrc, reports } => {
                assert_eq!(*ssrc, 0xABCD_0000);
                assert_eq!(reports.len(), 2);
                assert_eq!(reports[0], r0);
                assert_eq!(reports[1], r1);
            }
            other => panic!("expected RR, got {other:?}"),
        }
    }

    #[test]
    fn bye_roundtrip() {
        let pkt = build_compound(0x0BAD_F00D, None, &[], None, true);
        let items = parse(&pkt).expect("parse");
        // RR (with zero reports) then BYE.
        assert!(matches!(items[0], RtcpItem::ReceiverReport { .. }));
        match items.last().unwrap() {
            RtcpItem::Bye { ssrcs } => assert_eq!(ssrcs, &vec![0x0BAD_F00D]),
            other => panic!("expected BYE, got {other:?}"),
        }
    }

    #[test]
    fn negative_cumulative_lost_survives() {
        // Duplicates can make cumulative lost negative (RFC 3550 A.3).
        let mut r = sample_report(1);
        r.cumulative_lost = -100;
        let pkt = build_compound(9, None, &[r], None, false);
        let items = parse(&pkt).unwrap();
        if let RtcpItem::ReceiverReport { reports, .. } = &items[0] {
            assert_eq!(reports[0].cumulative_lost, -100);
        } else {
            panic!("expected RR");
        }
    }

    #[test]
    fn parse_rejects_bad_version() {
        let mut pkt = build_compound(1, None, &[], None, false);
        pkt[0] = 0x40; // version 1
        assert_eq!(parse(&pkt), Err(ParseError::BadVersion));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert_eq!(parse(&[0x80, 201]), Err(ParseError::TooShort));
    }

    #[test]
    fn parse_rejects_length_overrun() {
        let mut pkt = build_compound(1, None, &[sample_report(1)], None, false);
        // Claim a huge length in the first sub-packet header.
        pkt[2] = 0xFF;
        pkt[3] = 0xFF;
        assert_eq!(parse(&pkt), Err(ParseError::BadLength));
    }

    #[test]
    fn parse_captured_sr_bytes() {
        // A hand-authored SR: sender SSRC 0x11223344, one report block for
        // source 0x55667788. Verifies we decode a wire-format packet that we
        // did not encode ourselves.
        #[rustfmt::skip]
        let pkt: [u8; 52] = [
            0x81, 200, 0x00, 0x0C,             // V=2 RC=1 PT=SR len=12 (13 words)
            0x11, 0x22, 0x33, 0x44,             // sender SSRC
            0x83, 0xAA, 0x7E, 0x80,             // NTP sec
            0x00, 0x00, 0x00, 0x00,             // NTP frac
            0x00, 0x00, 0x27, 0x10,             // RTP ts = 10000
            0x00, 0x00, 0x00, 0x64,             // packet count = 100
            0x00, 0x00, 0x3E, 0x80,             // octet count = 16000
            0x55, 0x66, 0x77, 0x88,             // report: SSRC
            0x00, 0x00, 0x00, 0x00,             // fraction 0, cumulative 0
            0x00, 0x00, 0x01, 0x00,             // ext highest seq = 256
            0x00, 0x00, 0x00, 0x0A,             // jitter = 10
            0x00, 0x00, 0x00, 0x00,             // LSR
            0x00, 0x00, 0x00, 0x00,             // DLSR
        ];
        let items = parse(&pkt).expect("parse captured SR");
        match &items[0] {
            RtcpItem::SenderReport {
                ssrc,
                info,
                reports,
            } => {
                assert_eq!(*ssrc, 0x1122_3344);
                assert_eq!(info.rtp_ts, 10_000);
                assert_eq!(info.packet_count, 100);
                assert_eq!(info.octet_count, 16_000);
                assert_eq!(reports.len(), 1);
                assert_eq!(reports[0].ssrc, 0x5566_7788);
                assert_eq!(reports[0].ext_highest_seq, 256);
                assert_eq!(reports[0].jitter, 10);
            }
            other => panic!("expected SR, got {other:?}"),
        }
    }

    #[test]
    fn ntp_epoch_offset_is_applied() {
        // At the Unix epoch, NTP seconds equal the 1900→1970 offset.
        let (sec, frac) = ntp_from_system_time(UNIX_EPOCH);
        assert_eq!(sec, NTP_UNIX_OFFSET_SECS as u32);
        assert_eq!(frac, 0);
    }

    #[test]
    fn ntp_middle_32_packs_correctly() {
        let mid = ntp_middle_32(0xAABB_CCDD, 0x1122_3344);
        // low 16 of sec (CCDD) then high 16 of frac (1122).
        assert_eq!(mid, 0xCCDD_1122);
    }
}
