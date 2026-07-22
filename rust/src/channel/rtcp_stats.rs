// Receiver-side RTCP accounting — RFC 3550 Appendix A.
//
// `RxStats` tracks one remote source: sequence/loss bookkeeping (A.1 + A.3) and
// interarrival jitter (A.8). It is the source of truth for the reception report
// block we send about the peer, and for the corrected loss figure that finally
// feeds the MOS calculation. `RemoteReport` holds the mirror image — what the
// peer's SR/RR told us about the stream *we* send, plus round-trip time derived
// from LSR/DLSR.
//
// The core `update()` takes arrival time already expressed in RTP timestamp
// units so it is pure and deterministic under test; the `on_packet_at` wrapper
// converts a monotonic `Instant` for the live recv loop.

use std::time::Instant;

use super::rtcp::ReportBlock;

/// RTP sequence numbers are modulo 2^16.
const RTP_SEQ_MOD: u32 = 1 << 16;
/// A forward jump larger than this is treated as a restart, not loss (A.1).
const MAX_DROPOUT: u16 = 3000;
/// Reordering more than this many packets back is a large jump (A.1).
const MAX_MISORDER: u16 = 100;
/// Consecutive in-order packets required before a source is declared valid.
const MIN_SEQUENTIAL: u32 = 2;

/// DLSR / RTT are expressed in units of 1/65536 second (RFC 3550 §6.4.1).
const NTP_FRAC_PER_SEC: f64 = 65_536.0;

/// Default RTP clock for every Tier 1 codec. PCMU/PCMA/iLBC run at 8 kHz, and
/// G.722 uses an 8 kHz RTP clock by convention despite 16 kHz sampling.
pub const DEFAULT_CLOCK_RATE: u32 = 8000;

/// Per-source reception accounting for one remote SSRC.
pub struct RxStats {
    /// Latched from the first RTP packet's SSRC.
    pub remote_ssrc: Option<u32>,
    /// RTP clock rate used to convert arrival instants to timestamp units.
    clock_rate: u32,

    // --- sequence / loss state (RFC 3550 A.1) ---
    max_seq: u16,
    cycles: u32,
    base_seq: u32,
    bad_seq: u32,
    probation: u32,
    received: u32,
    expected_prior: u32,
    received_prior: u32,
    initialized: bool,

    // --- interarrival jitter (RFC 3550 A.8) ---
    transit: u32,
    have_transit: bool,
    jitter: f64,

    // --- for LSR/DLSR in the reports we generate ---
    /// Middle 32 bits of the NTP timestamp from the peer's most recent SR.
    last_sr_ntp_mid: u32,
    /// Local instant that SR arrived, for the DLSR delay.
    last_sr_at: Option<Instant>,

    // --- for converting live arrival instants to RTP units ---
    epoch: Option<Instant>,
}

impl RxStats {
    pub fn new(clock_rate: u32) -> Self {
        Self {
            remote_ssrc: None,
            clock_rate,
            max_seq: 0,
            cycles: 0,
            base_seq: 0,
            bad_seq: 0,
            probation: 0,
            received: 0,
            expected_prior: 0,
            received_prior: 0,
            initialized: false,
            transit: 0,
            have_transit: false,
            jitter: 0.0,
            last_sr_ntp_mid: 0,
            last_sr_at: None,
            epoch: None,
        }
    }

    /// Reset sequence state to a fresh base (RFC 3550 A.1 `init_seq`).
    fn init_seq(&mut self, seq: u16) {
        self.base_seq = seq as u32;
        self.max_seq = seq;
        self.bad_seq = RTP_SEQ_MOD + 1; // so a real seq can never equal it
        self.cycles = 0;
        self.received = 0;
        self.received_prior = 0;
        self.expected_prior = 0;
    }

    /// Live entry point for the recv loop: latch the SSRC, convert `now` to RTP
    /// timestamp units, and run the accounting. Returns true if counted valid.
    pub fn on_packet_at(&mut self, ssrc: u32, seq: u16, rtp_ts: u32, now: Instant) -> bool {
        if self.remote_ssrc.is_none() {
            self.remote_ssrc = Some(ssrc);
        }
        let arrival_rtp = self.arrival_units(now);
        self.update(seq, rtp_ts, arrival_rtp)
    }

    /// Convert a monotonic instant into RTP timestamp units relative to the
    /// first packet's arrival. Differences are what jitter uses, so the epoch
    /// offset cancels; wrapping past 2^32 is harmless (jitter uses wrapping_sub).
    fn arrival_units(&mut self, now: Instant) -> u32 {
        let epoch = *self.epoch.get_or_insert(now);
        let secs = now.saturating_duration_since(epoch).as_secs_f64();
        (secs * self.clock_rate as f64) as u64 as u32
    }

