// AudioReader — the Rust side of `channel.createReadStream(opts)`.
//
// Structurally parallels `Recorder`: per-channel, lives in
// `Subsystems::readers`, fed once per tick at the same point the recorder
// is fed. Instead of writing samples to a WAV on disk it pushes formatted
// frames into a bounded mpsc; a forwarder task (owned by the facade) reads
// the mpsc and hands each frame to a JS ThreadsafeFunction, which in turn
// drives a Node `Readable`.
//
// Drop policy (non-blocking at every layer — the 20 ms tick must never stall):
//   1. tick/mix → AudioReader::feed → mpsc `try_send`. Bounded at
//      FRAME_QUEUE_DEPTH (~1 s). On full or closed channel we bump `drops`
//      and return — no block, no await.
//   2. Forwarder → TSFN: uses `NonBlocking` call mode; napi drops if its
//      own queue is full.
//   3. JS Readable.push(): if the consumer is slow, push returns false and
//      the JS shim drops the frame (handled in index.js, not here).
//
// Format / sample-rate matrix for v1:
//   format=L16, samplerate=8000  → narrowband cache (always available)
//   format=L16, samplerate=16000 → wideband cache (decoded from G.722, or
//                                  upsampled from narrowband otherwise)
//   format=PCMA/PCMU/G722/ILBC   → raw wire bytes for that codec pulled
//                                  through `require_wire_as`. Useful for
//                                  forwarding without re-encoding.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::codec::CodecBundle;

/// How many 20 ms frames we buffer Rust-side before dropping.
/// 50 frames = 1 s of audio, comfortable for GC pauses on the JS side.
pub const FRAME_QUEUE_DEPTH: usize = 50;

/// Which direction of the call this reader observes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderDirection {
    /// Inbound audio — what the far peer sent us. This is what you want
    /// for STT on the caller.
    In,
    /// Outbound audio — what we sent to the peer (player/echo/mix output).
    Out,
    /// Both sides, interleaved stereo (L=in, R=out). `num_channels` must
    /// be 2.
    Both,
}

/// Wire / sample format delivered to JS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderFormat {
    /// Linear PCM-16 little-endian, sample-rate set by `samplerate`.
    L16,
    /// Raw A-law wire bytes (1 byte per 8 kHz sample).
    Pcma,
    /// Raw µ-law wire bytes.
    Pcmu,
    /// Raw G.722 wire bytes (1 byte per pair of 16 kHz samples).
    #[allow(dead_code)]
    G722,
    /// Raw iLBC wire bytes.
    #[allow(dead_code)]
    Ilbc,
}

#[derive(Debug, Clone)]
pub struct ReaderConfig {
    pub direction: ReaderDirection,
    pub format: ReaderFormat,
    /// Only meaningful for L16. 8000 or 16000. Ignored for wire-byte formats.
    pub samplerate: u32,
    /// 1 (mono) or 2 (interleaved stereo). Stereo requires direction=Both;
    /// for In or Out + num_channels=2 we duplicate the mono signal onto
    /// both output channels (mirrors the recorder's convenience).
    pub num_channels: u16,
}

impl Default for ReaderConfig {
    fn default() -> Self {
        Self {
            direction: ReaderDirection::In,
            format: ReaderFormat::L16,
            samplerate: 8000,
            num_channels: 1,
        }
    }
}

/// A reader registered on a channel. The facade owns the matching
/// forwarder task + JS TSFN; dropping this handle closes the mpsc which
/// signals the forwarder to shut down cleanly.
pub struct AudioReader {
    /// Stable id so JS can reference this reader for destroy.
    pub id: u64,
    cfg: ReaderConfig,
    sender: mpsc::Sender<Vec<u8>>,
    drops: u64,
}

impl AudioReader {
    pub fn new(id: u64, cfg: ReaderConfig, sender: mpsc::Sender<Vec<u8>>) -> Self {
        Self { id, cfg, sender, drops: 0 }
    }

    pub fn id(&self) -> u64 { self.id }
    /// Surfaced for diagnostics / future dynamic reconfiguration
    /// (e.g. if a reader's consumer wants to re-query its format
    /// after the fact). Not called from the hot path today.
    #[allow(dead_code)]
    pub fn config(&self) -> &ReaderConfig { &self.cfg }
    /// Exposes the per-reader drops counter to an eventual `stats()`
    /// summary. Not called yet — the counter is bumped internally.
    #[allow(dead_code)]
    pub fn drops(&self) -> u64 { self.drops }

    /// True once the JS side has dropped its end (forwarder task exited).
    /// Lets the tick garbage-collect finished readers.
    pub fn is_closed(&self) -> bool { self.sender.is_closed() }

    /// Feed one 20 ms frame. Called from the recorder's feed point in the
    /// tick (same cache state, same timing). Samples are already narrowband
    /// 8 kHz linear — wideband and wire-byte formats are pulled directly
    /// from `codecx`.
    ///
    /// Never blocks. On full queue or closed consumer the frame is silently
    /// dropped (bumping `drops`). Returns `true` if the frame was queued.
    pub fn feed(
        &mut self,
        codecx: &mut CodecBundle,
        in_samples_8k: Option<&[i16]>,
        out_samples_8k: Option<&[i16]>,
    ) -> bool {
        let Some(bytes) = self.build_frame(codecx, in_samples_8k, out_samples_8k) else {
            return false;
        };
        match self.sender.try_send(bytes) {
            Ok(()) => true,
            Err(_) => { self.drops = self.drops.saturating_add(1); false }
        }
    }

