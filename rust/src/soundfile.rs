// Soundfile — port of projectrtpsoundfile.{cpp,h}.
//
// Key goal: replace POSIX AIO (see commit 16016ef — every aio_read / aio_write
// must be paired 1:1 with aio_return, and misses caused undefined behavior)
// with tokio async I/O so lifetimes and cleanup are guaranteed by Drop.
//
// Public JS surface: `soundfile.info(filename)` — parse WAV header and return
// a shape matching the existing C++ output for backward-compat.
//
// Internal Rust surface (consumed by channel in Task #7):
//   WavReader::open / read_samples / seek_ms / position_ms / duration_ms
//   WavWriter::create / write_samples / close
//
// Scope: PCM-16 (WAVE_FORMAT_PCM) only at this layer. WAV files stored as
// A-law/µ-law/G.722/iLBC will have their data decoded via `codec.rs` at the
// channel boundary — that keeps this module focused on framing, not codecs.

use std::path::{Path, PathBuf};

use napi::bindgen_prelude::*;
use napi_derive::napi;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};

pub const WAV_HEADER_LEN: usize = 44;
pub const WAVE_FORMAT_PCM: u16 = 0x0001;
#[allow(dead_code)]
pub const WAVE_FORMAT_ALAW: u16 = 0x0006;
#[allow(dead_code)]
pub const WAVE_FORMAT_MULAW: u16 = 0x0007;
#[allow(dead_code)]
pub const WAVE_FORMAT_POLYCOM_G722: u16 = 0xA112;
#[allow(dead_code)]
pub const WAVE_FORMAT_GLOBAL_IP_ILBC: u16 = 0xA116;

// ---- WAV header parse / build ----

#[derive(Debug, Clone, Copy)]
pub struct WavHeader {
    pub audio_format: u16,
    pub num_channels: u16,
    pub sample_rate: u32,
    pub byte_rate: u32,
    pub sample_alignment: u16,
    pub bit_depth: u16,
    pub chunksize: u32,      // riff chunk size (= data_bytes + 36)
    pub fmt_chunk_size: u32, // typically 16 for PCM
    pub subchunksize: u32,   // data chunk size (bytes of PCM)
}

fn parse_header(h: &[u8; WAV_HEADER_LEN]) -> Result<WavHeader> {
    if &h[0..4] != b"RIFF" { return Err(Error::from_reason("Bad RIFF")); }
    if &h[8..12] != b"WAVE" { return Err(Error::from_reason("Bad WAVE")); }
    if &h[12..16] != b"fmt " { return Err(Error::from_reason("Bad fmt")); }
    if &h[36..40] != b"data" { return Err(Error::from_reason("Bad data")); }

    Ok(WavHeader {
        chunksize: u32::from_le_bytes(h[4..8].try_into().unwrap()),
        fmt_chunk_size: u32::from_le_bytes(h[16..20].try_into().unwrap()),
        audio_format: u16::from_le_bytes(h[20..22].try_into().unwrap()),
        num_channels: u16::from_le_bytes(h[22..24].try_into().unwrap()),
        sample_rate: u32::from_le_bytes(h[24..28].try_into().unwrap()),
        byte_rate: u32::from_le_bytes(h[28..32].try_into().unwrap()),
        sample_alignment: u16::from_le_bytes(h[32..34].try_into().unwrap()),
        bit_depth: u16::from_le_bytes(h[34..36].try_into().unwrap()),
        subchunksize: u32::from_le_bytes(h[40..44].try_into().unwrap()),
    })
}

fn build_header(h: &WavHeader) -> [u8; WAV_HEADER_LEN] {
    let mut out = [0u8; WAV_HEADER_LEN];
    out[0..4].copy_from_slice(b"RIFF");
    out[4..8].copy_from_slice(&h.chunksize.to_le_bytes());
    out[8..12].copy_from_slice(b"WAVE");
    out[12..16].copy_from_slice(b"fmt ");
    out[16..20].copy_from_slice(&h.fmt_chunk_size.to_le_bytes());
    out[20..22].copy_from_slice(&h.audio_format.to_le_bytes());
    out[22..24].copy_from_slice(&h.num_channels.to_le_bytes());
    out[24..28].copy_from_slice(&h.sample_rate.to_le_bytes());
    out[28..32].copy_from_slice(&h.byte_rate.to_le_bytes());
    out[32..34].copy_from_slice(&h.sample_alignment.to_le_bytes());
    out[34..36].copy_from_slice(&h.bit_depth.to_le_bytes());
    out[36..40].copy_from_slice(b"data");
    out[40..44].copy_from_slice(&h.subchunksize.to_le_bytes());
    out
}

