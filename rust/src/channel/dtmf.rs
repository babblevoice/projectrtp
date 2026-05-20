// DTMF — RFC 2833 telephone-event encoder/decoder.
//
// Send path: JS enqueues digits via `channel.dtmf("1234#")`. Each digit
// generates one burst of event packets (10 repeats + 3 end packets is the
// C++ default — see channel.cpp dtmf handling).
//
// Receive path: when an inbound RTP packet has payload type == the negotiated
// RFC 2833 PT, decode the event and emit a "telephone-event" callback to JS.
//
// RFC 2833 event packet layout (4-byte payload):
//
//   0                   1                   2                   3
//   0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//  |    event (8)  |E|R|volume (6) |         duration (16)         |
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

use std::collections::VecDeque;

// 11 body + 3 end packets per digit, matching pkcountperdtmfdigit = 14 in
// test/interface/projectrtpdtmf.js.
pub const EVENT_REPEATS: u8 = 11;
pub const END_REPEATS: u8 = 3;
// dB below max — matches the C++ `volume = 13` constant in
// projectrtpchannel.cpp senddtmf(). A prior Rust value of 10 was 3 dB
// quieter than the C++ sender and was flagged during interop review.
pub const DEFAULT_VOLUME: u8 = 13;
pub const EVENT_DURATION_UNIT: u16 = 160; // one G.711 packet worth
// Minimum gap between consecutive DTMF bursts, in tick units (20 ms). The
// C++ sender enforces `snout - lastdtmfsn >= 10` before starting the next
// burst, i.e. 200 ms. Some endpoints coalesce digits sent faster than
// this into a single event or miss the second press entirely.
pub const INTER_DIGIT_GAP_TICKS: u8 = 10;

/// Maps a DTMF character to its RFC 2833 event code. Accepted chars match C++
/// `dtmfchars[]` (channel.cpp:948).
pub fn char_to_event(c: char) -> Option<u8> {
    Some(match c {
        '0'..='9' => c as u8 - b'0',
        '*' => 10,
        '#' => 11,
        'A' | 'a' => 12,
        'B' | 'b' => 13,
        'C' | 'c' => 14,
        'D' | 'd' => 15,
        // RFC 2833 event 16 = Flash / hookflash.
        'F' | 'f' => 16,
        _ => return None,
    })
}

pub fn event_to_char(event: u8) -> Option<char> {
    Some(match event {
        0..=9 => (b'0' + event) as char,
        10 => '*',
        11 => '#',
        12 => 'A',
        13 => 'B',
        14 => 'C',
        15 => 'D',
        16 => 'F',
        _ => return None,
    })
}

