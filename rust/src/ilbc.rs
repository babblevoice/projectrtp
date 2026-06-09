// iLBC — thin safe wrapper over the WebRTC libilbc C API (libilbc.so.3).
//
// We only use the 6 functions we need for 20 ms-frame encode/decode:
//
//   WebRtcIlbcfix_EncoderCreate / WebRtcIlbcfix_EncoderFree
//   WebRtcIlbcfix_EncoderInit
//   WebRtcIlbcfix_Encode
//   WebRtcIlbcfix_DecoderCreate / WebRtcIlbcfix_DecoderFree
//   WebRtcIlbcfix_DecoderInit
//   WebRtcIlbcfix_Decode
//
// RTP payload type 97 (dynamic), 20 ms frames — 38 bytes on the wire, 160
// samples when decoded at 8 kHz. Matches what C++ projectrtpcodecx does via
// the system `-lilbc` link.
//
// Header for the FFI signatures is vendored at `libilbc/ilbc.h` in the repo;
// we duplicate the declarations here so the crate has no bindgen dependency.

use std::os::raw::{c_int, c_void};

/// Encoder/decoder instances are opaque structs allocated by the library.
#[repr(C)]
pub struct IlbcEncoderInstance {
    _private: [u8; 0],
}

#[repr(C)]
pub struct IlbcDecoderInstance {
    _private: [u8; 0],
}

// Link directive lives in `build.rs` so we can select the right library
// filename at build time (Fedora ships only `libilbc.so.3`, while
// Alpine/Debian `-dev` packages provide the unversioned `libilbc.so`).
extern "C" {
    fn WebRtcIlbcfix_EncoderCreate(enc: *mut *mut IlbcEncoderInstance) -> i16;
    fn WebRtcIlbcfix_EncoderFree(enc: *mut IlbcEncoderInstance) -> i16;
    fn WebRtcIlbcfix_EncoderInit(enc: *mut IlbcEncoderInstance, frame_len_ms: i16) -> i16;
    fn WebRtcIlbcfix_Encode(
        enc: *mut IlbcEncoderInstance,
        speech_in: *const i16,
        len: usize,
        encoded: *mut u8,
    ) -> c_int;

    fn WebRtcIlbcfix_DecoderCreate(dec: *mut *mut IlbcDecoderInstance) -> i16;
    fn WebRtcIlbcfix_DecoderFree(dec: *mut IlbcDecoderInstance) -> i16;
    fn WebRtcIlbcfix_DecoderInit(dec: *mut IlbcDecoderInstance, frame_len_ms: i16) -> i16;
    fn WebRtcIlbcfix_Decode(
        dec: *mut IlbcDecoderInstance,
        encoded: *const u8,
        len: usize,
        decoded: *mut i16,
        speech_type: *mut i16,
    ) -> c_int;
}

/// 20 ms iLBC frame — constants the RTP layer relies on.
pub const FRAME_SAMPLES_20MS: usize = 160;
pub const FRAME_BYTES_20MS: usize = 38;
pub const FRAME_LEN_MS: i16 = 20;

/// Safe encoder wrapper. `Drop` frees the C-side instance.
pub struct Encoder {
    inst: *mut IlbcEncoderInstance,
}

// The underlying WebRTC code holds per-instance state behind the pointer and
// has no thread-local globals; fine to move between threads, which is the
// pattern tokio tasks require.
unsafe impl Send for Encoder {}

impl Encoder {
    pub fn new() -> Option<Self> {
        let mut inst: *mut IlbcEncoderInstance = std::ptr::null_mut();
        let r = unsafe { WebRtcIlbcfix_EncoderCreate(&mut inst) };
        if r != 0 || inst.is_null() {
            return None;
        }
        let init = unsafe { WebRtcIlbcfix_EncoderInit(inst, FRAME_LEN_MS) };
        if init != 0 {
            unsafe {
                WebRtcIlbcfix_EncoderFree(inst);
            }
            return None;
        }
        Some(Self { inst })
    }

    /// Encode one 20 ms frame (160 samples at 8 kHz). Returns the encoded
    /// 38 bytes. `None` for the wrong input length or an FFI error.
    pub fn encode_20ms(&mut self, samples: &[i16]) -> Option<Vec<u8>> {
        if samples.len() != FRAME_SAMPLES_20MS {
            return None;
        }
        let mut out = vec![0u8; FRAME_BYTES_20MS];
        let n = unsafe {
            WebRtcIlbcfix_Encode(self.inst, samples.as_ptr(), samples.len(), out.as_mut_ptr())
        };
        if n <= 0 {
            return None;
        }
        out.truncate(n as usize);
        Some(out)
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        if !self.inst.is_null() {
            unsafe {
                WebRtcIlbcfix_EncoderFree(self.inst);
            }
            self.inst = std::ptr::null_mut();
        }
    }
}

/// Safe decoder wrapper.
pub struct Decoder {
    inst: *mut IlbcDecoderInstance,
}

unsafe impl Send for Decoder {}

impl Decoder {
    pub fn new() -> Option<Self> {
        let mut inst: *mut IlbcDecoderInstance = std::ptr::null_mut();
        let r = unsafe { WebRtcIlbcfix_DecoderCreate(&mut inst) };
        if r != 0 || inst.is_null() {
            return None;
        }
        let init = unsafe { WebRtcIlbcfix_DecoderInit(inst, FRAME_LEN_MS) };
        if init != 0 {
            unsafe {
                WebRtcIlbcfix_DecoderFree(inst);
            }
            return None;
        }
        Some(Self { inst })
    }

    /// Decode one iLBC payload. Returns the PCM samples (160 for a 20 ms
    /// frame). CNG frames and errors return `None`; the caller treats them
    /// as silence, matching the C++ behavior on decode failures.
    pub fn decode(&mut self, payload: &[u8]) -> Option<Vec<i16>> {
        // Output buffer: 160 samples for a 20 ms frame — allocate the max
        // (240 samples for 30 ms) to be safe against future frame-length
        // extensions without reallocating.
        let mut out = vec![0i16; 240];
        let mut speech_type: i16 = 0;
        let n = unsafe {
            WebRtcIlbcfix_Decode(
                self.inst,
                payload.as_ptr(),
                payload.len(),
                out.as_mut_ptr(),
                &mut speech_type,
            )
        };
        if n <= 0 {
            return None;
        }
        out.truncate(n as usize);
        Some(out)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        if !self.inst.is_null() {
            unsafe {
                WebRtcIlbcfix_DecoderFree(self.inst);
            }
            self.inst = std::ptr::null_mut();
        }
    }
}

// Silence unused warning for the raw `c_void` reference until another iLBC
// helper wants to pass one through.
const _: fn() -> *mut c_void = || std::ptr::null_mut();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_one_frame_is_stable() {
        let mut enc = Encoder::new().expect("encoder create");
        let mut dec = Decoder::new().expect("decoder create");
        // A quiet sine — not silence, or the decoder may return CNG.
        let samples: Vec<i16> = (0..160)
            .map(|i| ((i as f32 * 0.1).sin() * 1000.0) as i16)
            .collect();
        let encoded = enc.encode_20ms(&samples).expect("encode");
        assert_eq!(encoded.len(), FRAME_BYTES_20MS);
        let decoded = dec.decode(&encoded).expect("decode");
        assert_eq!(decoded.len(), FRAME_SAMPLES_20MS);
    }
}
