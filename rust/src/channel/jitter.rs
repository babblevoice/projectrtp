// Jitter / reorder buffer — port of projectrtpbuffer.{cpp,h}.
//
// Stores RTP packets indexed by `sn % buffer_count`. The expected read SN
// (`out_sn`) starts `water_level` behind the first inserted SN so the buffer
// can build up ordering before drain begins. Peek returns the next in-order
// packet (or None if missing); pop advances. Overflow dumps and continues.
//
// Buffer count must be a power of two so the u16 SN wrap matches modular
// indexing (same invariant as C++).
//
// Owns RtpPacket values — `Drop` returns the backing BytesMut to the pool via
// RtpPacket's own Drop (once the pool is wired up in state.rs). No raw slot
// pointers, no shared_from_this.

use super::rtp::RtpPacket;

pub const DEFAULT_BUFFER_PACKET_COUNT: usize = 20;
pub const DEFAULT_BUFFER_WATER_LEVEL: usize = 10;

pub struct JitterBuffer {
    slots: Box<[Option<RtpPacket>]>,
    buffer_count: usize,
    water_level: usize,
    out_sn: u16,
    primed: bool,
    /// Most recently peeked packet slot index (for poppeeked parity with C++).
    peeked_slot: Option<usize>,
    pub dropped: u64,
    pub pushed: u64,
    pub popped: u64,
    pub bad_sn: u64,
}

impl JitterBuffer {
    pub fn new(buffer_count: usize, water_level: usize) -> Self {
        assert!(buffer_count > 0, "buffer_count must be > 0");
        let slots: Vec<Option<RtpPacket>> = (0..buffer_count).map(|_| None).collect();
        Self {
            slots: slots.into_boxed_slice(),
            buffer_count,
            water_level,
            out_sn: 0,
            primed: false,
            peeked_slot: None,
            dropped: 0,
            pushed: 0,
            popped: 0,
            bad_sn: 0,
        }
    }

    pub fn size(&self) -> usize { self.buffer_count }
    #[allow(dead_code)]
    pub fn out_sn(&self) -> u16 { self.out_sn }

    /// Insert a packet. Caller has populated sequence_number etc.
    pub fn push(&mut self, pk: RtpPacket) {
        let sn = pk.sequence_number();

        if !self.primed {
            // First insertion — pull out_sn `water_level` packets behind sn so
            // we can build up ordering before any pop. Matches C++ line 124:
            //   this->outsn = sn - (uint16_t) this->waterlevel;
            self.out_sn = sn.wrapping_sub(self.water_level as u16);
            self.primed = true;
        }

        // Out-of-window: sn ahead of out_sn by more than buffer_count.
        // If the buffer is empty (all slots None), re-prime from this SN
        // instead of dropping — handles the "drain then restart at a very
        // different SN" pattern (e.g. call transfer, unmix-then-mix).
        // Matches C++ line 120-128 where the first packet into an empty
        // buffer re-initialises outsn.
        let ahead = sn.wrapping_sub(self.out_sn) as usize;
        if ahead > self.buffer_count {
            let empty = self.slots.iter().all(|s| s.is_none());
            if empty {
                self.out_sn = sn.wrapping_sub(self.water_level as u16);
            } else {
                self.dropped += 1;
                return;
            }
        }

        let idx = (sn as usize) % self.buffer_count;
        if self.slots[idx].is_none() {
            self.slots[idx] = Some(pk);
            self.pushed += 1;
        } else {
            // Duplicate sn or index collision — drop.
            self.dropped += 1;
        }
    }

    /// Peek the next in-order packet without advancing. If the slot is empty
    /// or holds a stale SN, advances out_sn past it and returns None.
    pub fn peek(&mut self) -> Option<&RtpPacket> {
        if !self.primed { return None; }
        let idx = (self.out_sn as usize) % self.buffer_count;
        match &self.slots[idx] {
            None => {
                // Hole — skip past it.
                self.out_sn = self.out_sn.wrapping_add(1);
                None
            }
            Some(pk) if pk.sequence_number() != self.out_sn => {
                // Stale entry from a previous wrap — evict and skip.
                self.slots[idx] = None;
                self.bad_sn += 1;
                None
            }
            Some(_) => {
                self.peeked_slot = Some(idx);
                self.slots[idx].as_ref()
            }
        }
    }

