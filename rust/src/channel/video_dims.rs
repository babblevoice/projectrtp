// Frame dimensions from the first packet of a video keyframe — the
// resolution half of the relay's abuse limits (see relay_limits.rs).
//
// The relay never decodes, but a guest can send any resolution it likes
// without renegotiating: the SDP only binds an honest browser. Both codecs
// the relay carries announce the coded size in-band, at a point a single RTP
// packet reaches:
//
// * VP8 (RFC 7741 payload, RFC 6386 bitstream): the first packet of a
//   keyframe (descriptor S=1, PID=0, frame tag P=0) carries the start code
//   9d 01 2a and the 14-bit width and height.
// * H.264 (RFC 6184, packetization-mode 0/1): the SPS (NAL type 7), sent as
//   a single NAL unit, inside a STAP-A, or at the start of an FU-A. The
//   macroblock dimensions sit after a handful of exp-Golomb fields — and,
//   for the high profiles, after the chroma format and optional scaling
//   lists, which a hostile sender may use on any stream, so they are parsed
//   rather than assumed absent.
//
// Everything here reads attacker-controlled bytes: every read is
// bounds-checked and a malformed or truncated header is an `Err`, never a
// panic. The caller fails closed on `Err` (a real encoder never produces
// one, and a decoder that disagrees with our reading of a malformed header
// is exactly the gap an attacker would use).

/// Coded frame size, in 16x16 macroblocks — what max-fs (RFC 6184 / 7741)
/// is expressed in and what a decoder allocates — plus the displayed size in
/// pixels (after H.264 cropping) for reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dims {
    pub width_mbs: u64,
    pub height_mbs: u64,
    pub width_px: u64,
    pub height_px: u64,
}

impl Dims {
    /// Does this frame break `maxfs` (macroblocks per frame), or have a side
    /// longer than `sqrt(8 * maxfs)` macroblocks (the max-fs aspect rule of
    /// RFC 6184 / 7741, which admits portrait as well as landscape)? A
    /// `maxfs` of 0 is unlimited.
    pub fn exceeds(&self, maxfs: u64) -> bool {
        if maxfs == 0 {
            return false;
        }
        let side = maxfs.saturating_mul(8).isqrt();
        self.width_mbs.saturating_mul(self.height_mbs) > maxfs
            || self.width_mbs > side
            || self.height_mbs > side
    }
}

/// The payload formats whose dimensions we can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoFormat {
    Vp8,
    H264,
}

/// Why a keyframe header could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimsError {
    /// Ran out of bytes before the dimensions.
    Truncated,
    /// A field outside what the bitstream spec allows.
    Invalid,
}

/// The dimensions a packet of `fmt` announces: `None` when it carries no
/// keyframe header / SPS (most packets), `Some(Err)` when it does but the
/// header is malformed. `maxfs` only matters for a packet announcing several
/// sizes (a STAP-A with more than one SPS): the one reported is the first
/// that exceeds it, else the last. `payload` is the RTP payload (after CSRCs, header
/// extension and padding — see `rtp_payload`).
pub fn packet_dims(
    fmt: VideoFormat,
    payload: &[u8],
    maxfs: u64,
) -> Option<Result<Dims, DimsError>> {
    match fmt {
        VideoFormat::Vp8 => vp8_keyframe_dims(payload),
        VideoFormat::H264 => h264_sps_dims(payload, maxfs),
    }
}

/// The RTP payload of `pkt`: past the fixed header, CSRCs and any header
/// extension (RFC 3550 §5.3.1 — WebRTC always sends one), and short of any
/// padding. `None` when the header lengths do not fit the packet.
pub fn rtp_payload(pkt: &[u8]) -> Option<&[u8]> {
    if pkt.len() < 12 {
        return None;
    }
    let mut start = 12 + (pkt[0] & 0x0F) as usize * 4;
    if pkt[0] & 0x10 != 0 {
        let ext = pkt.get(start..start + 4)?;
        start += 4 + u16::from_be_bytes([ext[2], ext[3]]) as usize * 4;
    }
    let mut end = pkt.len();
    if pkt[0] & 0x20 != 0 {
        end = end.checked_sub(*pkt.last()? as usize)?;
    }
    pkt.get(start..end)
}

// ---------------------------------------------------------------- VP8 ----