    /// Core sequence + jitter update. `arrival_rtp` is the packet's arrival time
    /// in RTP timestamp units. Returns true if the packet counts as received.
    pub fn update(&mut self, seq: u16, rtp_ts: u32, arrival_rtp: u32) -> bool {
        if !self.initialized {
            self.init_seq(seq);
            self.max_seq = seq.wrapping_sub(1);
            self.probation = MIN_SEQUENTIAL;
            self.initialized = true;
        }
        let valid = self.update_seq(seq);
        if valid {
            self.update_jitter(rtp_ts, arrival_rtp);
        }
        valid
    }

    /// RFC 3550 A.1 `update_seq`. Returns true when the packet is counted.
    fn update_seq(&mut self, seq: u16) -> bool {
        if self.probation > 0 {
            // Source not yet validated: require MIN_SEQUENTIAL in a row.
            if seq == self.max_seq.wrapping_add(1) {
                self.probation -= 1;
                self.max_seq = seq;
                if self.probation == 0 {
                    self.init_seq(seq);
                    self.received += 1;
                    return true;
                }
            } else {
                self.probation = MIN_SEQUENTIAL - 1;
                self.max_seq = seq;
            }
            return false;
        }

        let udelta = seq.wrapping_sub(self.max_seq);
        if udelta < MAX_DROPOUT {
            // In order, with a permissible small gap.
            if seq < self.max_seq {
                self.cycles += RTP_SEQ_MOD;
            }
            self.max_seq = seq;
        } else if (udelta as u32) <= RTP_SEQ_MOD - MAX_MISORDER as u32 {
            // A very large jump: restart only after two in a row (bad_seq).
            if seq as u32 == self.bad_seq {
                self.init_seq(seq);
            } else {
                self.bad_seq = (seq as u32 + 1) & (RTP_SEQ_MOD - 1);
                return false;
            }
        } else {
            // Duplicate or reordered within the window — counted, no advance.
        }
        self.received += 1;
        true
    }

    /// RFC 3550 A.8 interarrival jitter estimate.
    fn update_jitter(&mut self, rtp_ts: u32, arrival_rtp: u32) {
        let transit = arrival_rtp.wrapping_sub(rtp_ts);
        if self.have_transit {
            let d = transit.wrapping_sub(self.transit) as i32;
            let d = d.unsigned_abs() as f64;
            self.jitter += (d - self.jitter) / 16.0;
        } else {
            self.have_transit = true;
        }
        self.transit = transit;
    }

    // --- derived quantities ---

    /// Extended highest sequence number received (cycles + max_seq).
    pub fn ext_max(&self) -> u32 {
        self.cycles + self.max_seq as u32
    }

    /// Packets expected across the session (RFC 3550 A.3).
    pub fn expected(&self) -> u32 {
        self.ext_max().wrapping_sub(self.base_seq).wrapping_add(1)
    }

    /// Exposed for tests / completeness; the live path uses `cumulative_lost`.
    #[allow(dead_code)]
    pub fn received(&self) -> u32 {
        self.received
    }

    /// Cumulative packets lost, clamped to the signed 24-bit report field.
    pub fn cumulative_lost(&self) -> i32 {
        let lost = self.expected() as i64 - self.received as i64;
        lost.clamp(-0x0080_0000, 0x007F_FFFF) as i32
    }

    /// Interarrival jitter in RTP timestamp units (RFC 3550 A.8).
    pub fn jitter(&self) -> u32 {
        self.jitter as u32
    }

    /// Fraction lost since the previous call, as an 8-bit fixed-point value
    /// (RFC 3550 A.3). Mutates the prior-interval counters, so call exactly once
    /// per report block generated.
    pub fn fraction_lost(&mut self) -> u8 {
        let expected = self.expected();
        let expected_interval = expected.wrapping_sub(self.expected_prior);
        self.expected_prior = expected;
        let received_interval = self.received.wrapping_sub(self.received_prior);
        self.received_prior = self.received;
        let lost_interval = expected_interval as i64 - received_interval as i64;
        if expected_interval == 0 || lost_interval <= 0 {
            0
        } else {
            ((lost_interval << 8) / expected_interval as i64) as u8
        }
    }

    /// Fraction lost across the whole session so far, as an 8-bit fixed-point
    /// value (RFC 3550 A.3 scaling). Unlike `fraction_lost`, this reads the
    /// cumulative counters *without mutating* the per-interval state, so it is
    /// safe for the Close summary — calling it can never perturb the interval
    /// figure that `rtcp_tx`'s periodic report blocks depend on.
    pub fn fraction_lost_session(&self) -> u8 {
        let expected = self.expected();
        let lost = self.cumulative_lost();
        if expected == 0 || lost <= 0 {
            0
        } else {
            (((lost as i64) << 8) / expected as i64).min(255) as u8
        }
    }

