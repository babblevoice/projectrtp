// Relay abuse limits — what a relay leg's source may push through it.
//
// The guest video waiting room forwards media from anonymous web guests.
// SDP limits (max-fs, max-fr, b=AS) only bind an honest browser; a modified
// client can send any bitrate, packet rate, frame rate or resolution
// mid-call without renegotiating, and the relay would hand all of it to a
// staff browser. The relay is the one layer that sees every packet, so it
// enforces. Checked in the source leg's recv_loop on each *authenticated*
// inbound RTP packet (after SRTP on a secure leg), before fan-out, so a
// packet over the limits reaches no receiver. RTCP is not metered here.
//
// Per leg (all of the source's SSRCs together):
// * bitrate — a token bucket of RTP bytes (header + payload, as decrypted),
//   `bitrate / 8` bytes/s deep `burst` bytes (default: one second of
//   bitrate — 250 KB at 2 Mbit/s, above a 720p keyframe from an honest
//   encoder, which is the spike the depth exists to admit);
// * packetrate — a token bucket of packets, one second deep.
// A packet either bucket cannot pay for is dropped (`in.ratedropped`) and
// charged to neither. This is checked before the per-stream limits, so a
// packet over the rate is never parsed.
//
// Per source stream (SSRC):
// * framerate — a "frame" is a distinct RTP timestamp. Frames are metered by
//   a token bucket running at 1.5x the limit, one second (of that rate)
//   deep: an honest 30 fps sender averages 30 against a refill of 45, so its
//   bucket sits full and absorbs any bunching up to 45 frames at once — a
//   1.5 s network stall delivered in one burst — without a drop. Only a
//   sustained rate above 45 fps (or a burst of more than 45 frames) is cut.
//   A timestamp newer than any the stream has sent (RTP serial arithmetic,
//   so wrap-safe) is metered as a new frame. A packet of a recently admitted
//   frame (the last `ADMITTED_FRAMES`) is a straggler or a NACK
//   retransmission and is not: metering those is a feedback loop on a lossy
//   link — each resend is charged as a frame, drops the next real frame,
//   whose packets are NACKed and resent in turn — and it cut an honest
//   sender at 12% loss and 150 ms RTT to two thirds of its packets. Any
//   other older timestamp is metered as a frame too, so a sender cannot
//   escape the limit by counting its timestamps backwards. The verdict is
//   taken on a frame's first packet and applies to every packet with that
//   timestamp (the last few rejected frames are remembered, so a straggler
//   or resend of one is dropped too), so frames are dropped whole
//   (`in.framedropped`, counted in packets) rather than truncated.
// * maxfs — frame size in macroblocks, and each side at most
//   sqrt(8 * maxfs) macroblocks (RFC 6184 / 7741 max-fs semantics, so
//   portrait video fits a landscape limit). Read from keyframes (VP8) and
//   SPS (H.264) — see video_dims.rs — for PTs the leg's negotiated codecs
//   label as VP8 or H.264; any other PT is never guessed at. A stream whose
//   latest keyframe / SPS is over the limit, or malformed, has every packet
//   dropped (`in.oversizedropped`) until one within it arrives — the
//   receiver's PLI, which is relayed, asks the source for one. The
//   oversized keyframe / SPS itself is dropped, so no receiver's decoder
//   ever learns the oversized size. A stream whose size is not yet known
//   (joined mid-GOP) is forwarded.
//
// Every limit is 0 = unlimited. State is fixed-size per leg plus
// `MAX_STREAMS_PER_SOURCE` streams (least recently seen evicted), all owned
// by the leg's recv_loop behind one uncontended leaf mutex; each check is
// O(streams) arithmetic with no allocation and no await.

use std::time::Instant;

use super::relay::MAX_STREAMS_PER_SOURCE;
use super::video_dims::{self, VideoFormat};

/// A leg's limits — see the module comment. `0` in any field is unlimited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayLimits {
    /// Bits per second of inbound RTP, all the leg's SSRCs together.
    pub bitrate: u64,
    /// Bitrate bucket depth in bytes; 0 = one second of `bitrate`.
    pub burst: u64,
    /// RTP packets per second, bucket one second deep.
    pub packetrate: u64,
    /// Frames per second per source stream, enforced at `FRAMERATE_TOLERANCE`x.
    pub framerate: u64,
    /// Maximum frame size in 16x16 macroblocks.
    pub maxfs: u64,
}