/// Encode a single event packet payload.
pub fn encode_event(event: u8, end: bool, volume: u8, duration_ticks: u16) -> [u8; 4] {
    let mut out = [0u8; 4];
    out[0] = event;
    out[1] = (if end { 0x80 } else { 0x00 }) | (volume & 0x3F);
    out[2..4].copy_from_slice(&duration_ticks.to_be_bytes());
    out
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct DecodedEvent {
    pub event: u8,
    pub end: bool,
    pub volume: u8,
    pub duration: u16,
}

pub fn decode_event(payload: &[u8]) -> Option<DecodedEvent> {
    if payload.len() < 4 { return None; }
    let event = payload[0];
    let end = payload[1] & 0x80 != 0;
    let volume = payload[1] & 0x3F;
    let duration = u16::from_be_bytes([payload[2], payload[3]]);
    Some(DecodedEvent { event, end, volume, duration })
}

/// One packet ready to go on the wire. Callers copy these fields onto the
/// outgoing RTP packet verbatim; do NOT substitute `state.out_ts` for
/// `timestamp`, that defeats the burst-latching this type exists to enforce.
#[derive(Debug, Clone, Copy)]
pub struct NextDtmfPacket {
    // Purely informational — the event code is already encoded in
    // payload[0]. Send paths never need it; tests assert on it to
    // verify which digit a burst is emitting.
    #[allow(dead_code)]
    pub event: u8,
    pub payload: [u8; 4],
    /// Set on the first body packet of each burst, matching the C++
    /// sender's `dst->setmarker( 0 == this->dtmfsendcount )`. Some
    /// endpoints (notably Cisco and some Avaya) key event-start detection
    /// off the marker bit and miss digits entirely when it's absent.
    pub marker: bool,
    /// Constant for every packet in one burst — 11 body + 3 end all share
    /// the value captured when the burst started. RFC 2833 requires this;
    /// strict receivers treat packets with distinct TS as independent
    /// events, which on a mix produces ghost digits or none at all.
    pub timestamp: u32,
}

/// Send-side queue. One `DtmfBurst` is enqueued per digit; each tick the
/// caller invokes `next_event(current_ts)`, which returns a ready-to-send
/// packet (with latched burst TS and marker bit) or `None` if nothing
/// should go on the wire this tick — including inter-digit gap waits.
pub struct DtmfSender {
    queue: VecDeque<DtmfBurst>,
    ticks_remaining: u8,
    end_remaining: u8,
    duration_ticks: u16,
    current_event: Option<u8>,
    /// RTP timestamp latched at the start of the current burst. Every
    /// packet in the burst (body + end retransmits) carries this value
    /// regardless of what the channel's running `out_ts` has drifted to.
    burst_ts: Option<u32>,
    /// Ticks elapsed since the previous burst's last end packet. Starts
    /// saturated at INTER_DIGIT_GAP_TICKS so the very first burst fires
    /// immediately. After a burst ends, resets to 0 and counts up on
    /// subsequent calls until it's safe to pop the next queued digit.
    idle_ticks_since_burst: u8,
}

#[derive(Debug, Clone, Copy)]
struct DtmfBurst { event: u8 }

impl Default for DtmfSender {
    fn default() -> Self { Self::new() }
}

impl DtmfSender {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            ticks_remaining: 0,
            end_remaining: 0,
            duration_ticks: 0,
            current_event: None,
            burst_ts: None,
            idle_ticks_since_burst: INTER_DIGIT_GAP_TICKS,
        }
    }

    pub fn enqueue(&mut self, digits: &str) {
        for c in digits.chars() {
            if let Some(event) = char_to_event(c) {
                self.queue.push_back(DtmfBurst { event });
            }
        }
    }

    #[allow(dead_code)]
    pub fn is_idle(&self) -> bool {
        self.current_event.is_none() && self.queue.is_empty()
    }

    /// Advance one tick. `current_ts` is the channel's running RTP
    /// timestamp at the moment of the call; it's used only to latch the
    /// burst start — body and end packets reuse the latched value.
    ///
    /// Returns `Some(packet)` when a DTMF packet should go out this tick,
    /// or `None` if the sender is idle, queue-empty, or in the inter-digit
    /// gap after a previous burst.
    pub fn next_event(&mut self, current_ts: u32) -> Option<NextDtmfPacket> {
        if self.current_event.is_none() {
            // Between-bursts idle: count up to the inter-digit minimum.
            // Saturate so the counter stays at INTER_DIGIT_GAP_TICKS while
            // the queue is empty, which lets the first enqueued digit
            // fire on the very next tick.
            if self.idle_ticks_since_burst < INTER_DIGIT_GAP_TICKS {
                self.idle_ticks_since_burst += 1;
            }
            if self.queue.is_empty() { return None; }
            if self.idle_ticks_since_burst < INTER_DIGIT_GAP_TICKS { return None; }

            let burst = self.queue.pop_front()?;
            self.current_event = Some(burst.event);
            self.ticks_remaining = EVENT_REPEATS;
            self.end_remaining = END_REPEATS;
            self.duration_ticks = EVENT_DURATION_UNIT;
            self.burst_ts = Some(current_ts);
        }

        let event = self.current_event.unwrap();
        let ts = self.burst_ts.expect("burst_ts set alongside current_event");
        // Marker bit fires exactly once per burst: on the first body
        // packet (ticks_remaining still at its just-seeded EVENT_REPEATS).
        let marker = self.ticks_remaining == EVENT_REPEATS;

        if self.ticks_remaining > 0 {
            self.ticks_remaining -= 1;
            let payload = encode_event(event, false, DEFAULT_VOLUME, self.duration_ticks);
            self.duration_ticks = self.duration_ticks.saturating_add(EVENT_DURATION_UNIT);
            return Some(NextDtmfPacket { event, payload, marker, timestamp: ts });
        }

        if self.end_remaining > 0 {
            self.end_remaining -= 1;
            let payload = encode_event(event, true, DEFAULT_VOLUME, self.duration_ticks);
            if self.end_remaining == 0 {
                self.current_event = None;
                self.burst_ts = None;
                self.idle_ticks_since_burst = 0;
            }
            return Some(NextDtmfPacket { event, payload, marker: false, timestamp: ts });
        }

        // Unreachable: the `current_event.is_none()` guard above covers
        // the end-of-burst state, and both the body and end branches
        // handle their remaining counters exhaustively.
        self.current_event = None;
        self.burst_ts = None;
        None
    }
}