/// RFC 7741 §4.2 payload descriptor, then — on the first packet of a
/// keyframe only — the RFC 6386 §9.1 frame tag, start code and dimensions.
fn vp8_keyframe_dims(p: &[u8]) -> Option<Result<Dims, DimsError>> {
    let b0 = *p.first()?;
    let start = b0 & 0x10 != 0;
    let pid = b0 & 0x07;
    let mut i = 1usize;
    if b0 & 0x80 != 0 {
        // X: extension byte I L T K, each adding fields.
        let x = *p.get(i)?;
        i += 1;
        if x & 0x80 != 0 {
            // I: PictureID, 7 or (M set) 15 bits.
            let m = *p.get(i)?;
            i += if m & 0x80 != 0 { 2 } else { 1 };
        }
        if x & 0x40 != 0 {
            i += 1; // L: TL0PICIDX
        }
        if x & 0x30 != 0 {
            i += 1; // T or K: TID / Y / KEYIDX
        }
    }
    if !start || pid != 0 {
        return None; // not the start of a frame's first partition
    }
    let frame = p.get(i..)?;
    let tag = *frame.first()?;
    if tag & 0x01 != 0 {
        return None; // P set: an interframe — no dimensions
    }
    // A keyframe: 3-byte frame tag, start code, then 2+2 bytes of
    // (scale << 14 | size), little-endian. Past this point anything that
    // does not read is malformed, not "no information".
    let Some(hdr) = frame.get(3..10) else {
        return Some(Err(DimsError::Truncated));
    };
    if hdr[..3] != [0x9d, 0x01, 0x2a] {
        return Some(Err(DimsError::Invalid));
    }
    let w = u64::from(u16::from_le_bytes([hdr[3], hdr[4]]) & 0x3fff);
    let h = u64::from(u16::from_le_bytes([hdr[5], hdr[6]]) & 0x3fff);
    if w == 0 || h == 0 {
        return Some(Err(DimsError::Invalid));
    }
    // The scale bits ask the *display* to upscale; the decoder works at the
    // coded size, which is what max-fs limits.
    Some(Ok(Dims {
        width_mbs: w.div_ceil(16),
        height_mbs: h.div_ceil(16),
        width_px: w,
        height_px: h,
    }))
}

// --------------------------------------------------------------- H.264 ---

const NAL_SPS: u8 = 7;
const NAL_STAP_A: u8 = 24;
const NAL_FU_A: u8 = 28;

/// The SPS in an H.264 RTP payload, if it carries one. A STAP-A may carry
/// more than one: any malformed or oversized (for `maxfs`) SPS in it decides
/// the packet (the whole packet is what gets forwarded or dropped),
/// otherwise the last.
///
/// STAP-B, MTAP and FU-B (interleaved mode) are not parsed: we negotiate
/// packetization-mode 1 at most, and receivers (libwebrtc) drop those types.
fn h264_sps_dims(p: &[u8], maxfs: u64) -> Option<Result<Dims, DimsError>> {
    let nal = *p.first()?;
    match nal & 0x1f {
        NAL_SPS => Some(sps_dims(&p[1..])),
        NAL_STAP_A => {
            let mut found: Option<Result<Dims, DimsError>> = None;
            let mut i = 1usize;
            while i + 2 <= p.len() {
                let n = u16::from_be_bytes([p[i], p[i + 1]]) as usize;
                i += 2;
                let Some(unit) = p.get(i..i + n) else {
                    // A size running past the packet: if it is an SPS, what
                    // we can see of it must still parse.
                    let rest = &p[i..];
                    if rest.first().is_some_and(|h| h & 0x1f == NAL_SPS) {
                        return Some(Err(DimsError::Truncated));
                    }
                    break;
                };
                i += n;
                if unit.first().is_some_and(|h| h & 0x1f == NAL_SPS) {
                    let r = sps_dims(&unit[1..]);
                    // An oversized (or malformed) SPS must not hide behind
                    // a compliant one in the same aggregate.
                    if r.as_ref().map_or(true, |d| d.exceeds(maxfs)) {
                        return Some(r);
                    }
                    found = Some(r);
                }
            }
            found
        }
        NAL_FU_A => {
            // FU indicator, FU header (S E R type), then the fragment. Only
            // the first fragment (S) begins the NAL; an SPS is small enough
            // that a real sender never splits it, and one that does must
            // still put the dimensions in the first fragment for us to pass
            // it (a truncated read is an `Err`).
            let fu = *p.get(1)?;
            if fu & 0x80 == 0 || fu & 0x1f != NAL_SPS {
                return None;
            }
            Some(sps_dims(&p[2..]))
        }
        _ => None,
    }
}

/// Undo H.264 emulation prevention (§7.4.1: 00 00 03 → 00 00) into `out`,
/// returning the RBSP length. `out` is at least as long as `ebsp`.
fn unescape(ebsp: &[u8], out: &mut [u8]) -> usize {
    let mut n = 0usize;
    let mut zeros = 0u8;
    for &b in ebsp {
        if zeros >= 2 && b == 0x03 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros.saturating_add(1) } else { 0 };
        out[n] = b;
        n += 1;
    }
    n
}