/// How far above `framerate` a stream may run before frames are dropped —
/// see the module comment for why an honest sender never reaches it.
pub const FRAMERATE_TOLERANCE: f64 = 1.5;

/// Recently rejected frames remembered per stream, so a straggler or resend
/// of one is dropped with it rather than let through as "older".
const REJECTED_FRAMES: usize = 8;

/// Recently admitted frames remembered per stream, so a straggler or resend
/// of one passes unmetered. 64 frames is two seconds at 30 fps — past the
/// point a NACK resend is any use — and an older timestamp that is none of
/// them is metered as a frame of its own.
const ADMITTED_FRAMES: usize = 64;

impl RelayLimits {
    /// What a relay leg gets when the caller names no limits: 720p30 at
    /// 2 Mbit/s. Protection must not depend on the caller remembering.
    pub const DEFAULT: Self = Self {
        bitrate: 2_000_000,
        burst: 0,
        packetrate: 1000,
        framerate: 30,
        maxfs: 3600,
    };

    /// No limits at all.
    #[cfg(test)]
    pub const UNLIMITED: Self = Self {
        bitrate: 0,
        burst: 0,
        packetrate: 0,
        framerate: 0,
        maxfs: 0,
    };

    /// The limits as enforced: `burst` resolved to its default (and 0 when
    /// bitrate is unlimited, as it then means nothing).
    pub fn effective(self) -> Self {
        let burst = match (self.bitrate, self.burst) {
            (0, _) => 0,
            (b, 0) => b / 8,
            (_, burst) => burst,
        };
        Self { burst, ..self }
    }
}

/// A caller's value for one limit field: `Some` to use it, `None` to take
/// the default. Only an explicit 0 means unlimited; anything else must round
/// to a positive integer. A fraction that rounds to 0 (0.4) takes the
/// default rather than switching the protection off, as do negatives, NaN
/// and infinities. A huge value is clamped to `MAX_LIMIT`, so `livestats()`
/// reads back exactly what is enforced rather than a wrapped negative.
pub fn limit_from_number(n: f64) -> Option<u64> {
    if n == 0.0 {
        return Some(0);
    }
    let r = n.round();
    (r.is_finite() && r >= 1.0).then(|| r.min(MAX_LIMIT as f64) as u64)
}

/// The largest limit a caller can set: JavaScript's largest exact integer
/// (2^53 - 1), which is also far past any real rate or size.
pub const MAX_LIMIT: u64 = (1 << 53) - 1;

impl Default for RelayLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A token bucket: `rate` tokens/s up to `depth`, starting full.
#[derive(Debug, Clone)]
pub struct TokenBucket {
    rate: f64,
    depth: f64,
    tokens: f64,
    last: Option<Instant>,
}

impl TokenBucket {
    pub fn new(rate: f64, depth: f64) -> Self {
        Self {
            rate,
            depth,
            tokens: depth,
            last: None,
        }
    }

    fn refill(&mut self, now: Instant) {
        if let Some(last) = self.last {
            let dt = now.saturating_duration_since(last).as_secs_f64();
            self.tokens = (self.tokens + dt * self.rate).min(self.depth);
        }
        // Never move backwards: a caller's clock read that loses a race with
        // another's must not credit the same interval twice.
        if self.last.is_none_or(|l| now > l) {
            self.last = Some(now);
        }
    }

    /// Refill to `now`; is there room for `cost`?
    pub fn can(&mut self, cost: f64, now: Instant) -> bool {
        self.refill(now);
        self.tokens >= cost
    }

    /// Spend `cost` (after a successful `can`).
    pub fn take(&mut self, cost: f64) {
        self.tokens -= cost;
    }

    /// `can` and, if so, `take`.
    pub fn admit(&mut self, cost: f64, now: Instant) -> bool {
        let ok = self.can(cost, now);
        if ok {
            self.take(cost);
        }
        ok
    }
}

/// Frame-rate meter for one stream — see the module comment.
#[derive(Debug, Clone)]
pub struct FrameLimiter {
    bucket: TokenBucket,
    /// The newest timestamp the stream has sent, once it has sent one.
    highest: Option<u32>,
    /// Timestamps of the most recently rejected frames (a ring).
    rejected: [u32; REJECTED_FRAMES],
    rejected_len: usize,
    rejected_next: usize,
    /// Timestamps of the most recently admitted frames (a ring).
    admitted: [u32; ADMITTED_FRAMES],
    admitted_len: usize,
    admitted_next: usize,
}

