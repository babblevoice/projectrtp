// Tone generator — port of projectrtptonegen.cpp.
//
// Descriptor syntax (see index.js doc): `<freqs>:<cadences_ms>` where
//   <freqs>     = freq1/freq2/... and each freq is  F (single) | F+G (sum) | F~G (sweep)
//                 each may have `*amp` or `*amp1~amp2` appended.
//   <cadences>  = ms1/ms2/...  cycled through alongside frequencies.
//
// Generates 8 kHz 16-bit PCM mono and appends to `filename` (creating a new
// WAV file if none exists, or extending an existing one with matching format).

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use napi::bindgen_prelude::*;
use napi_derive::napi;

const SAMPLE_RATE: u32 = 8000;
const WAV_HEADER_LEN: usize = 44;
const WAVE_FORMAT_PCM: u16 = 1;

fn write_wav_header(out: &mut [u8; WAV_HEADER_LEN], data_bytes: u32) {
    // chunksize here matches the C++ definition: data_bytes + 36 (everything
    // after the initial 8 bytes of RIFF+size).
    let chunksize = data_bytes + 36;
    out[0..4].copy_from_slice(b"RIFF");
    out[4..8].copy_from_slice(&chunksize.to_le_bytes());
    out[8..12].copy_from_slice(b"WAVE");
    out[12..16].copy_from_slice(b"fmt ");
    out[16..20].copy_from_slice(&16u32.to_le_bytes());
    out[20..22].copy_from_slice(&WAVE_FORMAT_PCM.to_le_bytes());
    out[22..24].copy_from_slice(&1u16.to_le_bytes()); // num_channels
    out[24..28].copy_from_slice(&SAMPLE_RATE.to_le_bytes());
    out[28..32].copy_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte_rate
    out[32..34].copy_from_slice(&2u16.to_le_bytes()); // sample_alignment
    out[34..36].copy_from_slice(&16u16.to_le_bytes()); // bit_depth
    out[36..40].copy_from_slice(b"data");
    out[40..44].copy_from_slice(&data_bytes.to_le_bytes());
}

fn read_wav_header(bytes: &[u8; WAV_HEADER_LEN]) -> Option<(u16, u32, u32)> {
    // Returns (audio_format, sample_rate, data_bytes).
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let audio_format = u16::from_le_bytes(bytes[20..22].try_into().unwrap());
    let sample_rate = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
    let data_bytes = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
    Some((audio_format, sample_rate, data_bytes))
}

// Add a tone block into `out`. Matches C++ gentone(): accumulates into the
// existing samples (so `+` gets summed across calls).
fn gentone(
    out: &mut [i16],
    start_freq: f64,
    end_freq: f64,
    start_amp: f64,
    end_amp: f64,
    sample_rate: u32,
) {
    if start_freq == 0.0 && end_freq == 0.0 { return; }
    let n = out.len();
    if n == 0 { return; }
    let amp_per_sample = (end_amp - start_amp) / n as f64;
    let freq_per_sample = (end_freq - start_freq) / n as f64;
    let mut ampatpos = start_amp;
    let mut frequency = start_freq;
    let mut angle = if start_freq == 0.0 {
        0.0
    } else {
        (2.0 * std::f64::consts::PI / sample_rate as f64) * start_freq
    };
    for (i, slot) in out.iter_mut().enumerate() {
        let v = (angle * i as f64).sin() * i16::MAX as f64 * ampatpos;
        // Saturating add to prevent wrap on overlapping `+` tones.
        let sum = (*slot as i32).saturating_add(v as i32)
            .clamp(i16::MIN as i32, i16::MAX as i32);
        *slot = sum as i16;
        ampatpos += amp_per_sample;
        frequency += freq_per_sample;
        angle = if frequency == 0.0 {
            0.0
        } else {
            (2.0 * std::f64::consts::PI / sample_rate as f64) * frequency
        };
    }
}

#[derive(Debug)]
struct AmpSpec { start: f64, end: f64 }

fn parse_amp(spec: &str) -> AmpSpec {
    // spec is the part after `*`, may be `a` or `a~b`.
    let mut it = spec.split('~');
    let a = it.next().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let b = it.next().and_then(|s| s.parse().ok()).unwrap_or(a);
    AmpSpec { start: a, end: b }
}

fn gen_block(out: &mut [i16], freq_spec: &str) {
    // Split off amplitude (`*amp[~amp2]`) first.
    let (freq_part, amp) = match freq_spec.split_once('*') {
        Some((f, a)) => (f, parse_amp(a)),
        None => (freq_spec, AmpSpec { start: 1.0, end: 1.0 }),
    };

    // Detect operator: `+` sum, `~` sweep. `x` modulation not implemented (matches C++).
    let op_pos = freq_part.find(|c: char| c == '+' || c == '~' || c == 'x');
    match op_pos {
        None => {
            let f: f64 = freq_part.parse().unwrap_or(0.0);
            gentone(out, f, f, amp.start, amp.end, SAMPLE_RATE);
        }
        Some(p) => {
            let op = freq_part.as_bytes()[p] as char;
            match op {
                '+' => {
                    for part in freq_part.split(|c: char| c == '+' || c == 'x' || c == '~') {
                        let f: f64 = part.parse().unwrap_or(0.0);
                        gentone(out, f, f, amp.start, amp.end, SAMPLE_RATE);
                    }
                }
                '~' => {
                    let mut parts = freq_part.split(|c: char| c == '+' || c == 'x' || c == '~');
                    let f1: f64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    let f2: f64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(f1);
                    gentone(out, f1, f2, amp.start, amp.end, SAMPLE_RATE);
                }
                _ => {}
            }
        }
    }
}

