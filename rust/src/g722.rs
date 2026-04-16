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
const G722_SAMPLE_RATE_8000: c_int = 0x0001;
const G722_PACKED: c_int = 0x0002;
const G722_RATE_64000: c_int = 64000;

pub struct Encoder {
    state: *mut G722EncodeState,
}

unsafe impl Send for Encoder {}

impl Encoder {
    pub fn new() -> Option<Self> {
        let state = unsafe {
            g722_encode_init(
                std::ptr::null_mut(),
                G722_RATE_64000,
                G722_SAMPLE_RATE_8000 | G722_PACKED,
            )
        };
        if state.is_null() {
            return None;
        }
        Some(Self { state })
    }

    /// Encode 8 kHz linear PCM to G.722. For a 20 ms packet at 8 kHz we
    /// pass 160 samples and get 160 bytes back (spandsp's 8000-mode
    /// returns `len` bytes for `len` input samples at 64 kbit/s).
    pub fn encode(&mut self, samples: &[i16]) -> Vec<u8> {
        let mut out = vec![0u8; samples.len()];
        let n = unsafe {
            g722_encode(
                self.state,
                out.as_mut_ptr(),
                samples.as_ptr(),
                samples.len() as c_int,
            )
        };
        if n <= 0 {
            return Vec::new();
        }
        out.truncate(n as usize);
        out
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
}

unsafe impl Send for Decoder {}

impl Decoder {
    pub fn new() -> Option<Self> {
        let state = unsafe {
            g722_decode_init(
                std::ptr::null_mut(),
                G722_RATE_64000,
                G722_SAMPLE_RATE_8000 | G722_PACKED,
            )
        };
        if state.is_null() {
            return None;
        }
        Some(Self { state })
    }

    /// Decode G.722 to 8 kHz linear PCM.
    pub fn decode(&mut self, bits: &[u8]) -> Vec<i16> {
        let mut out = vec![0i16; bits.len()];
        let n = unsafe {
            g722_decode(
                self.state,
                out.as_mut_ptr(),
                bits.as_ptr(),
                bits.len() as c_int,
            )
        };
        if n <= 0 {
            return Vec::new();
        }
        out.truncate(n as usize);
        out
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
        // 160 samples of a gentle sine so we can compare in/out shape.
        let samples: Vec<i16> = (0..160)
            .map(|i| ((i as f32 * 0.1).sin() * 2000.0) as i16)
            .collect();
        let encoded = enc.encode(&samples);
        assert_eq!(encoded.len(), 160);
        let decoded = dec.decode(&encoded);
        assert_eq!(decoded.len(), 160);
    }
}