/// A bounds-checked MSB-first bit reader over an RBSP.
struct Bits<'a> {
    d: &'a [u8],
    pos: usize, // in bits
}

impl<'a> Bits<'a> {
    fn new(d: &'a [u8]) -> Self {
        Self { d, pos: 0 }
    }

    fn bit(&mut self) -> Result<u64, DimsError> {
        let byte = *self.d.get(self.pos / 8).ok_or(DimsError::Truncated)?;
        let v = (byte >> (7 - (self.pos % 8))) & 1;
        self.pos += 1;
        Ok(u64::from(v))
    }

    fn bits(&mut self, n: u32) -> Result<u64, DimsError> {
        let mut v = 0u64;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Ok(v)
    }

    /// ue(v), §9.1. More than 31 leading zeros is not a valid 32-bit code.
    fn ue(&mut self) -> Result<u64, DimsError> {
        let mut zeros = 0u32;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return Err(DimsError::Invalid);
            }
        }
        Ok((1u64 << zeros) - 1 + self.bits(zeros)?)
    }

    /// se(v), §9.1.1.
    fn se(&mut self) -> Result<i64, DimsError> {
        let k = self.ue()? as i64;
        Ok(if k & 1 == 1 { (k + 1) / 2 } else { -(k / 2) })
    }

    /// ue(v) that must not exceed `max`.
    fn ue_max(&mut self, max: u64) -> Result<u64, DimsError> {
        let v = self.ue()?;
        if v > max {
            return Err(DimsError::Invalid);
        }
        Ok(v)
    }
}

/// Profiles whose SPS carries chroma format, bit depths and scaling lists
/// (§7.3.2.1.1).
fn high_profile(profile_idc: u64) -> bool {
    matches!(
        profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    )
}

/// Profiles whose SPS layout we know: Baseline, Main, Extended, and the
/// high family above. Any other `profile_idc` is malformed, not "assume no
/// chroma fields": a decoder may read that profile's SPS with extra fields
/// (FFmpeg treats 144, the retired High 4:4:4, as a high profile), and a
/// sender that relabels a 1080p High SPS as such a profile would otherwise
/// be measured from misaligned bits while the decoder sees 1080p. Unknown
/// is fail-closed.
fn known_profile(profile_idc: u64) -> bool {
    matches!(profile_idc, 66 | 77 | 88) || high_profile(profile_idc)
}

/// §7.3.2.1.1.1 — read (and discard) one scaling list.
fn skip_scaling_list(r: &mut Bits, size: usize) -> Result<(), DimsError> {
    let (mut last, mut next) = (8i64, 8i64);
    for _ in 0..size {
        if next != 0 {
            let delta = r.se()?;
            if !(-128..=127).contains(&delta) {
                return Err(DimsError::Invalid);
            }
            next = (last + delta).rem_euclid(256);
        }
        if next != 0 {
            last = next;
        }
    }
    Ok(())
}

