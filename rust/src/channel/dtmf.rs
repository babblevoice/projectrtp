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
pub const DEFAULT_VOLUME: u8 = 10; // dB below max, matches C++ `volume` const
pub const EVENT_DURATION_UNIT: u16 = 160; // one G.711 packet worth

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

/// Send-side queue. One `DtmfBurst` is enqueued per digit; each tick that
/// should emit an event calls `next_event` which returns Some((event, end))
/// for EVENT_REPEATS body packets + END_REPEATS end packets per digit.
pub struct DtmfSender {
    queue: VecDeque<DtmfBurst>,
    ticks_remaining: u8,
    end_remaining: u8,
    duration_ticks: u16,
    current_event: Option<u8>,
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

    /// Advance one tick. Returns the event payload to send this tick, or None
    /// if nothing to do.
    pub fn next_event(&mut self) -> Option<(u8, [u8; 4])> {
        if self.current_event.is_none() {
            let burst = self.queue.pop_front()?;
            self.current_event = Some(burst.event);
            self.ticks_remaining = EVENT_REPEATS;
            self.end_remaining = END_REPEATS;
            self.duration_ticks = EVENT_DURATION_UNIT;
        }

        let event = self.current_event.unwrap();

        if self.ticks_remaining > 0 {
            self.ticks_remaining -= 1;
            let payload = encode_event(event, false, DEFAULT_VOLUME, self.duration_ticks);
            self.duration_ticks = self.duration_ticks.saturating_add(EVENT_DURATION_UNIT);
            return Some((event, payload));
        }

        if self.end_remaining > 0 {
            self.end_remaining -= 1;
            let payload = encode_event(event, true, DEFAULT_VOLUME, self.duration_ticks);
            if self.end_remaining == 0 {
                self.current_event = None;
            }
            return Some((event, payload));
        }

        self.current_event = None;
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
        while let Some((_e, payload)) = s.next_event() {
            if payload[1] & 0x80 != 0 { ends += 1; } else { body += 1; }
        }
        assert_eq!(body, EVENT_REPEATS as usize);
        assert_eq!(ends, END_REPEATS as usize);
        assert!(s.is_idle());
    }

    #[test]
    fn sender_drains_multiple_digits() {
        let mut s = DtmfSender::new();
        s.enqueue("12");
        let mut events = Vec::new();
        while let Some((e, _)) = s.next_event() { events.push(e); }
        let body_ends = (EVENT_REPEATS + END_REPEATS) as usize;
        assert_eq!(events.len(), body_ends * 2);
        assert!(events[..body_ends].iter().all(|&e| e == 1));
        assert!(events[body_ends..].iter().all(|&e| e == 2));
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