/// Receive-side de-duplicator. RFC 2833 sends each event multiple times; we
/// only report each distinct event once.
///
/// Dedup uses two signals:
///   1. End-of-event marker — when the burst terminates, clear last_event
///      so the next body packet (even of the same digit) reports.
///   2. Large RTP-timestamp gap — all packets of one RFC 2833 burst
///      share (roughly) the same ts. A jump of several hundred ticks
///      signals a new press even when every end-of-event packet was
///      dropped by the jitter buffer. The threshold is loose enough to
///      tolerate test fixtures that don't quite follow RFC 2833's
///      "constant ts within a burst" rule.
pub struct DtmfReceiver {
    last_sn: Option<u16>,
    last_event: Option<u8>,
    last_event_ts: Option<u32>,
}

/// Timestamp delta (in RTP ticks @ 8 kHz = 1 ms) beyond which we treat
/// two packets as belonging to different bursts. An in-burst ts is
/// supposed to be constant; one whole RFC 2833 burst lasts ~220 ms, so
/// a 3200-tick (400 ms) gap safely distinguishes bursts without
/// false-positives on test fixtures that nudge ts within a press.
const NEW_BURST_TS_DELTA: u32 = 3200;

impl Default for DtmfReceiver {
    fn default() -> Self { Self::new() }
}

impl DtmfReceiver {
    pub fn new() -> Self {
        Self { last_sn: None, last_event: None, last_event_ts: None }
    }