    // --- LSR / DLSR ---

    /// Record the arrival of a peer SR so our next report can echo LSR/DLSR.
    pub fn note_sender_report(&mut self, ntp_sec: u32, ntp_frac: u32, at: Instant) {
        self.last_sr_ntp_mid = super::rtcp::ntp_middle_32(ntp_sec, ntp_frac);
        self.last_sr_at = Some(at);
    }

    /// LSR: middle 32 bits of the last SR we received (0 if none).
    pub fn lsr(&self) -> u32 {
        self.last_sr_ntp_mid
    }

    /// DLSR: delay since the last SR arrived, in units of 1/65536 s (0 if none).
    pub fn dlsr(&self, now: Instant) -> u32 {
        match self.last_sr_at {
            Some(t) => (now.saturating_duration_since(t).as_secs_f64() * NTP_FRAC_PER_SEC) as u32,
            None => 0,
        }
    }

    /// Build the reception report block about this source, or `None` if no SSRC
    /// has been latched yet. Mutates fraction-lost interval state.
    pub fn report_block(&mut self, now: Instant) -> Option<ReportBlock> {
        let ssrc = self.remote_ssrc?;
        Some(ReportBlock {
            ssrc,
            fraction_lost: self.fraction_lost(),
            cumulative_lost: self.cumulative_lost(),
            ext_highest_seq: self.ext_max(),
            jitter: self.jitter(),
            lsr: self.lsr(),
            dlsr: self.dlsr(now),
        })
    }
}

/// Round-trip time from a report block's LSR/DLSR and the current NTP middle.
/// Returns `None` if the peer has not yet echoed one of our SRs (LSR == 0).
pub fn compute_rtt_ms(now_ntp_mid: u32, lsr: u32, dlsr: u32) -> Option<f64> {
    if lsr == 0 {
        return None;
    }
    let rtt_units = now_ntp_mid.wrapping_sub(lsr).wrapping_sub(dlsr);
    Some(rtt_units as f64 / NTP_FRAC_PER_SEC * 1000.0)
}

/// The peer's view of the stream we send, from its SR/RR report block about us.
#[derive(Debug, Clone, Copy, Default)]
pub struct RemoteReport {
    pub valid: bool,
    pub fraction_lost: u8,
    pub cumulative_lost: i32,
    pub ext_highest_seq: u32,
    pub jitter: u32,
    pub rtt_ms: Option<f64>,
}