/// Parse an SPS body (after the one-byte NAL header, emulation prevention
/// still in place) as far as the frame size and cropping (§7.3.2.1.1).
pub fn sps_dims(ebsp: &[u8]) -> Result<Dims, DimsError> {
    let mut buf = [0u8; super::rtp::RTP_MAX_LENGTH];
    let ebsp = &ebsp[..ebsp.len().min(buf.len())];
    let n = unescape(ebsp, &mut buf);
    let mut r = Bits::new(&buf[..n]);

    let profile_idc = r.bits(8)?;
    if !known_profile(profile_idc) {
        return Err(DimsError::Invalid);
    }
    r.bits(8)?; // constraint_set flags + reserved
    r.bits(8)?; // level_idc
    r.ue_max(31)?; // seq_parameter_set_id
    let mut chroma_format_idc = 1u64;
    let mut separate_colour_plane = false;
    if high_profile(profile_idc) {
        chroma_format_idc = r.ue_max(3)?;
        if chroma_format_idc == 3 {
            separate_colour_plane = r.bit()? == 1;
        }
        r.ue_max(6)?; // bit_depth_luma_minus8
        r.ue_max(6)?; // bit_depth_chroma_minus8
        r.bit()?; // qpprime_y_zero_transform_bypass_flag
        if r.bit()? == 1 {
            // seq_scaling_matrix_present_flag
            let lists = if chroma_format_idc != 3 { 8 } else { 12 };
            for i in 0..lists {
                if r.bit()? == 1 {
                    skip_scaling_list(&mut r, if i < 6 { 16 } else { 64 })?;
                }
            }
        }
    }
    r.ue_max(12)?; // log2_max_frame_num_minus4
    match r.ue_max(2)? {
        0 => {
            r.ue_max(12)?; // log2_max_pic_order_cnt_lsb_minus4
        }
        1 => {
            r.bit()?; // delta_pic_order_always_zero_flag
            r.se()?; // offset_for_non_ref_pic
            r.se()?; // offset_for_top_to_bottom_field
            let cycle = r.ue_max(255)?;
            for _ in 0..cycle {
                r.se()?; // offset_for_ref_frame
            }
        }
        _ => {}
    }
    r.ue()?; // max_num_ref_frames
    r.bit()?; // gaps_in_frame_num_value_allowed_flag
    let width_mbs = r.ue()? + 1;
    let map_units = r.ue()? + 1;
    let frame_mbs_only = r.bit()?;
    if frame_mbs_only == 0 {
        r.bit()?; // mb_adaptive_frame_field_flag
    }
    r.bit()?; // direct_8x8_inference_flag
    let height_mbs = map_units * (2 - frame_mbs_only);

    // Cropping (§7.4.2.1.1): offsets count chroma samples horizontally and
    // (chroma rows x field factor) vertically.
    let chroma_array_type = if separate_colour_plane {
        0
    } else {
        chroma_format_idc
    };
    let (sub_w, sub_h) = match chroma_array_type {
        1 => (2, 2), // 4:2:0
        2 => (2, 1), // 4:2:2
        _ => (1, 1), // monochrome, 4:4:4
    };
    let crop_x = sub_w;
    let crop_y = sub_h * (2 - frame_mbs_only);
    let (mut crop_w, mut crop_h) = (0u64, 0u64);
    if r.bit()? == 1 {
        let (left, right) = (r.ue()?, r.ue()?);
        let (top, bottom) = (r.ue()?, r.ue()?);
        crop_w = crop_x * (left + right);
        crop_h = crop_y * (top + bottom);
    }
    let (coded_w, coded_h) = (width_mbs * 16, height_mbs * 16);
    if crop_w >= coded_w || crop_h >= coded_h {
        return Err(DimsError::Invalid);
    }
    Ok(Dims {
        width_mbs,
        height_mbs,
        width_px: coded_w - crop_w,
        height_px: coded_h - crop_h,
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// MSB-first bit writer for building SPS bodies byte-exact.
    #[derive(Default)]
    pub struct W {
        bytes: Vec<u8>,
        n: usize,
    }

    impl W {
        pub fn bit(&mut self, b: u64) -> &mut Self {
            if self.n.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if b & 1 == 1 {
                *self.bytes.last_mut().unwrap() |= 0x80 >> (self.n % 8);
            }
            self.n += 1;
            self
        }
        pub fn bits(&mut self, v: u64, n: u32) -> &mut Self {
            for i in (0..n).rev() {
                self.bit(v >> i);
            }
            self
        }
        pub fn ue(&mut self, v: u64) -> &mut Self {
            let x = v + 1;
            let len = 64 - x.leading_zeros();
            self.bits(0, len - 1).bits(x, len)
        }
        pub fn se(&mut self, v: i64) -> &mut Self {
            let k = if v > 0 { 2 * v - 1 } else { -2 * v };
            self.ue(k as u64)
        }
        /// rbsp_trailing_bits, then emulation prevention.
        pub fn finish(&mut self) -> Vec<u8> {
            self.bit(1);
            while !self.n.is_multiple_of(8) {
                self.bit(0);
            }
            let mut out = Vec::new();
            let mut zeros = 0;
            for &b in &self.bytes {
                if zeros >= 2 && b <= 3 {
                    out.push(3);
                    zeros = 0;
                }
                zeros = if b == 0 { zeros + 1 } else { 0 };
                out.push(b);
            }
            out
        }
    }

    pub struct Sps {
        pub profile: u64,
        pub width_mbs: u64,
        pub map_units: u64,
        pub frame_mbs_only: bool,
        pub crop: Option<(u64, u64, u64, u64)>,
        pub chroma_format_idc: u64,
        pub scaling: bool,
        pub poc_type: u64,
    }

    impl Sps {
        pub fn baseline(width_mbs: u64, height_mbs: u64) -> Self {
            Self {
                profile: 66,
                width_mbs,
                map_units: height_mbs,
                frame_mbs_only: true,
                crop: None,
                chroma_format_idc: 1,
                scaling: false,
                poc_type: 2,
            }
        }

        /// The SPS NAL unit: header byte 0x67, then the escaped body.
        pub fn nal(&self) -> Vec<u8> {
            let mut w = W::default();
            w.bits(self.profile, 8).bits(0xc0, 8).bits(31, 8).ue(0);
            if high_profile(self.profile) {
                w.ue(self.chroma_format_idc);
                if self.chroma_format_idc == 3 {
                    w.bit(0);
                }
                w.ue(0).ue(0).bit(0);
                w.bit(u64::from(self.scaling));
                if self.scaling {
                    let lists = if self.chroma_format_idc != 3 { 8 } else { 12 };
                    for i in 0..lists {
                        w.bit(1);
                        let size = if i < 6 { 16 } else { 64 };
                        // A varied list: deltas walking up and back.
                        for j in 0..size {
                            w.se(if j % 2 == 0 { 3 } else { -2 });
                        }
                    }
                }
            }
            w.ue(0); // log2_max_frame_num_minus4
            w.ue(self.poc_type);
            match self.poc_type {
                0 => {
                    w.ue(2);
                }
                1 => {
                    w.bit(0).se(-1).se(2).ue(3).se(1).se(-5).se(7);
                }
                _ => {}
            }
            w.ue(1).bit(0); // max_num_ref_frames, gaps
            w.ue(self.width_mbs - 1).ue(self.map_units - 1);
            w.bit(u64::from(self.frame_mbs_only));
            if !self.frame_mbs_only {
                w.bit(0);
            }
            w.bit(1); // direct_8x8_inference_flag
            match self.crop {
                Some((l, r, t, b)) => {
                    w.bit(1).ue(l).ue(r).ue(t).ue(b);
                }
                None => {
                    w.bit(0);
                }
            }
            w.bit(0); // vui_parameters_present_flag
            let mut nal = vec![0x67];
            nal.extend(w.finish());
            nal
        }
    }

    /// A VP8 RTP payload: descriptor (with a 15-bit PictureID, as browsers
    /// send) and, for a keyframe, the frame tag, start code and size.
    pub fn vp8_payload(keyframe: bool, start: bool, w: u16, h: u16) -> Vec<u8> {
        let mut p = vec![0x80 | if start { 0x10 } else { 0 }, 0x80, 0x80 | 0x12, 0x34];
        if keyframe {
            p.extend_from_slice(&[0x50, 0x2a, 0x01]); // tag: P=0, show_frame
            p.extend_from_slice(&[0x9d, 0x01, 0x2a]);
            p.extend_from_slice(&w.to_le_bytes());
            p.extend_from_slice(&h.to_le_bytes());
        } else {
            p.extend_from_slice(&[0x31, 0x02, 0x00]); // tag: P=1
        }
        p.extend_from_slice(&[0xaa; 40]);
        p
    }

    /// `packet_dims` at the default max-fs.
    fn pd(fmt: VideoFormat, p: &[u8]) -> Option<Result<Dims, DimsError>> {
        packet_dims(fmt, p, 3600)
    }

    fn mbs(d: Dims) -> (u64, u64, u64, u64) {
        (d.width_mbs, d.height_mbs, d.width_px, d.height_px)
    }

    #[test]
    fn vp8_keyframe_dimensions() {
        let d = pd(VideoFormat::Vp8, &vp8_payload(true, true, 1280, 720))
            .unwrap()
            .unwrap();
        assert_eq!(mbs(d), (80, 45, 1280, 720));
        assert!(!d.exceeds(3600));
        // portrait from a phone
        let d = pd(VideoFormat::Vp8, &vp8_payload(true, true, 720, 1280))
            .unwrap()
            .unwrap();
        assert_eq!(mbs(d), (45, 80, 720, 1280));
        assert!(!d.exceeds(3600));
        // 1080p is 120 x 68 = 8160 macroblocks
        let d = pd(VideoFormat::Vp8, &vp8_payload(true, true, 1920, 1080))
            .unwrap()
            .unwrap();
        assert_eq!(mbs(d), (120, 68, 1920, 1080));
        assert!(d.exceeds(3600));
        // scale bits are not part of the size
        let mut p = vp8_payload(true, true, 640, 480);
        p[11] |= 0xc0;
        p[13] |= 0x40;
        let d = pd(VideoFormat::Vp8, &p).unwrap().unwrap();
        assert_eq!(mbs(d), (40, 30, 640, 480));
    }

    #[test]
    fn vp8_only_the_first_packet_of_a_keyframe_carries_a_size() {
        assert_eq!(pd(VideoFormat::Vp8, &vp8_payload(false, true, 0, 0)), None);
        assert_eq!(
            pd(VideoFormat::Vp8, &vp8_payload(true, false, 1920, 1080)),
            None
        );
        // PID != 0: a later partition
        let mut p = vp8_payload(true, true, 1920, 1080);
        p[0] |= 0x01;
        assert_eq!(pd(VideoFormat::Vp8, &p), None);
        // Minimal descriptor (no X byte)
        let mut p = vec![0x10];
        p.extend_from_slice(&vp8_payload(true, true, 1920, 1080)[4..]);
        assert!(pd(VideoFormat::Vp8, &p).unwrap().unwrap().exceeds(3600));
        // Every extension field present: I (7-bit), L, T/K
        let mut p = vec![0x90, 0xf0, 0x05, 0x01, 0x02];
        p.extend_from_slice(&vp8_payload(true, true, 320, 240)[4..]);
        assert_eq!(
            mbs(pd(VideoFormat::Vp8, &p).unwrap().unwrap()),
            (20, 15, 320, 240)
        );
    }

    #[test]
    fn vp8_malformed_keyframe_header_is_an_error() {
        let mut p = vp8_payload(true, true, 1280, 720);
        p[7] = 0x9c; // start code
        assert_eq!(pd(VideoFormat::Vp8, &p), Some(Err(DimsError::Invalid)));
        let p = vp8_payload(true, true, 1280, 720);
        assert_eq!(
            pd(VideoFormat::Vp8, &p[..12]),
            Some(Err(DimsError::Truncated))
        );
        let p = vp8_payload(true, true, 0, 720);
        assert_eq!(pd(VideoFormat::Vp8, &p), Some(Err(DimsError::Invalid)));
    }

    #[test]
    fn h264_baseline_sps_single_nal() {
        let nal = Sps::baseline(80, 45).nal();
        let d = pd(VideoFormat::H264, &nal).unwrap().unwrap();
        assert_eq!(mbs(d), (80, 45, 1280, 720));
        assert!(!d.exceeds(3600));
        let d = pd(VideoFormat::H264, &Sps::baseline(45, 80).nal())
            .unwrap()
            .unwrap();
        assert_eq!(mbs(d), (45, 80, 720, 1280));
        assert!(!d.exceeds(3600));
    }

    #[test]
    fn h264_1080p_with_cropping_is_over_the_limit() {
        // 1920x1088 coded, cropped 8 rows (4 chroma rows) to 1080.
        let mut sps = Sps::baseline(120, 68);
        sps.profile = 100; // high, so the chroma branch is exercised
        sps.crop = Some((0, 0, 0, 4));
        sps.poc_type = 0;
        let d = pd(VideoFormat::H264, &sps.nal()).unwrap().unwrap();
        assert_eq!(mbs(d), (120, 68, 1920, 1080));
        assert!(d.exceeds(3600));
    }

    #[test]
    fn h264_high_profile_with_scaling_lists_and_field_coding() {
        let mut sps = Sps::baseline(40, 15);
        sps.profile = 100;
        sps.scaling = true;
        sps.frame_mbs_only = false; // 15 map units = 30 MB rows
        sps.poc_type = 1;
        sps.crop = Some((1, 1, 0, 2)); // x2 per unit wide; x4 per unit high (field)
        let d = pd(VideoFormat::H264, &sps.nal()).unwrap().unwrap();
        assert_eq!(mbs(d), (40, 30, 640 - 4, 480 - 8));
        // 4:4:4 with its 12 scaling lists
        let mut sps = Sps::baseline(80, 45);
        sps.profile = 244;
        sps.chroma_format_idc = 3;
        sps.scaling = true;
        sps.crop = Some((2, 2, 1, 1)); // 4:4:4 crop unit is 1
        let d = pd(VideoFormat::H264, &sps.nal()).unwrap().unwrap();
        assert_eq!(mbs(d), (80, 45, 1276, 718));
    }

    /// x264's SPS for 1920x1080 High (profile_idc 100, level 4.0, 4:2:0,
    /// cropped 1088 -> 1080), byte for byte as it writes it.
    const X264_HIGH_1080: [u8; 27] = [
        0x67, 0x64, 0x00, 0x28, 0xac, 0xd9, 0x40, 0x78, 0x02, 0x27, 0xe5, 0xc0, 0x44, 0x00, 0x00,
        0x03, 0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xf0, 0x3c, 0x60, 0xc6, 0x58,
    ];

    #[test]
    fn h264_real_high_sps_is_read() {
        let d = pd(VideoFormat::H264, &X264_HIGH_1080).unwrap().unwrap();
        assert_eq!(mbs(d), (120, 68, 1920, 1080));
        assert!(d.exceeds(3600));
    }

    #[test]
    fn h264_unknown_profile_is_malformed() {
        // The same 1080p SPS relabelled to profile_idc 144 (the retired High
        // 4:4:4, which FFmpeg still reads with the high-profile fields and
        // decodes at 1920x1080). Read as a profile without those fields it
        // misaligns to a tiny size and would pass — so it must not be read.
        let mut p144 = X264_HIGH_1080;
        p144[1] = 144;
        assert_eq!(pd(VideoFormat::H264, &p144), Some(Err(DimsError::Invalid)));
        // Any profile_idc outside the spec's list, likewise.
        for profile in [0u64, 1, 65, 67, 99, 101, 143, 144, 145, 255] {
            let mut sps = Sps::baseline(40, 30);
            sps.profile = profile;
            assert_eq!(
                pd(VideoFormat::H264, &sps.nal()),
                Some(Err(DimsError::Invalid)),
                "profile_idc {profile}"
            );
        }
        // Every profile the spec defines is still read.
        for profile in [
            66u64, 77, 88, 100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135,
        ] {
            let mut sps = Sps::baseline(40, 30);
            sps.profile = profile;
            let d = pd(VideoFormat::H264, &sps.nal()).unwrap().unwrap();
            assert_eq!(
                (d.width_mbs, d.height_mbs),
                (40, 30),
                "profile_idc {profile}"
            );
        }
    }

    #[test]
    fn h264_sps_in_stap_a_and_fu_a() {
        let sps = Sps::baseline(120, 68).nal();
        let pps = [0x68, 0xce, 0x3c, 0x80];
        let mut stap = vec![0x78]; // F=0 NRI=3 type 24
        for nal in [&sps[..], &pps[..]] {
            stap.extend_from_slice(&(nal.len() as u16).to_be_bytes());
            stap.extend_from_slice(nal);
        }
        let d = pd(VideoFormat::H264, &stap).unwrap().unwrap();
        assert_eq!((d.width_mbs, d.height_mbs), (120, 68));

        // An oversized SPS may not hide behind a compliant one.
        let small = Sps::baseline(40, 30).nal();
        let mut stap2 = vec![0x78];
        for nal in [&sps[..], &small[..]] {
            stap2.extend_from_slice(&(nal.len() as u16).to_be_bytes());
            stap2.extend_from_slice(nal);
        }
        assert!(pd(VideoFormat::H264, &stap2)
            .unwrap()
            .unwrap()
            .exceeds(3600));
        // ...nor behind one placed before it: every SPS in the aggregate is
        // read, not just the first.
        let mut stap3 = vec![0x78];
        for nal in [&small[..], &pps[..], &sps[..]] {
            stap3.extend_from_slice(&(nal.len() as u16).to_be_bytes());
            stap3.extend_from_slice(nal);
        }
        let d = pd(VideoFormat::H264, &stap3).unwrap().unwrap();
        assert_eq!((d.width_mbs, d.height_mbs), (120, 68));
        // and a malformed one after a compliant one fails the packet too
        let mut stap4 = vec![0x78];
        let mut p144 = X264_HIGH_1080;
        p144[1] = 144;
        for nal in [&small[..], &p144[..]] {
            stap4.extend_from_slice(&(nal.len() as u16).to_be_bytes());
            stap4.extend_from_slice(nal);
        }
        assert_eq!(pd(VideoFormat::H264, &stap4), Some(Err(DimsError::Invalid)));

        // FU-A start fragment of an SPS
        let mut fu = vec![0x7c, 0x80 | 7];
        fu.extend_from_slice(&sps[1..]);
        let d = pd(VideoFormat::H264, &fu).unwrap().unwrap();
        assert_eq!((d.width_mbs, d.height_mbs), (120, 68));
        // ...not a continuation fragment, nor an IDR
        let mut cont = vec![0x7c, 7];
        cont.extend_from_slice(&sps[1..]);
        assert_eq!(pd(VideoFormat::H264, &cont), None);
        assert_eq!(pd(VideoFormat::H264, &[0x65, 0x88, 0x80, 0x40]), None);
        assert_eq!(pd(VideoFormat::H264, &[0x7c, 0x85, 0x88, 0x80]), None);
        // An FU-A SPS split too early to reach the size fails closed.
        assert!(matches!(pd(VideoFormat::H264, &fu[..6]), Some(Err(_))));
    }

    #[test]
    fn h264_emulation_prevention_is_removed() {
        // Long exp-Golomb prefixes put runs of zero bits in the body; the
        // escaper inserts 03s wherever they would read as a start code, and
        // the parser has to take them out again to reach the right sizes.
        let mut escaped = 0;
        for k in 8..24 {
            for w in [1u64 << k, (1u64 << k) - 1] {
                let mut sps = Sps::baseline(w, 1 + k);
                sps.crop = Some((0, 0, 0, 0));
                let nal = sps.nal();
                if nal.windows(3).any(|x| x == [0, 0, 3]) {
                    escaped += 1;
                }
                let d = pd(VideoFormat::H264, &nal).unwrap().unwrap();
                assert_eq!((d.width_mbs, d.height_mbs), (w, 1 + k), "{nal:02x?}");
            }
        }
        assert!(escaped > 0, "no case exercised emulation prevention");
    }

    #[test]
    fn dims_limit_is_macroblocks_and_the_aspect_rule() {
        let d = |w, h| Dims {
            width_mbs: w,
            height_mbs: h,
            width_px: w * 16,
            height_px: h * 16,
        };
        assert!(!d(80, 45).exceeds(3600));
        assert!(!d(45, 80).exceeds(3600));
        assert!(d(81, 45).exceeds(3600));
        // sqrt(8 * 3600) = 169.7: a 169-wide sliver passes, 170 does not
        assert!(!d(169, 21).exceeds(3600));
        assert!(d(170, 1).exceeds(3600));
        assert!(!d(10_000, 10_000).exceeds(0));
    }

    /// xorshift — deterministic "fuzz" input.
    fn noise(seed: &mut u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    }

    #[test]
    fn garbage_and_truncation_never_panic() {
        let mut seed = 0x1234_5678_9abc_def0u64;
        let good = [
            Sps::baseline(80, 45).nal(),
            {
                let mut s = Sps::baseline(40, 15);
                s.profile = 100;
                s.scaling = true;
                s.poc_type = 1;
                s.crop = Some((1, 1, 0, 2));
                s.nal()
            },
            vp8_payload(true, true, 1280, 720),
        ];
        // every prefix of real headers, as every format and packetisation
        for g in &good {
            for n in 0..=g.len() {
                for fmt in [VideoFormat::Vp8, VideoFormat::H264] {
                    let _ = pd(fmt, &g[..n]);
                    let mut fu = vec![0x7c, 0x87];
                    fu.extend_from_slice(&g[..n]);
                    let _ = pd(fmt, &fu);
                    let mut stap = vec![0x78, 0xff, 0xff];
                    stap.extend_from_slice(&g[..n]);
                    let _ = pd(fmt, &stap);
                }
                let _ = sps_dims(&g[..n]);
            }
        }
        // random bytes, random lengths, with real NAL / descriptor prefixes
        for i in 0..20_000 {
            let len = (noise(&mut seed) % 64) as usize;
            let mut p: Vec<u8> = (0..len).map(|_| noise(&mut seed) as u8).collect();
            match i % 4 {
                0 if !p.is_empty() => p[0] = 0x67,
                1 if p.len() > 1 => {
                    p[0] = 0x7c;
                    p[1] = 0x87;
                }
                2 if !p.is_empty() => p[0] = 0x78,
                3 if !p.is_empty() => p[0] = 0x90,
                _ => {}
            }
            let _ = pd(VideoFormat::H264, &p);
            let _ = pd(VideoFormat::Vp8, &p);
            let _ = sps_dims(&p);
            let _ = rtp_payload(&p);
        }
        // pathological exp-Golomb: all zero bits, all one bits
        assert!(sps_dims(&[0u8; 64]).is_err());
        let _ = sps_dims(&[0xffu8; 1600]); // longer than any buffer
        assert!(sps_dims(&[]).is_err());
    }

    #[test]
    fn rtp_payload_skips_csrcs_extension_and_padding() {
        let mut p = vec![0x80 | 0x10 | 0x20 | 0x02, 96, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1];
        p.extend_from_slice(&[0; 8]); // two CSRCs
        p.extend_from_slice(&[0xbe, 0xde, 0, 1, 0x10, 0xff, 0, 0]); // one-word extension
        p.extend_from_slice(&[1, 2, 3]); // payload
        p.extend_from_slice(&[0, 0, 3]); // 3 bytes of padding
        assert_eq!(rtp_payload(&p), Some(&[1u8, 2, 3][..]));
        // extension length running off the end
        let mut bad = p.clone();
        bad[22] = 0xff;
        assert_eq!(rtp_payload(&bad), None);
        // padding longer than the packet
        let mut bad = p.clone();
        *bad.last_mut().unwrap() = 200;
        assert_eq!(rtp_payload(&bad), None);
        assert_eq!(rtp_payload(&[0x80, 96]), None);
    }
}