    /// Feed an RFC 2833 payload with its RTP sequence number and
    /// timestamp. Returns the digit char on the first packet of a new
    /// distinct event; duplicate packets within the same burst (body
    /// repeats + end-of-event) return None. A repeated digit after an
    /// end-marker — or after a large timestamp gap even without the
    /// end-marker — counts as new, so three "1" presses in a row
    /// produce three reports whether or not the EOE packets survive
    /// the jitter buffer.
    pub fn feed(&mut self, sn: u16, ts: u32, payload: &[u8]) -> Option<char> {
        let ev = decode_event(payload)?;
        let advanced = match self.last_sn {
            Some(last) => sn.wrapping_sub(last) < 0x8000 && sn != last,
            None => true,
        };
        if !advanced { return None; }
        self.last_sn = Some(sn);

        if ev.end {
            // Burst terminator — clear last_event so the next body packet
            // of any code (including the same digit re-pressed) starts a
            // new burst and gets reported.
            self.last_event = None;
            self.last_event_ts = None;
            return None;
        }

        // New-burst detection:
        //   - event code changed → different digit pressed
        //   - ts jumped far beyond the normal in-burst spread → same
        //     digit re-pressed, EOE was lost in jitter
        let same_event = self.last_event == Some(ev.event);
        let big_ts_gap = match self.last_event_ts {
            Some(last_ts) => ts.wrapping_sub(last_ts) >= NEW_BURST_TS_DELTA,
            None => false,
        };
        let same_burst = same_event && !big_ts_gap;
        if same_burst { return None; }

        self.last_event = Some(ev.event);
        self.last_event_ts = Some(ts);
        event_to_char(ev.event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_event_roundtrip() {
        for c in "0123456789*#ABCD".chars() {
            let e = char_to_event(c).unwrap();
            assert_eq!(event_to_char(e), Some(c.to_ascii_uppercase()));
        }
    }

    #[test]
    fn encode_decode_roundtrip() {
        let p = encode_event(7, true, 12, 640);
        let d = decode_event(&p).unwrap();
        assert_eq!(d.event, 7);
        assert!(d.end);
        assert_eq!(d.volume, 12);
        assert_eq!(d.duration, 640);
    }

    #[test]
    fn sender_emits_11_body_plus_3_end_per_digit() {
        let mut s = DtmfSender::new();
        s.enqueue("5");
        let mut body = 0;
        let mut ends = 0;
        while let Some(pkt) = s.next_event(0) {
            if pkt.payload[1] & 0x80 != 0 { ends += 1; } else { body += 1; }
        }
        assert_eq!(body, EVENT_REPEATS as usize);
        assert_eq!(ends, END_REPEATS as usize);
        assert!(s.is_idle());
    }

    #[test]
    fn sender_drains_multiple_digits_with_gap() {
        let mut s = DtmfSender::new();
        s.enqueue("12");
        let body_ends = (EVENT_REPEATS + END_REPEATS) as usize;

        // Collect the first burst — fires immediately on tick 0.
        let mut first = Vec::new();
        for _ in 0..body_ends {
            first.push(s.next_event(0).expect("first burst packet"));
        }
        assert!(first.iter().all(|p| p.event == 1));

        // Inter-digit gap: next INTER_DIGIT_GAP_TICKS - 1 calls yield
        // None (the tick that ended the burst reset the counter to 0; it
        // increments by 1 each subsequent call and only unblocks when it
        // hits INTER_DIGIT_GAP_TICKS).
        for _ in 0..(INTER_DIGIT_GAP_TICKS as usize - 1) {
            assert!(s.next_event(0).is_none(), "expected gap silence");
        }

        // Second burst fires once the gap has elapsed.
        let mut second = Vec::new();
        for _ in 0..body_ends {
            second.push(s.next_event(0).expect("second burst packet"));
        }
        assert!(second.iter().all(|p| p.event == 2));
        assert!(s.is_idle());
    }

    #[test]
    fn marker_set_only_on_first_body_packet_of_each_burst() {
        // Regression: C++ sets the marker bit on the first packet of
        // each DTMF burst (setmarker( 0 == this->dtmfsendcount )). Rust
        // previously never set it, which caused some endpoints to miss
        // the event-start edge and drop the digit entirely.
        let mut s = DtmfSender::new();
        s.enqueue("7");
        let body_ends = (EVENT_REPEATS + END_REPEATS) as usize;
        let mut markers = Vec::new();
        for _ in 0..body_ends {
            markers.push(s.next_event(12345).unwrap().marker);
        }
        // First packet: marker=true. Every other packet (body repeats +
        // end retransmits): marker=false.
        assert!(markers[0], "first body packet must have marker=true");
        assert!(markers[1..].iter().all(|&m| !m), "only first packet marks");
    }

    #[test]
    fn timestamp_is_latched_for_entire_burst() {
        // Regression: RFC 2833 requires a constant RTP timestamp across
        // every packet of one burst. The mixer path advances out_ts on
        // each audio send *before* send_dtmf_outbound fires, so if the
        // caller passes a fresh current_ts each tick the sender must
        // still reuse the one captured at burst start.
        let mut s = DtmfSender::new();
        s.enqueue("3");
        let body_ends = (EVENT_REPEATS + END_REPEATS) as usize;
        let mut ts_seen = Vec::new();
        for n in 0..body_ends {
            // Simulate the mixer: caller's out_ts drifts by 160 per tick.
            let caller_ts: u32 = 1_000 + (n as u32) * 160;
            ts_seen.push(s.next_event(caller_ts).unwrap().timestamp);
        }
        // Every packet in the burst must carry the burst-start value.
        assert!(ts_seen.iter().all(|&ts| ts == 1_000),
                "burst TS drifted: {:?}", ts_seen);
    }

    #[test]
    fn first_burst_fires_immediately_no_startup_gap() {
        // The inter-digit counter is seeded at INTER_DIGIT_GAP_TICKS so
        // a freshly-enqueued digit never has to wait for a phantom gap
        // from "the prior burst that never happened".
        let mut s = DtmfSender::new();
        s.enqueue("0");
        assert!(s.next_event(0).is_some(), "first digit must fire immediately");
    }

    #[test]
    fn receiver_reports_each_digit_once() {
        let mut r = DtmfReceiver::new();
        let p = encode_event(3, false, 10, 160);
        // Multiple duplicates of the same burst — only one report.
        assert_eq!(r.feed(100, 1000, &p), Some('3'));
        assert_eq!(r.feed(101, 1000, &p), None);
        assert_eq!(r.feed(102, 1000, &p), None);
        // End marker observed — doesn't re-report same digit.
        let pe = encode_event(3, true, 10, 480);
        assert_eq!(r.feed(103, 1000, &pe), None);
        // New digit — reported.
        let p2 = encode_event(7, false, 10, 160);
        assert_eq!(r.feed(104, 2000, &p2), Some('7'));
    }

    #[test]
    fn receiver_handles_ivr_pcap_full_sequence() {
        // Regression for ivr_dtmf_issue_9_1_9_9_3.pcap: real call where the
        // caller dialed 9, then 1-9-9-3 (IVR didn't trigger), then 3 again,
        // then 1-1-1-1-2 retrying. Sequence numbers, timestamps, event
        // codes, and end-bits are taken verbatim from the pcap.
        //
        // The two close-spaced "1" presses (ts=232640 and ts=235200,
        // delta=2560) press-start to press-start are 320 ms apart — under
        // the 400 ms NEW_BURST_TS_DELTA threshold. End packets are present
        // in the pcap so the second "1" must be reported via end-bit
        // clearing, not the timestamp fallback.
        let mut r = DtmfReceiver::new();
        let bursts: &[(u32, u8, &[(u16, bool)])] = &[
            (36160,  9, &[(226,false),(228,false),(230,false),(232,false),
                          (234,false),(236,false),(238,false),
                          (239,true),(241,true),(242,true)]),
            (113280, 1, &[(718,false),(720,false),(722,false),(724,false),
                          (726,false),(728,false),(730,false),
                          (731,true),(733,true),(734,true)]),
            (116640, 9, &[(749,false),(751,false),(753,false),(755,false),
                          (757,false),(759,false),(761,false),
                          (762,true),(764,true),(765,true)]),
            (120000, 9, &[(780,false),(782,false),(784,false),(786,false),
                          (788,false),(790,false),(792,false),
                          (793,true),(795,true),(796,true)]),
            (123200, 3, &[(810,false),(812,false),(814,false),(816,false),
                          (818,false),(820,false),(822,false),
                          (823,true),(825,true),(826,true)]),
            (179840, 3, &[(1174,false),(1176,false),(1178,false),(1180,false),
                          (1182,false),(1184,false),(1186,false),
                          (1187,true),(1189,true),(1190,true)]),
            (232640, 1, &[(1514,false),(1516,false),(1518,false),(1520,false),
                          (1522,false),(1524,false),(1526,false),
                          (1527,true),(1529,true),(1530,true)]),
            (235200, 1, &[(1540,false),(1542,false),(1544,false),(1546,false),
                          (1548,false),(1550,false),
                          (1551,true),(1553,true),(1554,true)]),
            (261440, 1, &[(1713,false),(1715,false),(1717,false),(1719,false),
                          (1721,false),(1723,false),(1725,false),
                          (1726,true),(1728,true),(1729,true)]),
            (324800, 1, &[(2119,false),(2121,false),(2123,false),(2125,false),
                          (2127,false),(2129,false),(2131,false),
                          (2132,true),(2134,true),(2135,true)]),
            (328480, 2, &[(2152,false),(2154,false),(2156,false),(2158,false),
                          (2160,false),(2162,false),(2164,false),
                          (2165,true),(2167,true),(2168,true)]),
        ];
        let mut got: Vec<char> = Vec::new();
        for (ts, event, packets) in bursts {
            for (sn, end) in *packets {
                let p = encode_event(*event, *end, 10, 160);
                if let Some(c) = r.feed(*sn, *ts, &p) {
                    got.push(c);
                }
            }
        }
        assert_eq!(got, vec!['9','1','9','9','3','3','1','1','1','1','2'],
                   "expected full IVR sequence, got {:?}", got);
    }

    #[test]
    fn receiver_handles_lost_end_of_event_via_timestamp() {
        // Regression for the PCAP-replay test 2 failure: when all
        // end-of-event packets for one burst are dropped by the jitter
        // buffer, the next press of the same digit must still report
        // — detected via the RTP timestamp changing between bursts.
        let mut r = DtmfReceiver::new();
        let p = encode_event(1, false, 10, 160);

        // First press — body packets at ts=5000.
        assert_eq!(r.feed(100, 5000, &p), Some('1'));
        assert_eq!(r.feed(101, 5000, &p), None);
        assert_eq!(r.feed(102, 5000, &p), None);
        // EOE packets are lost (never fed).

        // Second press — body at ts=10000. Must report despite EOE loss.
        assert_eq!(r.feed(110, 10000, &p), Some('1'));

        // Third press — body at ts=15000. Same digit again.
        assert_eq!(r.feed(120, 15000, &p), Some('1'));
    }
}