fn pcm_header(num_channels: u16, sample_rate: u32) -> WavHeader {
    let bit_depth: u16 = 16;
    let sample_alignment = num_channels * (bit_depth / 8);
    WavHeader {
        audio_format: WAVE_FORMAT_PCM,
        num_channels,
        sample_rate,
        byte_rate: sample_rate * sample_alignment as u32,
        sample_alignment,
        bit_depth,
        chunksize: 36, // will be fixed up as data grows
        fmt_chunk_size: 16,
        subchunksize: 0,
    }
}

// ---- JS-facing info() ----

#[napi(object)]
pub struct WavInfo {
    pub audioformat: u32,
    pub channelcount: u32,
    pub samplerate: u32,
    pub samplealignment: u32,
    pub byterate: u32,
    pub bitdepth: u32,
    pub chunksize: u32,
    pub fmtchunksize: u32,
    pub subchunksize: u32,
}

impl From<WavHeader> for WavInfo {
    fn from(h: WavHeader) -> Self {
        Self {
            audioformat: h.audio_format as u32,
            channelcount: h.num_channels as u32,
            samplerate: h.sample_rate,
            samplealignment: h.sample_alignment as u32,
            byterate: h.byte_rate,
            bitdepth: h.bit_depth as u32,
            chunksize: h.chunksize,
            fmtchunksize: h.fmt_chunk_size,
            subchunksize: h.subchunksize,
        }
    }
}

#[napi(namespace = "soundfile", js_name = "info")]
pub fn js_info(filename: String) -> Result<WavInfo> {
    let bytes = std::fs::read(&filename)
        .map_err(|e| Error::from_reason(format!("open {filename}: {e}")))?;
    if bytes.len() < WAV_HEADER_LEN {
        return Err(Error::from_reason("Bad wav header"));
    }
    let mut hdr = [0u8; WAV_HEADER_LEN];
    hdr.copy_from_slice(&bytes[..WAV_HEADER_LEN]);
    Ok(parse_header(&hdr)?.into())
}

// ---- Reader ----

pub struct WavReader {
    file: File,
    header: WavHeader,
    data_start: u64,
    data_end: u64,
    #[allow(dead_code)]
    url: PathBuf,
}

impl WavReader {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let url = path.as_ref().to_path_buf();
        let mut file = File::open(&url)
            .await
            .map_err(|e| Error::from_reason(format!("open {url:?}: {e}")))?;
        let mut buf = [0u8; WAV_HEADER_LEN];
        file.read_exact(&mut buf)
            .await
            .map_err(|e| Error::from_reason(format!("read header: {e}")))?;
        let header = parse_header(&buf)?;
        if header.audio_format != WAVE_FORMAT_PCM || header.bit_depth != 16 {
            return Err(Error::from_reason(format!(
                "only WAVE_FORMAT_PCM / 16-bit supported in WavReader, got format=0x{:X} depth={}",
                header.audio_format, header.bit_depth
            )));
        }
        let data_start = WAV_HEADER_LEN as u64;
        let data_end = data_start + header.subchunksize as u64;
        Ok(Self { file, header, data_start, data_end, url })
    }

    pub fn header(&self) -> &WavHeader { &self.header }
    #[allow(dead_code)]
    pub fn url(&self) -> &Path { &self.url }

    /// Read up to `samples` int16 samples starting from the current position.
    /// Returns a vec that may be shorter than `samples` if EOF is hit.
    pub async fn read_samples(&mut self, samples: usize) -> Result<Vec<i16>> {
        let max_bytes = samples.checked_mul(2).ok_or_else(|| Error::from_reason("overflow"))?;
        let pos = self.file.stream_position().await.map_err(io_err)?;
        let remaining = self.data_end.saturating_sub(pos) as usize;
        let to_read = max_bytes.min(remaining);
        let mut buf = vec![0u8; to_read];
        if to_read > 0 {
            self.file.read_exact(&mut buf).await.map_err(io_err)?;
        }
        let mut out = Vec::with_capacity(to_read / 2);
        for chunk in buf.chunks_exact(2) {
            out.push(i16::from_le_bytes([chunk[0], chunk[1]]));
        }
        Ok(out)
    }

    pub async fn seek_ms(&mut self, ms: u64) -> Result<()> {
        let bytes_per_ms = (self.header.byte_rate as u64) / 1000;
        let mut offset = bytes_per_ms * ms;
        // Align to sample boundary.
        let align = self.header.sample_alignment as u64;
        if align > 0 { offset -= offset % align; }
        let abs = self.data_start + offset.min(self.data_end - self.data_start);
        self.file.seek(SeekFrom::Start(abs)).await.map_err(io_err)?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn position_ms(&mut self) -> Result<u64> {
        let pos = self.file.stream_position().await.map_err(io_err)?;
        let data_pos = pos.saturating_sub(self.data_start);
        let bytes_per_ms = (self.header.byte_rate as u64) / 1000;
        if bytes_per_ms == 0 { return Ok(0); }
        Ok(data_pos / bytes_per_ms)
    }

    #[allow(dead_code)]
    pub fn duration_ms(&self) -> u64 {
        let bytes_per_ms = (self.header.byte_rate as u64) / 1000;
        if bytes_per_ms == 0 { return 0; }
        (self.header.subchunksize as u64) / bytes_per_ms
    }

    #[allow(dead_code)]
    pub async fn complete(&mut self) -> Result<bool> {
        let pos = self.file.stream_position().await.map_err(io_err)?;
        Ok(pos >= self.data_end)
    }
}