/// Is RTP timestamp `a` after `b` (RFC 3550 serial arithmetic, so a stream
/// crossing 2^32 keeps counting forwards)?
fn ts_after(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) > 0
}

impl FrameLimiter {
    pub fn new(framerate: u64) -> Self {
        let rate = framerate as f64 * FRAMERATE_TOLERANCE;
        Self {
            bucket: TokenBucket::new(rate, rate),
            highest: None,
            rejected: [0; REJECTED_FRAMES],
            rejected_len: 0,
            rejected_next: 0,
            admitted: [0; ADMITTED_FRAMES],
            admitted_len: 0,
            admitted_next: 0,
        }
    }

    /// Meter one frame and remember the verdict for its other packets.
    fn meter(&mut self, ts: u32, now: Instant) -> bool {
        let ok = self.bucket.admit(1.0, now);
        if ok {
            self.admitted[self.admitted_next] = ts;
            self.admitted_next = (self.admitted_next + 1) % ADMITTED_FRAMES;
            self.admitted_len = (self.admitted_len + 1).min(ADMITTED_FRAMES);
        } else {
            self.rejected[self.rejected_next] = ts;
            self.rejected_next = (self.rejected_next + 1) % REJECTED_FRAMES;
            self.rejected_len = (self.rejected_len + 1).min(REJECTED_FRAMES);
        }
        ok
    }

    /// Is the packet with RTP timestamp `ts` part of an admitted frame?
    pub fn admit(&mut self, ts: u32, now: Instant) -> bool {
        if self.highest.is_none_or(|h| ts_after(ts, h)) {
            // A new frame.
            self.highest = Some(ts);
            return self.meter(ts, now);
        }
        // The current frame or an older one. A straggler or resend follows
        // its frame's verdict where we still know it...
        if self.rejected[..self.rejected_len].contains(&ts) {
            return false;
        }
        if self.admitted[..self.admitted_len].contains(&ts) {
            return true;
        }
        // ...and an older timestamp we never admitted is a frame of its own:
        // otherwise a sender counting backwards is never metered.
        self.meter(ts, now)
    }
}

/// Why a packet was not forwarded — each has its own counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Forward,
    /// Over the leg's bitrate or packet rate (`in.ratedropped`).
    Rate,
    /// A frame over the stream's frame rate (`in.framedropped`).
    Frame,
    /// The stream's latest keyframe / SPS is over max-fs or malformed
    /// (`in.oversizedropped`).
    Oversize,
}

struct StreamGuard {
    ssrc: u32,
    last: Instant,
    frames: Option<FrameLimiter>,
    /// The stream's latest keyframe / SPS was over the limit (or unreadable):
    /// drop until a compliant one arrives.
    blocked: bool,
}

/// A leg's inbound enforcement state — owned by its recv_loop.
pub struct InboundGuard {
    limits: RelayLimits,
    bytes: Option<TokenBucket>,
    packets: Option<TokenBucket>,
    streams: Vec<StreamGuard>,
}

impl InboundGuard {
    pub fn new(limits: RelayLimits) -> Self {
        let l = limits.effective();
        Self {
            limits: l,
            bytes: (l.bitrate > 0)
                .then(|| TokenBucket::new(l.bitrate as f64 / 8.0, l.burst as f64)),
            packets: (l.packetrate > 0)
                .then(|| TokenBucket::new(l.packetrate as f64, l.packetrate as f64)),
            streams: Vec::with_capacity(MAX_STREAMS_PER_SOURCE),
        }
    }

    /// Decide one authenticated inbound RTP packet (the whole datagram as
    /// decrypted). `fmt` is the payload format its PT is negotiated as on
    /// this leg, `None` when unknown (dimensions are then not checked).
    pub fn check(&mut self, pkt: &[u8], fmt: Option<VideoFormat>, now: Instant) -> Verdict {
        if pkt.len() < super::rtp::RTP_FIXED_HEADER_LEN {
            return Verdict::Forward; // the recv loop never hands us one
        }
        // Rate first, without spending: the checks below parse the payload,
        // and a flood must not buy that work with packets it has no budget
        // to forward anyway.
        let size = pkt.len() as f64;
        let bytes_ok = self.bytes.as_mut().is_none_or(|b| b.can(size, now));
        let packets_ok = self.packets.as_mut().is_none_or(|b| b.can(1.0, now));
        if !(bytes_ok && packets_ok) {
            return Verdict::Rate;
        }
        let limits = self.limits;
        let per_stream = limits.framerate > 0 || (limits.maxfs > 0 && fmt.is_some());
        if per_stream {
            let s = self.stream(super::rtp::ssrc(pkt), now);
            // Resolution before frame rate: a stream we are refusing should
            // not spend frame budget (nor rate budget — taken last).
            if let (Some(fmt), true) = (fmt, limits.maxfs > 0) {
                if let Some(dims) = video_dims::rtp_payload(pkt)
                    .and_then(|p| video_dims::packet_dims(fmt, p, limits.maxfs))
                {
                    s.blocked = dims.map_or(true, |d| d.exceeds(limits.maxfs));
                }
                if s.blocked {
                    return Verdict::Oversize;
                }
            }
            if let Some(frames) = s.frames.as_mut() {
                if !frames.admit(super::rtp::timestamp(pkt), now) {
                    return Verdict::Frame;
                }
            }
        }
        if let Some(b) = self.bytes.as_mut() {
            b.take(size);
        }
        if let Some(b) = self.packets.as_mut() {
            b.take(1.0);
        }
        Verdict::Forward
    }

