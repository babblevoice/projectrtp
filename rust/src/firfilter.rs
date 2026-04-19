// FIR / MA / DC filters — port of projectrtpfirfilter.{cpp,h}.
//
// JS-facing surface (via #[napi]): rtpfilter.filterlowfir(buffer) — mutates a
// Buffer of big-endian int16 samples in place through the lowpass filter and
// returns true. Used by tests only; not on the production hot path.

use napi::bindgen_prelude::*;
use napi_derive::napi;

pub const LOWPASS_LEN: usize = 17;

// Coefficients lifted verbatim from the C++ version (Kaiser-Bessel, 16 kHz
// sampling, 3.4 kHz cutoff — for 16k→8k downsampling anti-alias).
const LP_COEFFS: [f32; LOWPASS_LEN] = [
    -0.002102, 0.000519, 0.014189, 0.010317, -0.037919, -0.060378, 0.063665,
    0.299972, 0.425000, 0.299972, 0.063665, -0.060378, -0.037919, 0.010317,
    0.014189, 0.000519, -0.002102,
];

pub struct Lowpass3_4k16k {
    history: [f32; LOWPASS_LEN],
    round: usize,
}

impl Default for Lowpass3_4k16k {
    fn default() -> Self { Self::new() }
}

impl Lowpass3_4k16k {
    pub fn new() -> Self { Self { history: [0.0; LOWPASS_LEN], round: 0 } }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.history = [0.0; LOWPASS_LEN];
        self.round = 0;
    }

    pub fn execute(&mut self, val: i16) -> i16 {
        let j0 = self.round;
        self.history[j0] = val as f32;
        let mut runtot = 0.0f32;
        let mut i = 0usize;
        let mut j = (j0 + 1) % LOWPASS_LEN;
        while j < LOWPASS_LEN {
            runtot += LP_COEFFS[i] * self.history[j];
            i += 1;
            j += 1;
        }
        let mut j = 0usize;
        while i < LOWPASS_LEN {
            runtot += LP_COEFFS[i] * self.history[j];
            i += 1;
            j += 1;
        }
        self.round = (self.round + 1) % LOWPASS_LEN;
        runtot as i16
    }
}

pub const MA_LENGTH: usize = 50 * 5;

pub struct MaFilter {
    history: [f32; MA_LENGTH],
    round: usize,
    l: usize,
    rtotal: i32,
}

impl Default for MaFilter {
    fn default() -> Self { Self::new() }
}

impl MaFilter {
    pub fn new() -> Self {
        Self { history: [0.0; MA_LENGTH], round: 0, l: MA_LENGTH, rtotal: 0 }
    }

    pub fn reset(&mut self, packets: usize) {
        self.l = packets.min(MA_LENGTH);
        self.rtotal = 0;
        self.history = [0.0; MA_LENGTH];
        self.round = 0;
    }

    pub fn execute(&mut self, val: i16) -> i16 {
        self.rtotal -= self.history[self.round] as i32;
        self.rtotal += val as i32;
        self.history[self.round] = val as f32;
        self.round = (self.round + 1) % self.l;
        (self.rtotal / self.l as i32) as i16
    }

    #[allow(dead_code)]
    pub fn get(&self) -> i32 { self.rtotal / self.l as i32 }
    #[allow(dead_code)]
    pub fn length(&self) -> usize { self.l }
}

// DC blocker. y[n] = x[n] - x[n-1] + 0.995 * y[n-1]
pub struct DcFilter { xm: i16, ym: i16 }

impl Default for DcFilter {
    fn default() -> Self { Self::new() }
}

impl DcFilter {
    pub fn new() -> Self { Self { xm: 0, ym: 0 } }
    #[allow(dead_code)]
    pub fn reset(&mut self) { self.xm = 0; self.ym = 0; }

    #[inline]
    pub fn execute(&mut self, x: i16) -> i16 {
        let y = (x as f32 - self.xm as f32 + 0.995 * self.ym as f32) as i16;
        self.xm = x;
        self.ym = y;
        y
    }
}

// rtpfilter.filterlowfir(buf) — in-place lowpass across a Buffer of BE int16.
#[napi(namespace = "rtpfilter", js_name = "filterlowfir")]
pub fn js_filter_lowfir(mut buf: Buffer) -> Result<bool> {
    let bytes: &mut [u8] = buf.as_mut();
    if bytes.len() % 2 != 0 {
        return Err(Error::from_reason("buffer length must be even (BE int16)"));
    }
    let mut filter = Lowpass3_4k16k::new();
    for chunk in bytes.chunks_exact_mut(2) {
        let v = i16::from_be_bytes([chunk[0], chunk[1]]);
        let out = filter.execute(v);
        let enc = out.to_be_bytes();
        chunk[0] = enc[0];
        chunk[1] = enc[1];
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowpass_zero_input_zero_output() {
        let mut f = Lowpass3_4k16k::new();
        for _ in 0..100 { assert_eq!(f.execute(0), 0); }
    }

    #[test]
    fn lowpass_stable_on_dc() {
        // Sum of coefficients is ~1.0 → DC input passes through near-unity.
        let mut f = Lowpass3_4k16k::new();
        let mut last = 0i16;
        for _ in 0..100 { last = f.execute(1000); }
        assert!((last as i32 - 1000).abs() < 50, "DC gain off: {last}");
    }

    #[test]
    fn ma_reaches_target_for_constant_input() {
        let mut m = MaFilter::new();
        m.reset(MA_LENGTH);
        for _ in 0..MA_LENGTH { m.execute(1); }
        assert_eq!(m.get(), 1);
        for _ in 0..(MA_LENGTH / 2) { m.execute(100); }
        assert_eq!(m.get(), 50);
        for _ in 0..(MA_LENGTH / 2) { m.execute(100); }
        assert_eq!(m.get(), 100);
    }

    #[test]
    fn dc_filter_removes_dc() {
        let mut f = DcFilter::new();
        let mut last = 0i16;
        for _ in 0..2000 { last = f.execute(5000); }
        assert!(last.abs() < 50, "DC not removed: {last}");
    }
}