// ---- Writer ----
//
// Drop rewrites the header with the final byte count. This is the crucial
// AIO-replacement behavior: no matter how the writer dies, the header is
// finalized by a sync write in Drop, not via manual aio_return tracking.

pub struct WavWriter {
    // Option so Drop can take the File out and flush synchronously.
    file: Option<std::fs::File>,
    header: WavHeader,
    bytes_written: u64,
    #[allow(dead_code)]
    url: PathBuf,
    closed: bool,
}

impl WavWriter {
    pub async fn create(
        path: impl AsRef<Path>,
        num_channels: u16,
        sample_rate: u32,
    ) -> Result<Self> {
        let url = path.as_ref().to_path_buf();
        let header = pcm_header(num_channels, sample_rate);
        // Write initial header via tokio (async create), then hand a std::fs::File
        // to the struct so Drop can finalize without async runtime.
        let mut afile = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&url)
            .await
            .map_err(|e| Error::from_reason(format!("create {url:?}: {e}")))?;
        afile.write_all(&build_header(&header)).await.map_err(io_err)?;
        afile.flush().await.map_err(io_err)?;
        let file = afile.into_std().await;
        Ok(Self {
            file: Some(file),
            header,
            bytes_written: 0,
            url,
            closed: false,
        })
    }

    #[allow(dead_code)]
    pub fn header(&self) -> &WavHeader { &self.header }
    #[allow(dead_code)]
    pub fn url(&self) -> &Path { &self.url }
    #[allow(dead_code)]
    pub fn bytes_written(&self) -> u64 { self.bytes_written }

    pub async fn write_samples(&mut self, samples: &[i16]) -> Result<()> {
        if self.closed { return Err(Error::from_reason("writer closed")); }
        let mut buf = Vec::with_capacity(samples.len() * 2);
        for s in samples { buf.extend_from_slice(&s.to_le_bytes()); }
        let _file = self.file.as_mut().ok_or_else(|| Error::from_reason("no file"))?;
        // std::fs::File writes are synchronous — do them via spawn_blocking so
        // we don't stall the tokio worker during long writes on slow disks.
        let to_write = buf;
        let mut taken = self.file.take().unwrap();
        let (file, written) = tokio::task::spawn_blocking(move || {
            use std::io::Write;
            match taken.write_all(&to_write) {
                Ok(()) => Ok::<_, std::io::Error>((taken, to_write.len() as u64)),
                Err(e) => Err(e),
            }
        })
        .await
        .map_err(|e| Error::from_reason(format!("join: {e}")))?
        .map_err(io_err)?;
        let _ = file; // keep name stable
        self.file = Some(file);
        self.bytes_written += written;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn write_samples_sync(&mut self, samples: &[i16]) -> Result<()> {
        if self.closed { return Err(Error::from_reason("writer closed")); }
        use std::io::Write;
        let file = self.file.as_mut().ok_or_else(|| Error::from_reason("no file"))?;
        for s in samples { file.write_all(&s.to_le_bytes()).map_err(io_err)?; }
        self.bytes_written += (samples.len() as u64) * 2;
        Ok(())
    }

    /// Finalize the header explicitly. Idempotent; Drop will call this if
    /// not already done.
    pub fn close(&mut self) -> Result<()> {
        if self.closed { return Ok(()); }
        self.closed = true;
        let mut file = match self.file.take() {
            Some(f) => f,
            None => return Ok(()),
        };
        use std::io::{Seek, Write};
        let data_bytes = self.bytes_written as u32;
        self.header.subchunksize = data_bytes;
        self.header.chunksize = data_bytes + 36;
        let hdr = build_header(&self.header);
        file.seek(std::io::SeekFrom::Start(0)).map_err(io_err)?;
        file.write_all(&hdr).map_err(io_err)?;
        file.flush().map_err(io_err)?;
        Ok(())
    }
}