    /// The state for `ssrc`, created (evicting the least recently seen
    /// stream past `MAX_STREAMS_PER_SOURCE`) if new.
    fn stream(&mut self, ssrc: u32, now: Instant) -> &mut StreamGuard {
        let i = match self.streams.iter().position(|s| s.ssrc == ssrc) {
            Some(i) => i,
            None => {
                if self.streams.len() >= MAX_STREAMS_PER_SOURCE {
                    let lru = (0..self.streams.len())
                        .min_by_key(|&i| self.streams[i].last)
                        .expect("bound is above zero");
                    self.streams.swap_remove(lru);
                }
                self.streams.push(StreamGuard {
                    ssrc,
                    last: now,
                    frames: (self.limits.framerate > 0)
                        .then(|| FrameLimiter::new(self.limits.framerate)),
                    blocked: false,
                });
                self.streams.len() - 1
            }
        };
        let s = &mut self.streams[i];
        s.last = now;
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::video_dims::tests::{vp8_payload, Sps};
    use std::time::Duration;

    fn t(us: u64) -> Instant {
        static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *ORIGIN.get_or_init(Instant::now) + Duration::from_micros(us)
    }

    /// An RTP packet: 12-byte header plus `payload`.
    fn pkt(ssrc: u32, ts: u32, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![0x80, 96, 0, 1];
        p.extend_from_slice(&ts.to_be_bytes());
        p.extend_from_slice(&ssrc.to_be_bytes());
        p.extend_from_slice(payload);
        p
    }

    fn only(f: impl FnOnce(&mut RelayLimits)) -> RelayLimits {
        let mut l = RelayLimits::UNLIMITED;
        f(&mut l);
        l
    }

    #[test]
    fn effective_burst_defaults_to_one_second() {
        assert_eq!(RelayLimits::DEFAULT.effective().burst, 250_000);
        assert_eq!(only(|l| l.bitrate = 800_000).effective().burst, 100_000);
        assert_eq!(
            only(|l| {
                l.bitrate = 800_000;
                l.burst = 5000
            })
            .effective()
            .burst,
            5000
        );
        assert_eq!(only(|l| l.burst = 5000).effective().burst, 0);
    }

    #[test]
    fn limit_values_only_an_explicit_zero_is_unlimited() {
        assert_eq!(limit_from_number(0.0), Some(0));
        assert_eq!(limit_from_number(-0.0), Some(0));
        assert_eq!(limit_from_number(30.0), Some(30));
        assert_eq!(limit_from_number(29.6), Some(30));
        assert_eq!(limit_from_number(0.5), Some(1));
        assert_eq!(limit_from_number(1e30), Some(MAX_LIMIT));
        assert_eq!(limit_from_number(MAX_LIMIT as f64), Some(MAX_LIMIT));
        // would truncate (or round) to 0: the default, not "unlimited"
        for n in [0.4, 1e-9, f64::MIN_POSITIVE, 5e-324] {
            assert_eq!(limit_from_number(n), None, "{n}");
        }
        for n in [-1.0, -0.4, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(limit_from_number(n), None, "{n}");
        }
    }

    #[test]
    fn token_bucket_admits_burst_drops_overrate_and_refills() {
        let mut b = TokenBucket::new(1000.0, 5000.0);
        // the full depth at once
        for _ in 0..5 {
            assert!(b.admit(1000.0, t(0)));
        }
        assert!(!b.admit(1000.0, t(0)));
        // 0.5 s later: 500 tokens — not enough, and a failed ask costs nothing
        assert!(!b.admit(1000.0, t(500_000)));
        assert!(b.admit(1000.0, t(1_000_000)));
        // refill caps at depth
        for _ in 0..5 {
            assert!(b.admit(1000.0, t(100_000_000)));
        }
        assert!(!b.admit(1000.0, t(100_000_000)));
        // a clock read from before `last` credits nothing
        assert!(!b.admit(1.0, t(99_000_000)));
    }

    #[test]
    fn keyframe_burst_passes_sustained_overrate_does_not() {
        let mut g = InboundGuard::new(RelayLimits::DEFAULT);
        // A 200 KB keyframe in one go (one timestamp): all through.
        let big = pkt(1, 0, &[0u8; 1188]);
        for _ in 0..166 {
            assert_eq!(g.check(&big, None, t(0)), Verdict::Forward);
        }
        // 4 Mbit/s sustained for 3 s, 30 fps: about half is dropped.
        let (mut ok, mut dropped) = (0, 0);
        for i in 0..1260u64 {
            // 420 packets/s of 1200 bytes = 4.03 Mbit/s; 14 packets a frame
            let now = t(1_000_000 + i * 1_000_000 / 420);
            match g.check(&pkt(1, 3000 * (i / 14) as u32, &[0u8; 1188]), None, now) {
                Verdict::Forward => ok += 1,
                Verdict::Rate => dropped += 1,
                v => panic!("{v:?}"),
            }
        }
        // ~1 s of refill before this started topped the bucket back up
        // (~250 KB = 208 packets), then 2 Mbit/s = 208 packets/s for 3 s.
        assert!((780..=880).contains(&ok), "forwarded {ok}");
        assert_eq!(ok + dropped, 1260);
    }

    #[test]
    fn packet_rate_is_limited_independently() {
        let mut g = InboundGuard::new(only(|l| l.packetrate = 100));
        let small = pkt(1, 0, &[0u8; 10]);
        let fwd = (0..150)
            .filter(|_| g.check(&small, None, t(0)) == Verdict::Forward)
            .count();
        assert_eq!(fwd, 100);
        assert_eq!(g.check(&small, None, t(10_000)), Verdict::Forward);
        assert_eq!(g.check(&small, None, t(10_000)), Verdict::Rate);
    }

    #[test]
    fn a_rate_drop_charges_neither_bucket() {
        // packetrate would admit, bitrate would not: nothing is spent.
        let mut g = InboundGuard::new(only(|l| {
            l.bitrate = 8000;
            l.burst = 1500;
            l.packetrate = 2;
        }));
        let big = pkt(1, 0, &[0u8; 1400]);
        assert_eq!(g.check(&big, None, t(0)), Verdict::Forward);
        assert_eq!(g.check(&big, None, t(0)), Verdict::Rate);
        // the second packet did not consume a packet token
        let tiny = pkt(1, 0, &[]);
        assert_eq!(g.check(&tiny, None, t(0)), Verdict::Forward);
        assert_eq!(g.check(&tiny, None, t(0)), Verdict::Rate);
    }

    #[test]
    fn unlimited_forwards_everything() {
        let mut g = InboundGuard::new(RelayLimits::UNLIMITED);
        let over = pkt(1, 0, &vp8_payload(true, true, 3840, 2160));
        for i in 0..10_000u32 {
            assert_eq!(
                g.check(&pkt(1, i, &[0u8; 1188]), None, t(0)),
                Verdict::Forward
            );
            assert_eq!(
                g.check(&over, Some(VideoFormat::Vp8), t(0)),
                Verdict::Forward
            );
        }
    }

    /// Frames from a stream sent at `fps` for `secs`, each `per` packets,
    /// arriving at `arrival(frame index) -> µs`. Returns (forwarded, dropped).
    fn run_frames(
        g: &mut InboundGuard,
        fps: u64,
        secs: u64,
        per: u64,
        arrival: impl Fn(u64) -> u64,
    ) -> (u64, u64) {
        let (mut ok, mut dropped) = (0, 0);
        for f in 0..fps * secs {
            let ts = (f * 90_000 / fps) as u32;
            for _ in 0..per {
                match g.check(&pkt(7, ts, &[0u8; 100]), None, t(arrival(f))) {
                    Verdict::Forward => ok += 1,
                    Verdict::Frame => dropped += 1,
                    v => panic!("{v:?}"),
                }
            }
        }
        (ok, dropped)
    }

    #[test]
    fn honest_30fps_with_jitter_and_stalls_is_never_dropped() {
        let mut g = InboundGuard::new(only(|l| l.framerate = 30));
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut jitter = Vec::new();
        for _ in 0..30 * 120 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            jitter.push(seed % 80_000); // 0-80 ms late
        }
        let (ok, dropped) = run_frames(&mut g, 30, 120, 3, |f| {
            let nominal = f * 1_000_000 / 30;
            // every 20 s the network stalls for 1.2 s and then delivers
            // everything it held at once
            let in_stall = nominal % 20_000_000 < 1_200_000 && nominal >= 20_000_000;
            if in_stall {
                (nominal / 20_000_000) * 20_000_000 + 1_200_000
            } else {
                nominal + jitter[f as usize]
            }
        });
        assert_eq!(dropped, 0);
        assert_eq!(ok, 30 * 120 * 3);
    }