    fn build_frame(
        &self,
        codecx: &mut CodecBundle,
        in_samples_8k: Option<&[i16]>,
        out_samples_8k: Option<&[i16]>,
    ) -> Option<Vec<u8>> {
        match self.cfg.format {
            ReaderFormat::L16 => self.build_l16(codecx, in_samples_8k, out_samples_8k),
            ReaderFormat::Pcma => codecx.require_wire_as(8).map(|b| b.to_vec()),
            ReaderFormat::Pcmu => codecx.require_wire_as(0).map(|b| b.to_vec()),
            ReaderFormat::G722 => codecx.require_wire_as(9).map(|b| b.to_vec()),
            // iLBC PT is channel-dependent; wire-byte iLBC readers aren't
            // exposed to JS yet, so this branch is not reached in v1.
            ReaderFormat::Ilbc => None,
        }
    }

    fn build_l16(
        &self,
        codecx: &mut CodecBundle,
        in_samples_8k: Option<&[i16]>,
        out_samples_8k: Option<&[i16]>,
    ) -> Option<Vec<u8>> {
        // Resolve the two sides at the requested sample rate.
        let (in_samples, out_samples): (Option<Vec<i16>>, Option<Vec<i16>>) = match self.cfg.samplerate {
            16000 => {
                // Wideband is cached on codecx; upsamples from narrowband if the
                // wire is a narrowband codec. Only the "in" side lives on codecx
                // — the "out" side can't sensibly become wideband in v1 (no
                // upsample cache for player frames). Treat out=None at 16k.
                let wb = codecx.require_wideband_16k().map(|s| s.to_vec());
                (wb, None)
            }
            _ => (
                in_samples_8k.map(|s| s.to_vec()),
                out_samples_8k.map(|s| s.to_vec()),
            ),
        };

        let in_ref: &[i16] = in_samples.as_deref().unwrap_or(&[]);
        let out_ref: &[i16] = out_samples.as_deref().unwrap_or(&[]);

        // Nothing to emit (neither side provided samples at this rate).
        if in_ref.is_empty() && out_ref.is_empty() { return None; }

        let n_out = in_ref.len().max(out_ref.len());
        let ch = self.cfg.num_channels as usize;

        let mut bytes = Vec::with_capacity(n_out * 2 * ch);
        match (self.cfg.direction, ch) {
            (ReaderDirection::In, 1) => push_mono(&mut bytes, in_ref, n_out),
            (ReaderDirection::Out, 1) => push_mono(&mut bytes, out_ref, n_out),
            (ReaderDirection::Both, 2) => push_stereo_interleaved(&mut bytes, in_ref, out_ref, n_out),
            // Mono direction on a stereo reader — duplicate across L/R.
            (ReaderDirection::In, 2) => push_stereo_interleaved(&mut bytes, in_ref, in_ref, n_out),
            (ReaderDirection::Out, 2) => push_stereo_interleaved(&mut bytes, out_ref, out_ref, n_out),
            // direction=Both on a mono reader — sum and return, matches recorder semantics.
            (ReaderDirection::Both, 1) => {
                for i in 0..n_out {
                    let a = in_ref.get(i).copied().unwrap_or(0) as i32;
                    let b = out_ref.get(i).copied().unwrap_or(0) as i32;
                    let s = (a + b).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                    bytes.extend_from_slice(&s.to_le_bytes());
                }
            }
            // Unsupported channel count — drop.
            _ => return None,
        }
        Some(bytes)
    }
}

fn push_mono(bytes: &mut Vec<u8>, samples: &[i16], n: usize) {
    for i in 0..n {
        let s = samples.get(i).copied().unwrap_or(0);
        bytes.extend_from_slice(&s.to_le_bytes());
    }
}

fn push_stereo_interleaved(bytes: &mut Vec<u8>, left: &[i16], right: &[i16], n: usize) {
    for i in 0..n {
        let l = left.get(i).copied().unwrap_or(0);
        let r = right.get(i).copied().unwrap_or(0);
        bytes.extend_from_slice(&l.to_le_bytes());
        bytes.extend_from_slice(&r.to_le_bytes());
    }
}

// ---- forwarder plumbing --------------------------------------------------
//
// The facade creates (mpsc_tx, mpsc_rx) as a pair, hands mpsc_tx to the
// actor (wrapped in an AudioReader), and spawns a forwarder task holding
// mpsc_rx plus the JS TSFN. This module owns only the Rust types — the
// forwarder is started from facade.rs where the napi callback lives.

/// Small builder so callers don't have to know the mpsc depth constant.
pub fn make_channel() -> (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>) {
    mpsc::channel(FRAME_QUEUE_DEPTH)
}

// Monotonic id generator for readers (shared across all channels; u64 is
// plenty for any realistic uptime). Visible to the facade so it can stamp
// ids onto `AudioReader`s before they're handed to the actor.
static NEXT_READER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
pub fn next_reader_id() -> u64 {
    NEXT_READER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Handle to a reader kept on the facade side, used to tear down the
/// forwarder and unref the TSFN when the JS Readable is destroyed.
/// (Placeholder for now — forwarder task detaches on mpsc close, so
/// dropping the sender from the actor side is sufficient cleanup.)
#[allow(dead_code)]
pub struct ForwarderHandle {
    pub id: u64,
    pub cancel: Arc<tokio::sync::Notify>,
}