pub fn generate(tone: &str, filename: &Path) -> Result<()> {
    let (freqs_part, cadences_part) = tone
        .split_once(':')
        .ok_or_else(|| Error::from_reason("tone must be 'freqs:cadences', e.g. 400:1000"))?;

    let frequencies: Vec<&str> = freqs_part.split('/').collect();
    let cadences: Vec<u32> = cadences_part
        .split('/')
        .map(|s| s.trim().parse::<u32>().unwrap_or(0))
        .collect();
    if cadences.is_empty() {
        return Err(Error::from_reason("no cadences"));
    }

    // Total sample count: sum cadences (cycled) for each frequency block.
    let total_samples: u32 = (0..frequencies.len())
        .map(|i| cadences[i % cadences.len()] * SAMPLE_RATE / 1000)
        .sum();

    let mut samples = vec![0i16; total_samples as usize];

    let mut pos = 0usize;
    for (i, freq_spec) in frequencies.iter().enumerate() {
        let ms = cadences[i % cadences.len()];
        let block_len = (ms * SAMPLE_RATE / 1000) as usize;
        if pos + block_len > samples.len() { break; }
        gen_block(&mut samples[pos..pos + block_len], freq_spec);
        pos += block_len;
    }

    // Raw bytes (little-endian i16).
    let mut data_bytes = Vec::with_capacity(samples.len() * 2);
    for s in &samples {
        data_bytes.extend_from_slice(&s.to_le_bytes());
    }

    append_to_wav(filename, &data_bytes)
}

fn append_to_wav(filename: &Path, new_data: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(filename)
        .map_err(|e| Error::from_reason(format!("open {filename:?}: {e}")))?;

    let end = file
        .seek(SeekFrom::End(0))
        .map_err(|e| Error::from_reason(format!("seek: {e}")))?;

    if end == 0 {
        let mut header = [0u8; WAV_HEADER_LEN];
        write_wav_header(&mut header, new_data.len() as u32);
        file.write_all(&header).map_err(wrap_io)?;
        file.write_all(new_data).map_err(wrap_io)?;
        return Ok(());
    }

    // Append path: verify format, update sizes, append data.
    file.seek(SeekFrom::Start(0)).map_err(wrap_io)?;
    let mut hdr = [0u8; WAV_HEADER_LEN];
    file.read_exact(&mut hdr)
        .map_err(|e| Error::from_reason(format!("read header: {e}")))?;

    let (format, sr, existing_data) = read_wav_header(&hdr)
        .ok_or_else(|| Error::from_reason("existing file is not a valid WAV"))?;
    if format != WAVE_FORMAT_PCM || sr != SAMPLE_RATE {
        return Err(Error::from_reason("existing WAV format/samplerate mismatch"));
    }

    let new_total = existing_data + new_data.len() as u32;
    write_wav_header(&mut hdr, new_total);
    file.seek(SeekFrom::Start(0)).map_err(wrap_io)?;
    file.write_all(&hdr).map_err(wrap_io)?;
    file.seek(SeekFrom::End(0)).map_err(wrap_io)?;
    file.write_all(new_data).map_err(wrap_io)?;
    Ok(())
}

fn wrap_io(e: std::io::Error) -> Error { Error::from_reason(e.to_string()) }

#[napi(namespace = "tone", js_name = "generate")]
pub fn js_generate(tone: String, filename: String) -> Result<bool> {
    generate(&tone, Path::new(&filename))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn generates_simple_tone_wav() {
        let tmp = std::env::temp_dir().join("projectrtp_tone_test.wav");
        let _ = fs::remove_file(&tmp);

        generate("400:100", &tmp).expect("gen");
        let data = fs::read(&tmp).expect("read");
        assert_eq!(&data[0..4], b"RIFF");
        assert_eq!(&data[8..12], b"WAVE");
        // 100 ms at 8 kHz * 2 bytes = 1600 bytes of PCM after header.
        assert_eq!(data.len(), WAV_HEADER_LEN + 1600);
        // Contains non-zero samples (a 400 Hz sine in there somewhere).
        assert!(data[WAV_HEADER_LEN..].iter().any(|b| *b != 0));

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn appends_compatible_wav() {
        let tmp = std::env::temp_dir().join("projectrtp_tone_append_test.wav");
        let _ = fs::remove_file(&tmp);

        generate("400:50", &tmp).expect("gen 1");
        generate("500:50", &tmp).expect("gen 2");
        let data = fs::read(&tmp).expect("read");
        // 50 + 50 = 100 ms → 1600 bytes of data.
        assert_eq!(data.len(), WAV_HEADER_LEN + 1600);

        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn cadenced_silence_gap_is_zero() {
        // `400/0:50/50` = 50 ms at 400 Hz, then 50 ms of silence.
        let tmp = std::env::temp_dir().join("projectrtp_tone_silence_test.wav");
        let _ = fs::remove_file(&tmp);

        generate("400/0:50/50", &tmp).expect("gen");
        let data = fs::read(&tmp).expect("read");
        let pcm = &data[WAV_HEADER_LEN..];
        // Second half should be silent.
        let half = pcm.len() / 2;
        assert!(pcm[half..].iter().all(|b| *b == 0), "silence block not zero");

        let _ = fs::remove_file(&tmp);
    }
}