    #[test]
    fn ninety_fps_is_cut_to_the_tolerated_rate_whole_frames_at_a_time() {
        let mut g = InboundGuard::new(only(|l| l.framerate = 30));
        let (ok, dropped) = run_frames(&mut g, 90, 10, 3, |f| f * 1_000_000 / 90);
        // one second of depth (45) plus 45/s for the rest of 10 s
        let frames_ok = ok / 3;
        assert!((480..=500).contains(&frames_ok), "{frames_ok} frames");
        assert_eq!(ok % 3, 0, "frames are forwarded or dropped whole");
        assert_eq!(dropped % 3, 0);
        assert_eq!(ok + dropped, 90 * 10 * 3);
    }

    #[test]
    fn a_reordered_packet_follows_its_frames_verdict() {
        let mut f = FrameLimiter::new(2); // 3 frames/s, depth 3
        for ts in [1, 2, 3] {
            assert!(f.admit(ts, t(0)));
        }
        assert!(!f.admit(4, t(0)));
        // a straggler of frame 2 still passes; one of frame 4 still does not
        assert!(f.admit(2, t(0)));
        assert!(!f.admit(4, t(0)));
    }

    #[test]
    fn a_rejected_frames_stragglers_stay_dropped_after_newer_frames() {
        let mut f = FrameLimiter::new(2); // 3 frames/s, depth 3
        for ts in [1, 2, 3] {
            assert!(f.admit(ts, t(0)));
        }
        assert!(!f.admit(4, t(0)));
        // a second later: room again
        assert!(f.admit(5, t(1_000_000)));
        // frame 4 is older than the newest now, but it was dropped: so are
        // its late packets and resends
        assert!(!f.admit(4, t(1_000_000)));
        // an admitted older frame's resend passes, and is not metered
        for _ in 0..100 {
            assert!(f.admit(3, t(1_000_000)));
        }
        assert!(f.admit(6, t(1_000_000)));
    }