    /// Advance past the last peeked packet without returning it.
    pub fn discard_peeked(&mut self) {
        if let Some(idx) = self.peeked_slot.take() {
            self.slots[idx] = None;
            self.out_sn = self.out_sn.wrapping_add(1);
            self.popped += 1;
        }
    }

    /// Take the next in-order packet.
    pub fn pop(&mut self) -> Option<RtpPacket> {
        // Drive peek() to settle out_sn past any hole.
        self.peek()?;
        let idx = self.peeked_slot.take()?;
        let out = self.slots[idx].take();
        self.out_sn = self.out_sn.wrapping_add(1);
        self.popped += 1;
        out
    }

    pub fn clear(&mut self) {
        for slot in self.slots.iter_mut() { *slot = None; }
        self.peeked_slot = None;
        self.primed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::rtp::RtpPacket;

    fn mkpk(sn: u16) -> RtpPacket {
        let mut p = RtpPacket::new();
        p.init(1234);
        p.set_sequence_number(sn);
        p
    }

    #[test]
    fn in_order_push_pop_drains_exactly() {
        let mut b = JitterBuffer::new(32, 10);
        for sn in 1000..1010 {
            b.push(mkpk(sn));
        }
        let mut seen = Vec::new();
        for _ in 0..40 {
            if let Some(p) = b.pop() {
                seen.push(p.sequence_number());
            }
        }
        assert_eq!(seen, (1000..1010).collect::<Vec<_>>());
    }

    #[test]
    fn out_of_order_reorders() {
        let mut b = JitterBuffer::new(32, 2);
        b.push(mkpk(500));   // primes out_sn = 498
        b.push(mkpk(502));
        b.push(mkpk(501));
        b.push(mkpk(503));
        let mut seen = Vec::new();
        for _ in 0..20 {
            if let Some(p) = b.pop() { seen.push(p.sequence_number()); }
        }
        assert_eq!(seen, vec![500, 501, 502, 503]);
    }

    #[test]
    fn over_buffer_range_drops() {
        let mut b = JitterBuffer::new(32, 2);
        b.push(mkpk(100));
        b.push(mkpk(20_000));   // absurdly ahead → dropped
        assert!(b.dropped >= 1);
    }

    #[test]
    fn missing_sn_creates_hole_but_does_not_stall() {
        let mut b = JitterBuffer::new(32, 2);
        b.push(mkpk(100));
        b.push(mkpk(101));
        // 102 missing
        b.push(mkpk(103));
        let mut seen = Vec::new();
        for _ in 0..10 {
            if let Some(p) = b.pop() { seen.push(p.sequence_number()); }
        }
        assert!(seen.contains(&100));
        assert!(seen.contains(&101));
        assert!(seen.contains(&103));
        assert!(!seen.contains(&102));
    }

    #[test]
    fn sequence_number_wrap_around() {
        let mut b = JitterBuffer::new(32, 2);
        b.push(mkpk(u16::MAX - 1));
        b.push(mkpk(u16::MAX));
        b.push(mkpk(0));   // wrapped
        b.push(mkpk(1));
        let mut seen = Vec::new();
        for _ in 0..20 {
            if let Some(p) = b.pop() { seen.push(p.sequence_number()); }
        }
        assert_eq!(seen, vec![u16::MAX - 1, u16::MAX, 0, 1]);
    }

    #[test]
    fn restart_at_different_sn_after_drain() {
        let mut b = JitterBuffer::new(32, 10);
        // Push 15, drain all.
        for sn in 256..271 { b.push(mkpk(sn)); }
        let mut seen = Vec::new();
        for _ in 0..30 { if let Some(p) = b.pop() { seen.push(p.sequence_number()); } }
        assert_eq!(seen.len(), 15);

        // Restart from a much higher SN — should re-prime, not drop.
        for sn in 512..522 { b.push(mkpk(sn)); }
        let mut seen2 = Vec::new();
        for _ in 0..30 { if let Some(p) = b.pop() { seen2.push(p.sequence_number()); } }
        assert_eq!(seen2, (512..522).collect::<Vec<_>>());
    }
}
