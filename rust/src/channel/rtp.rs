// RTP packet — port of projectrtppacket.{cpp,h}.
//
// Header layout (RFC 3550 §5.1):
//
//   0                   1                   2                   3
//   0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//  |V=2|P|X|  CC   |M|     PT      |       sequence number         |
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//  |                           timestamp                           |
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//  |           synchronization source (SSRC) identifier            |
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//  |            contributing source (CSRC) identifiers             |
//  |                             ....                              |
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//
// The backing buffer comes from the channel's packet pool (`BytesMut` of
// RTP_MAX_LENGTH capacity). Parsing/mutation is done via free functions on
// byte slices so they're pool-agnostic and trivially testable.

use bytes::BytesMut;

// Payload sizes — lifted from globals.h.
#[allow(dead_code)]
pub const G711_PAYLOAD_BYTES: usize = 160;
#[allow(dead_code)]
pub const G722_PAYLOAD_BYTES: usize = 160;
#[allow(dead_code)]
pub const L16_NARROWBAND_BYTES: usize = 320;
#[allow(dead_code)]
pub const L16_WIDEBAND_BYTES: usize = 640;
#[allow(dead_code)]
pub const ILBC20_PAYLOAD_BYTES: usize = 38;

// Payload types.
#[allow(dead_code)]
pub const PCMU_PAYLOAD_TYPE: u8 = 0;
#[allow(dead_code)]
pub const PCMA_PAYLOAD_TYPE: u8 = 8;
#[allow(dead_code)]
pub const G722_PAYLOAD_TYPE: u8 = 9;
#[allow(dead_code)]
pub const L16_8K_PAYLOAD_TYPE: u8 = 11;
#[allow(dead_code)]
pub const L16_16K_PAYLOAD_TYPE: u8 = 12;
#[allow(dead_code)]
pub const ILBC_PAYLOAD_TYPE: u8 = 97;
#[allow(dead_code)]
pub const RFC2833_PAYLOAD_TYPE: u8 = 101;

pub const RTP_MAX_LENGTH: usize = 1500;
pub const RTP_FIXED_HEADER_LEN: usize = 12;

// ---------- header getters (work on any &[u8]) ----------

#[allow(dead_code)]
#[inline]
pub fn version(pk: &[u8]) -> u8 {
    (pk[0] & 0xC0) >> 6
}
#[allow(dead_code)]
#[inline]
pub fn padding(pk: &[u8]) -> bool {
    pk[0] & 0x20 != 0
}
#[allow(dead_code)]
#[inline]
pub fn extension(pk: &[u8]) -> bool {
    pk[0] & 0x10 != 0
}
#[inline]
pub fn csrc_count(pk: &[u8]) -> u8 {
    pk[0] & 0x0F
}
#[allow(dead_code)]
#[inline]
pub fn marker(pk: &[u8]) -> bool {
    pk[1] & 0x80 != 0
}
#[inline]
pub fn payload_type(pk: &[u8]) -> u8 {
    pk[1] & 0x7F
}
#[inline]
pub fn sequence_number(pk: &[u8]) -> u16 {
    u16::from_be_bytes([pk[2], pk[3]])
}
#[inline]
pub fn timestamp(pk: &[u8]) -> u32 {
    u32::from_be_bytes([pk[4], pk[5], pk[6], pk[7]])
}
#[allow(dead_code)]
#[inline]
pub fn ssrc(pk: &[u8]) -> u32 {
    u32::from_be_bytes([pk[8], pk[9], pk[10], pk[11]])
}

#[inline]
pub fn header_len(pk: &[u8]) -> usize {
    RTP_FIXED_HEADER_LEN + (csrc_count(pk) as usize) * 4
}

// ---------- header setters ----------

#[inline]
#[allow(dead_code)]
pub fn set_marker(pk: &mut [u8], v: bool) {
    if v {
        pk[1] |= 0x80;
    } else {
        pk[1] &= 0x7F;
    }
}

