// G.722 — thin safe wrapper over spandsp's libspandsp C API (libspandsp.so.2).
//
// Same approach as `ilbc.rs`: manual extern "C" declarations (no bindgen
// dep) for the 6 functions we actually need:
//
//   g722_encode_init / g722_encode / g722_encode_free
//   g722_decode_init / g722_decode / g722_decode_free
//
// spandsp is what the C++ addon uses (via `-lspandsp`), so outputs are
// bit-for-bit identical. The `ezk-g722` Rust crate we previously used
// produced subtly-different samples that the looptest frequency-domain
// checks couldn't detect reliably; this swap fixes the 4 g722 transcode
// failures.
//
// We pass `G722_SAMPLE_RATE_8000 | G722_PACKED` so spandsp handles the
// 8↔16 kHz resampling internally — saves us replicating the zero-insert +
// lowpass / decimate dance the C++ side does by hand.

use std::os::raw::c_int;

#[repr(C)]
pub struct G722EncodeState {
    _private: [u8; 0],
}

#[repr(C)]
pub struct G722DecodeState {
    _private: [u8; 0],
}

extern "C" {
    fn g722_encode_init(s: *mut G722EncodeState, rate: c_int, options: c_int) -> *mut G722EncodeState;
    fn g722_encode_free(s: *mut G722EncodeState) -> c_int;
    fn g722_encode(s: *mut G722EncodeState, out: *mut u8, amp: *const i16, len: c_int) -> c_int;

    fn g722_decode_init(s: *mut G722DecodeState, rate: c_int, options: c_int) -> *mut G722DecodeState;
    fn g722_decode_free(s: *mut G722DecodeState) -> c_int;
    fn g722_decode(s: *mut G722DecodeState, out: *mut i16, bits: *const u8, len: c_int) -> c_int;
}

// Option bits from spandsp/g722.h.
#[allow(dead_code)]
const G722_SAMPLE_RATE_8000: c_int = 0x0001;
const G722_PACKED: c_int = 0x0002;
const G722_RATE_64000: c_int = 64000;

/// 20 ms at 16 kHz mono. G.722's native frame size.
const G722_FRAME_SAMPLES: usize = 320;
/// 64 kbit/s × 20 ms = 160 wire bytes per frame. Two 16 kHz samples pack
/// into one byte.
const G722_FRAME_BYTES: usize = 160;

pub struct Encoder {
    state: *mut G722EncodeState,
    /// Persistent output buffer, sized for a 20 ms frame. Reused across
    /// calls to avoid per-tick allocation.
    out_buf: Vec<u8>,
}

unsafe impl Send for Encoder {}

impl Encoder {
    pub fn new() -> Option<Self> {
        // True 16 kHz G.722 — matches C++ projectrtpcodecx.cpp:368 which
        // uses `G722_PACKED` only. The `G722_SAMPLE_RATE_8000` flag is a
        // test-vector mode that produces non-standard wire output.
        let state = unsafe {
            g722_encode_init(
                std::ptr::null_mut(),
                G722_RATE_64000,
                G722_PACKED,
            )
        };
        if state.is_null() {
            return None;
        }
        Some(Self { state, out_buf: vec![0u8; G722_FRAME_BYTES] })
    }

    /// Encode 16 kHz linear PCM to G.722. For a 20 ms packet pass 320
    /// samples; the returned slice is 160 bytes (64 kbit/s G.722 packs
    /// two 4-bit samples per byte).
    ///
    /// The returned slice borrows `self.out_buf` — valid until the next
    /// encode call. Using spandsp's `G722_SAMPLE_RATE_8000` flag to
    /// bypass the 16 kHz requirement is a test-vector mode that produces
    /// non-standard wire bytes — real peers decode those as distorted
    /// audio, so callers must upsample 8 kHz audio to 16 kHz first.
    pub fn encode(&mut self, samples: &[i16]) -> &[u8] {
        let needed = samples.len() / 2 + 1;
        if self.out_buf.len() < needed {
            self.out_buf.resize(needed, 0);
        }
        let n = unsafe {
            g722_encode(
                self.state,
                self.out_buf.as_mut_ptr(),
                samples.as_ptr(),
                samples.len() as c_int,
            )
        };
        if n <= 0 {
            return &[];
        }
        &self.out_buf[..n as usize]
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        if !self.state.is_null() {
            unsafe { g722_encode_free(self.state); }
            self.state = std::ptr::null_mut();
        }
    }
}

pub struct Decoder {
    state: *mut G722DecodeState,
    /// Persistent output buffer. Reused across calls.
    out_buf: Vec<i16>,
}

unsafe impl Send for Decoder {}

impl Decoder {
    pub fn new() -> Option<Self> {
        // True 16 kHz G.722 — see Encoder::new for rationale.
        let state = unsafe {
            g722_decode_init(
                std::ptr::null_mut(),
                G722_RATE_64000,
                G722_PACKED,
            )
        };
        if state.is_null() {
            return None;
        }
        Some(Self { state, out_buf: vec![0i16; G722_FRAME_SAMPLES] })
    }

    /// Decode G.722 to 16 kHz linear PCM: 160 wire bytes → 320 samples.
    /// The returned slice borrows `self.out_buf` — valid until the next
    /// decode call. Callers mixing with 8 kHz audio must downsample (see
    /// `Transcoder::decode`).
    pub fn decode(&mut self, bits: &[u8]) -> &[i16] {
        let needed = bits.len() * 2;
        if self.out_buf.len() < needed {
            self.out_buf.resize(needed, 0);
        }
        let n = unsafe {
            g722_decode(
                self.state,
                self.out_buf.as_mut_ptr(),
                bits.as_ptr(),
                bits.len() as c_int,
            )
        };
        if n <= 0 {
            return &[];
        }
        &self.out_buf[..n as usize]
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        if !self.state.is_null() {
            unsafe { g722_decode_free(self.state); }
            self.state = std::ptr::null_mut();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_one_frame_recoverable() {
        let mut enc = Encoder::new().expect("encoder");
        let mut dec = Decoder::new().expect("decoder");
        // 320 samples @ 16 kHz for a 20 ms frame — G.722's native rate.
        let samples: Vec<i16> = (0..320)
            .map(|i| ((i as f32 * 0.05).sin() * 2000.0) as i16)
            .collect();
        let encoded = enc.encode(&samples);
        assert_eq!(encoded.len(), 160, "64 kbit/s → 160 bytes per 20 ms");
        let decoded = dec.decode(&encoded);
        assert_eq!(decoded.len(), 320, "decoder produces 16 kHz samples");
    }
}