impl RemoteReport {
    /// Fold a freshly received report block (about our SSRC) into this summary,
    /// computing RTT from the block's LSR/DLSR against `now_ntp_mid`.
    pub fn update_from(&mut self, rb: &ReportBlock, now_ntp_mid: u32) {
        self.valid = true;
        self.fraction_lost = rb.fraction_lost;
        self.cumulative_lost = rb.cumulative_lost;
        self.ext_highest_seq = rb.ext_highest_seq;
        self.jitter = rb.jitter;
        if let Some(rtt) = compute_rtt_ms(now_ntp_mid, rb.lsr, rb.dlsr) {
            self.rtt_ms = Some(rtt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed a sequence of (seq, ts) pairs with arrival == ts (zero jitter).
    fn feed(rx: &mut RxStats, pairs: &[(u16, u32)]) {
        for &(seq, ts) in pairs {
            rx.update(seq, ts, ts);
        }
    }

    #[test]
    fn in_order_no_loss() {
        let mut rx = RxStats::new(DEFAULT_CLOCK_RATE);
        let pairs: Vec<(u16, u32)> = (100u16..=110).map(|s| (s, s as u32 * 160)).collect();
        feed(&mut rx, &pairs);
        // Probation consumes seq 100; base latches at 101.
        assert_eq!(rx.ext_max(), 110);
        assert_eq!(rx.expected(), 10);
        assert_eq!(rx.received(), 10);
        assert_eq!(rx.cumulative_lost(), 0);
    }

    #[test]
    fn gap_counts_as_loss() {
        let mut rx = RxStats::new(DEFAULT_CLOCK_RATE);
        // 103 and 104 are lost.
        feed(
            &mut rx,
            &[(100, 0), (101, 160), (102, 320), (105, 800), (106, 960)],
        );
        assert_eq!(rx.expected(), 6); // 101..=106
        assert_eq!(rx.received(), 4); // 101,102,105,106
        assert_eq!(rx.cumulative_lost(), 2);
    }

    #[test]
    fn reorder_within_window_is_not_loss() {
        let mut rx = RxStats::new(DEFAULT_CLOCK_RATE);
        feed(
            &mut rx,
            &[(100, 0), (101, 160), (102, 320), (104, 640), (103, 480)],
        );
        assert_eq!(rx.ext_max(), 104);
        assert_eq!(rx.received(), 4); // 101,102,104,103 all counted
        assert_eq!(rx.cumulative_lost(), 0);
    }

    #[test]
    fn sequence_wrap_is_handled() {
        let mut rx = RxStats::new(DEFAULT_CLOCK_RATE);
        feed(
            &mut rx,
            &[(65534, 0), (65535, 160), (0, 320), (1, 480), (2, 640)],
        );
        // base latches at 65535; cycle crossed at seq 0.
        assert_eq!(rx.ext_max(), RTP_SEQ_MOD + 2);
        assert_eq!(rx.expected(), 4);
        assert_eq!(rx.received(), 4);
        assert_eq!(rx.cumulative_lost(), 0);
    }

    #[test]
    fn duplicate_packet_is_counted_no_loss() {
        let mut rx = RxStats::new(DEFAULT_CLOCK_RATE);
        // 102 arrives twice.
        feed(&mut rx, &[(100, 0), (101, 160), (102, 320), (102, 320)]);
        assert_eq!(rx.ext_max(), 102);
        assert_eq!(rx.received(), 3); // 101, 102, dup-102
                                      // expected = 2, received = 3 → negative cumulative loss (RFC A.3).
        assert_eq!(rx.expected(), 2);
        assert_eq!(rx.cumulative_lost(), -1);
    }

    #[test]
    fn zero_jitter_for_evenly_spaced_arrivals() {
        let mut rx = RxStats::new(DEFAULT_CLOCK_RATE);
        // arrival == ts for every packet → transit constant → jitter 0.
        rx.update(100, 0, 0);
        rx.update(101, 160, 160);
        rx.update(102, 320, 320);
        rx.update(103, 480, 480);
        assert_eq!(rx.jitter(), 0);
    }

    #[test]
    fn jitter_grows_with_uneven_arrivals() {
        let mut rx = RxStats::new(DEFAULT_CLOCK_RATE);
        // ts steps by 160; arrival steps by 200 then 240 → transit changes.
        rx.update(100, 0, 0); // probation
        rx.update(101, 160, 200); // first valid: sets transit
        rx.update(102, 320, 400); // transit jumps: jitter rises
        rx.update(103, 480, 640);
        assert!(
            rx.jitter() > 0,
            "jitter should be positive, got {}",
            rx.jitter()
        );
    }

    #[test]
    fn fraction_lost_over_intervals() {
        let mut rx = RxStats::new(DEFAULT_CLOCK_RATE);
        // Interval 1: clean run 101..=104.
        feed(
            &mut rx,
            &[(100, 0), (101, 160), (102, 320), (103, 480), (104, 640)],
        );
        assert_eq!(rx.fraction_lost(), 0);
        // Interval 2: 105,106 then jump to 109,110 → 107,108 lost.
        feed(&mut rx, &[(105, 800), (106, 960), (109, 1440), (110, 1600)]);
        // expected_interval = 6, received_interval = 4, lost = 2 → (2<<8)/6 = 85.
        assert_eq!(rx.fraction_lost(), 85);
    }

    #[test]
    fn dlsr_and_lsr_zero_without_sender_report() {
        let rx = RxStats::new(DEFAULT_CLOCK_RATE);
        let now = Instant::now();
        assert_eq!(rx.lsr(), 0);
        assert_eq!(rx.dlsr(now), 0);
    }

    #[test]
    fn compute_rtt_none_without_lsr() {
        assert!(compute_rtt_ms(0x1234_5678, 0, 0).is_none());
    }

    #[test]
    fn compute_rtt_basic() {
        // now - lsr = 0x20000 units (2 s); dlsr = 0x10000 units (1 s) → 1 s RTT.
        let rtt = compute_rtt_ms(0x0003_0000, 0x0001_0000, 0x0001_0000).unwrap();
        assert!((rtt - 1000.0).abs() < 0.01, "rtt was {rtt}");
    }

    #[test]
    fn report_block_none_before_first_packet() {
        let mut rx = RxStats::new(DEFAULT_CLOCK_RATE);
        assert!(rx.report_block(Instant::now()).is_none());
    }

    #[test]
    fn report_block_carries_latched_ssrc() {
        let mut rx = RxStats::new(DEFAULT_CLOCK_RATE);
        let now = Instant::now();
        rx.on_packet_at(0xCAFE_BABE, 100, 0, now);
        rx.on_packet_at(0xCAFE_BABE, 101, 160, now);
        let rb = rx.report_block(now).expect("have ssrc");
        assert_eq!(rb.ssrc, 0xCAFE_BABE);
    }
}