    #[test]
    fn a_sender_counting_its_timestamps_backwards_is_still_metered() {
        let mut f = FrameLimiter::new(2); // 3 frames/s, depth 3
        assert!(f.admit(100_000, t(0)));
        // every later frame is "older" than the first
        for ts in [97_000, 94_000] {
            assert!(f.admit(ts, t(0)));
        }
        assert!(!f.admit(91_000, t(0)));
        assert!(!f.admit(88_000, t(0)));
        // their stragglers follow their verdicts, unmetered
        assert!(f.admit(97_000, t(0)));
        assert!(!f.admit(91_000, t(0)));
        // and the bucket refills as usual
        assert!(f.admit(85_000, t(1_000_000)));
    }

    #[test]
    fn frame_rate_is_metered_across_timestamp_wraparound() {
        let mut f = FrameLimiter::new(2); // 3 frames/s, depth 3
        let (a, b) = (u32::MAX - 5999, u32::MAX - 2999);
        for ts in [a, b, 1] {
            assert!(f.admit(ts, t(0)));
        }
        // 3001 follows 1: a new frame, and over the rate
        assert!(!f.admit(3001, t(0)));
        // frames from before the wrap are older, not "far in the future"
        assert!(f.admit(b, t(0)));
        assert!(f.admit(a, t(0)));
        assert!(!f.admit(3001, t(0)));
        assert!(f.admit(6001, t(1_000_000)));
    }