impl Drop for WavWriter {
    fn drop(&mut self) {
        // RAII finalize — no-op if close() was already called.
        let _ = self.close();
    }
}

fn io_err(e: std::io::Error) -> Error { Error::from_reason(e.to_string()) }

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpfile(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(name);
        let _ = std::fs::remove_file(&p);
        p
    }

    #[tokio::test]
    async fn writer_then_reader_roundtrip() {
        let path = tmpfile("projectrtp_sf_rt.wav");
        let samples: Vec<i16> = (0..8000).map(|i| (i as i16).wrapping_mul(3)).collect();
        {
            let mut w = WavWriter::create(&path, 1, 8000).await.unwrap();
            w.write_samples(&samples).await.unwrap();
            w.close().unwrap();
        }
        let mut r = WavReader::open(&path).await.unwrap();
        assert_eq!(r.header().sample_rate, 8000);
        assert_eq!(r.header().num_channels, 1);
        assert_eq!(r.duration_ms(), 1000); // 8000 samples @ 8kHz = 1 s
        let got = r.read_samples(samples.len()).await.unwrap();
        assert_eq!(got, samples);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn raii_drop_finalizes_header_even_without_close() {
        let path = tmpfile("projectrtp_sf_raii.wav");
        {
            let mut w = WavWriter::create(&path, 1, 8000).await.unwrap();
            let samples: Vec<i16> = (0..1600).map(|i| i as i16).collect();
            w.write_samples(&samples).await.unwrap();
            // intentionally: no explicit close(), Drop must finalize.
        }
        let info = js_info(path.to_string_lossy().into_owned()).unwrap();
        assert_eq!(info.subchunksize, 3200); // 1600 samples × 2 bytes
        assert_eq!(info.chunksize, 3200 + 36);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn seek_ms_round_trip() {
        let path = tmpfile("projectrtp_sf_seek.wav");
        let samples: Vec<i16> = (0..16000).map(|i| i as i16).collect(); // 2 s
        {
            let mut w = WavWriter::create(&path, 1, 8000).await.unwrap();
            w.write_samples(&samples).await.unwrap();
            w.close().unwrap();
        }
        let mut r = WavReader::open(&path).await.unwrap();
        r.seek_ms(1000).await.unwrap();
        assert_eq!(r.position_ms().await.unwrap(), 1000);
        let rest = r.read_samples(8000).await.unwrap();
        assert_eq!(rest.len(), 8000);
        assert_eq!(rest[0], samples[8000]);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn info_matches_header() {
        let path = tmpfile("projectrtp_sf_info.wav");
        {
            let mut w = WavWriter::create(&path, 2, 16000).await.unwrap();
            let samples = vec![0i16; 800 * 2]; // 50 ms stereo
            w.write_samples(&samples).await.unwrap();
            w.close().unwrap();
        }
        let info = js_info(path.to_string_lossy().into_owned()).unwrap();
        assert_eq!(info.audioformat, WAVE_FORMAT_PCM as u32);
        assert_eq!(info.channelcount, 2);
        assert_eq!(info.samplerate, 16000);
        assert_eq!(info.bitdepth, 16);
        assert_eq!(info.subchunksize, 800 * 2 * 2);
        let _ = std::fs::remove_file(&path);
    }
}