#[inline]
pub fn set_payload_type(pk: &mut [u8], pt: u8) {
    pk[1] = (pk[1] & 0x80) | (pt & 0x7F);
}

#[inline]
pub fn set_sequence_number(pk: &mut [u8], sn: u16) {
    pk[2..4].copy_from_slice(&sn.to_be_bytes());
}

#[inline]
pub fn set_timestamp(pk: &mut [u8], ts: u32) {
    pk[4..8].copy_from_slice(&ts.to_be_bytes());
}

#[inline]
pub fn set_ssrc(pk: &mut [u8], ssrc: u32) {
    pk[8..12].copy_from_slice(&ssrc.to_be_bytes());
}

/// Initialise a packet buffer with RTP v2, no flags, given SSRC, zero payload.
/// Caller sets PT / SN / TS / payload afterwards.
pub fn init(pk: &mut [u8], ssrc_val: u32) {
    for b in pk.iter_mut().take(RTP_FIXED_HEADER_LEN) {
        *b = 0;
    }
    pk[0] = 0x80; // v=2, P=0, X=0, CC=0
    set_ssrc(pk, ssrc_val);
}

// ---------- owning wrapper ----------
//
// Wraps a BytesMut whose capacity == RTP_MAX_LENGTH, with a separate `len` for
// the active slice. Using a capacity-full BytesMut means writes never re-alloc
// — matches the C++ fixed-size `pk[RTPMAXLENGTH]` invariant. When sent, we
// `split_to(len)` to freeze a refcounted `Bytes` that the UDP task can hold.

pub struct RtpPacket {
    pub(crate) buf: BytesMut,
    pub(crate) len: usize,
}

impl RtpPacket {
    pub fn new() -> Self {
        let mut buf = BytesMut::zeroed(RTP_MAX_LENGTH);
        buf[0] = 0x80;
        Self {
            buf,
            len: RTP_FIXED_HEADER_LEN,
        }
    }

    /// Wrap a preallocated buffer from a pool. Capacity must be RTP_MAX_LENGTH.
    #[allow(dead_code)]
    pub fn from_pool(mut buf: BytesMut) -> Self {
        debug_assert_eq!(buf.capacity(), RTP_MAX_LENGTH);
        // Ensure full-length view so direct indexing works.
        unsafe {
            buf.set_len(RTP_MAX_LENGTH);
        }
        for b in buf.iter_mut().take(RTP_FIXED_HEADER_LEN) {
            *b = 0;
        }
        buf[0] = 0x80;
        Self {
            buf,
            len: RTP_FIXED_HEADER_LEN,
        }
    }

    pub fn init(&mut self, ssrc_val: u32) {
        init(&mut self.buf[..], ssrc_val);
        self.len = RTP_FIXED_HEADER_LEN;
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf[..self.len]
    }
    #[allow(dead_code)]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.buf[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    #[allow(dead_code)]
    pub fn set_len(&mut self, len: usize) {
        debug_assert!(len <= self.buf.capacity());
        self.len = len;
    }

    pub fn header_len(&self) -> usize {
        header_len(&self.buf)
    }

    pub fn payload(&self) -> &[u8] {
        let start = self.header_len();
        &self.buf[start..self.len]
    }

    #[allow(dead_code)]
    pub fn payload_mut(&mut self) -> &mut [u8] {
        let start = self.header_len();
        &mut self.buf[start..self.len]
    }

    #[allow(dead_code)]
    pub fn payload_len(&self) -> usize {
        self.len.saturating_sub(self.header_len())
    }

    #[allow(dead_code)]
    pub fn set_payload_len(&mut self, payload_len: usize) {
        self.len = self.header_len() + payload_len;
    }

    /// Copy `src` into our payload region, extending `len` to match.
    pub fn set_payload(&mut self, src: &[u8]) {
        let start = self.header_len();
        debug_assert!(start + src.len() <= self.buf.capacity());
        self.buf[start..start + src.len()].copy_from_slice(src);
        self.len = start + src.len();
    }