    /// An honest 30 fps sender, 6 packets a frame, on a link losing 12% of
    /// packets at 150 ms RTT. Every packet that does not get through — lost
    /// in the network or dropped by us — is NACKed by the receiver and
    /// resent on the same SSRC one RTT later (up to 10 times). Deterministic:
    /// seeded loss, and the clock is the simulated one.
    #[test]
    fn nack_retransmissions_under_loss_are_not_metered_as_frames() {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        const RTT_US: u64 = 150_000;
        let mut g = InboundGuard::new(only(|l| l.framerate = 30));
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut lost = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % 100 < 12
        };
        // (arrival µs, packet index, attempt)
        let mut q = BinaryHeap::new();
        let (fps, per, secs) = (30u64, 6u64, 60u64);
        for f in 0..fps * secs {
            for k in 0..per {
                // packets of a frame paced 1 ms apart
                q.push(Reverse((f * 1_000_000 / fps + k * 1000, f * per + k, 0u32)));
            }
        }
        let (mut delivered, mut framedropped, mut resends) = (0u64, 0u64, 0u64);
        while let Some(Reverse((at, idx, attempt))) = q.pop() {
            let ts = (idx / per * 90_000 / fps) as u32;
            let got = !lost()
                && match g.check(&pkt(7, ts, &[0u8; 100]), None, t(at)) {
                    Verdict::Forward => true,
                    Verdict::Frame => {
                        framedropped += 1;
                        false
                    }
                    v => panic!("{v:?}"),
                };
            if got {
                delivered += 1;
            } else if attempt < 10 {
                resends += 1;
                q.push(Reverse((at + RTT_US, idx, attempt + 1)));
            }
        }
        assert!(resends > 1000, "the loss model is not exercising resends");
        assert_eq!(framedropped, 0, "honest sender frame-dropped under loss");
        assert_eq!(delivered, fps * secs * per);
    }

    #[test]
    fn frame_rate_is_per_stream() {
        let mut g = InboundGuard::new(only(|l| l.framerate = 10));
        for f in 0..15u32 {
            assert_eq!(g.check(&pkt(1, f, &[]), None, t(0)), Verdict::Forward);
            assert_eq!(g.check(&pkt(2, f, &[]), None, t(0)), Verdict::Forward);
        }
        assert_eq!(g.check(&pkt(1, 99, &[]), None, t(0)), Verdict::Frame);
    }

    /// A packet over the leg's rate is refused before its payload is read:
    /// it is dropped either way, so it leaves no mark on its stream.
    #[test]
    fn a_packet_over_the_rate_is_not_parsed() {
        let mut g = InboundGuard::new(RelayLimits {
            packetrate: 1,
            ..RelayLimits::DEFAULT
        });
        let vp8 = Some(VideoFormat::Vp8);
        let inter = pkt(5, 0, &vp8_payload(false, true, 0, 0));
        assert_eq!(g.check(&inter, vp8, t(0)), Verdict::Forward);
        let big = pkt(5, 3000, &vp8_payload(true, true, 1920, 1080));
        assert_eq!(g.check(&big, vp8, t(1)), Verdict::Rate);
        let inter = pkt(5, 6000, &vp8_payload(false, true, 0, 0));
        assert_eq!(g.check(&inter, vp8, t(1_500_000)), Verdict::Forward);
    }

    #[test]
    fn oversized_vp8_stream_is_dropped_until_a_compliant_keyframe() {
        let mut g = InboundGuard::new(RelayLimits::DEFAULT);
        let vp8 = Some(VideoFormat::Vp8);
        let mut now = 0u64;
        let mut send = |g: &mut InboundGuard, ts: u32, payload: &[u8]| {
            now += 1000;
            g.check(&pkt(5, ts, payload), vp8, t(now))
        };
        // mid-GOP start, size unknown: forwarded
        assert_eq!(
            send(&mut g, 0, &vp8_payload(false, true, 0, 0)),
            Verdict::Forward
        );
        // 1080p keyframe: its first packet, its other packets and the
        // interframes after it are all dropped
        assert_eq!(
            send(&mut g, 3000, &vp8_payload(true, true, 1920, 1080)),
            Verdict::Oversize
        );
        assert_eq!(
            send(&mut g, 3000, &vp8_payload(true, false, 0, 0)),
            Verdict::Oversize
        );
        assert_eq!(
            send(&mut g, 6000, &vp8_payload(false, true, 0, 0)),
            Verdict::Oversize
        );
        // a 720p keyframe restores the stream, all its packets through
        assert_eq!(
            send(&mut g, 9000, &vp8_payload(true, true, 1280, 720)),
            Verdict::Forward
        );
        assert_eq!(
            send(&mut g, 9000, &vp8_payload(true, false, 0, 0)),
            Verdict::Forward
        );
        assert_eq!(
            send(&mut g, 12000, &vp8_payload(false, true, 0, 0)),
            Verdict::Forward
        );
        // portrait 720x1280 is within max-fs
        assert_eq!(
            send(&mut g, 15000, &vp8_payload(true, true, 720, 1280)),
            Verdict::Forward
        );
        // a malformed keyframe header fails closed
        let mut bad = vp8_payload(true, true, 640, 480);
        bad[8] = 0;
        assert_eq!(send(&mut g, 18000, &bad), Verdict::Oversize);
        assert_eq!(
            send(&mut g, 21000, &vp8_payload(false, true, 0, 0)),
            Verdict::Oversize
        );
        assert_eq!(
            send(&mut g, 24000, &vp8_payload(true, true, 640, 480)),
            Verdict::Forward
        );
    }

    #[test]
    fn oversized_h264_sps_blocks_the_stream_until_a_compliant_one() {
        let mut g = InboundGuard::new(RelayLimits::DEFAULT);
        let h264 = Some(VideoFormat::H264);
        let idr = [0x65, 0x88, 0x84, 0x00, 0x33];
        let big = Sps::baseline(120, 68).nal();
        let ok = Sps::baseline(80, 45).nal();
        let stap = |sps: &[u8]| {
            let mut p = vec![0x78];
            p.extend_from_slice(&(sps.len() as u16).to_be_bytes());
            p.extend_from_slice(sps);
            p.extend_from_slice(&4u16.to_be_bytes());
            p.extend_from_slice(&[0x68, 0xce, 0x3c, 0x80]);
            p
        };
        assert_eq!(g.check(&pkt(9, 0, &idr), h264, t(1)), Verdict::Forward);
        assert_eq!(
            g.check(&pkt(9, 3000, &stap(&big)), h264, t(2)),
            Verdict::Oversize
        );
        assert_eq!(g.check(&pkt(9, 3000, &idr), h264, t(3)), Verdict::Oversize);
        // an oversized SPS behind a compliant one in the same STAP-A
        let mut both = stap(&ok);
        both.extend_from_slice(&(big.len() as u16).to_be_bytes());
        both.extend_from_slice(&big);
        assert_eq!(g.check(&pkt(12, 0, &idr), h264, t(4)), Verdict::Forward);
        assert_eq!(
            g.check(&pkt(12, 3000, &both), h264, t(4)),
            Verdict::Oversize
        );
        // another SSRC is unaffected
        assert_eq!(g.check(&pkt(10, 3000, &idr), h264, t(4)), Verdict::Forward);
        assert_eq!(g.check(&pkt(9, 6000, &ok), h264, t(5)), Verdict::Forward);
        assert_eq!(g.check(&pkt(9, 6000, &idr), h264, t(6)), Verdict::Forward);
        // the same bytes under a PT we cannot name are never judged
        assert_eq!(
            g.check(&pkt(11, 0, &stap(&big)), None, t(7)),
            Verdict::Forward
        );
        // maxfs 0 turns it off
        let mut g = InboundGuard::new(RelayLimits {
            maxfs: 0,
            ..RelayLimits::DEFAULT
        });
        assert_eq!(g.check(&pkt(9, 0, &big), h264, t(8)), Verdict::Forward);
    }

    #[test]
    fn stream_state_is_bounded() {
        let mut g = InboundGuard::new(RelayLimits::DEFAULT);
        for ssrc in 0..1000u32 {
            g.check(
                &pkt(ssrc, 0, &[0u8; 20]),
                Some(VideoFormat::Vp8),
                t(u64::from(ssrc)),
            );
        }
        assert_eq!(g.streams.len(), MAX_STREAMS_PER_SOURCE);
    }

    #[test]
    fn garbage_packets_never_panic() {
        let mut g = InboundGuard::new(RelayLimits::DEFAULT);
        let mut seed = 0xdead_beef_u64;
        for i in 0..20_000u64 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let len = (seed % 80) as usize;
            let mut p: Vec<u8> = (0..len)
                .map(|k| (seed >> (k % 56)) as u8 ^ k as u8)
                .collect();
            if p.len() > 12 {
                p[0] = 0x80 | (p[0] & 0x3f); // RTP v2, any P/X/CC
            }
            for fmt in [None, Some(VideoFormat::Vp8), Some(VideoFormat::H264)] {
                let _ = g.check(&p, fmt, t(i));
            }
        }
    }
}