    // Header convenience accessors —
    // defer to the free functions so there's one source of truth.
    pub fn payload_type(&self) -> u8 {
        payload_type(&self.buf)
    }
    pub fn set_payload_type(&mut self, pt: u8) {
        set_payload_type(&mut self.buf, pt)
    }
    #[allow(dead_code)]
    pub fn marker(&self) -> bool {
        marker(&self.buf)
    }
    #[allow(dead_code)]
    pub fn set_marker(&mut self, v: bool) {
        set_marker(&mut self.buf, v)
    }
    pub fn sequence_number(&self) -> u16 {
        sequence_number(&self.buf)
    }
    pub fn set_sequence_number(&mut self, sn: u16) {
        set_sequence_number(&mut self.buf, sn)
    }
    pub fn timestamp(&self) -> u32 {
        timestamp(&self.buf)
    }
    pub fn set_timestamp(&mut self, ts: u32) {
        set_timestamp(&mut self.buf, ts)
    }
    #[allow(dead_code)]
    pub fn ssrc(&self) -> u32 {
        ssrc(&self.buf)
    }
    #[allow(dead_code)]
    pub fn set_ssrc(&mut self, s: u32) {
        set_ssrc(&mut self.buf, s)
    }
    #[allow(dead_code)]
    pub fn csrc_count(&self) -> u8 {
        csrc_count(&self.buf)
    }
}

impl Default for RtpPacket {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_sets_v2_and_ssrc() {
        let mut p = RtpPacket::new();
        p.init(0xDEADBEEF);
        assert_eq!(p.csrc_count(), 0);
        assert_eq!(version(p.as_slice()), 2);
        assert_eq!(p.ssrc(), 0xDEADBEEF);
        assert_eq!(p.len(), RTP_FIXED_HEADER_LEN);
    }

    #[test]
    fn header_roundtrip() {
        let mut p = RtpPacket::new();
        p.init(123);
        p.set_payload_type(PCMA_PAYLOAD_TYPE);
        p.set_marker(true);
        p.set_sequence_number(0xFFFE);
        p.set_timestamp(0x01020304);
        assert_eq!(p.payload_type(), PCMA_PAYLOAD_TYPE);
        assert!(p.marker());
        assert_eq!(p.sequence_number(), 0xFFFE);
        assert_eq!(p.timestamp(), 0x01020304);
    }

    #[test]
    fn payload_set_and_get() {
        let mut p = RtpPacket::new();
        p.init(1);
        let src = vec![42u8; G711_PAYLOAD_BYTES];
        p.set_payload(&src);
        assert_eq!(p.payload_len(), G711_PAYLOAD_BYTES);
        assert_eq!(p.payload(), &src[..]);
        assert_eq!(p.len(), RTP_FIXED_HEADER_LEN + G711_PAYLOAD_BYTES);
    }

    #[test]
    fn marker_set_does_not_clobber_pt() {
        let mut p = RtpPacket::new();
        p.init(1);
        p.set_payload_type(97);
        p.set_marker(true);
        assert_eq!(p.payload_type(), 97);
        p.set_marker(false);
        assert_eq!(p.payload_type(), 97);
        assert!(!p.marker());
    }

    #[test]
    fn parse_external_bytes() {
        // v=2, PT=8 (PCMA), marker=1, sn=100, ts=1000, ssrc=0x11223344
        let mut pk = [0u8; 20];
        pk[0] = 0x80;
        pk[1] = 0x80 | 8;
        pk[2..4].copy_from_slice(&100u16.to_be_bytes());
        pk[4..8].copy_from_slice(&1000u32.to_be_bytes());
        pk[8..12].copy_from_slice(&0x11223344u32.to_be_bytes());
        assert_eq!(version(&pk), 2);
        assert_eq!(payload_type(&pk), 8);
        assert!(marker(&pk));
        assert_eq!(sequence_number(&pk), 100);
        assert_eq!(timestamp(&pk), 1000);
        assert_eq!(ssrc(&pk), 0x11223344);
    }
}
