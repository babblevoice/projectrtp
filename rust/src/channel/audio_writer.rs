// AudioWriter — the Rust side of `channel.createWriteStream(opts)`.
//
// Symmetric counterpart to AudioReader. Where the reader forwards
// decoded inbound frames OUT to JS, the writer accepts linear PCM IN
// from JS and feeds it onto the outbound path as if it were the
// player's output.
//
// Unlike the reader, the writer owns the mpsc `Receiver` (not the
// Sender — the facade hands the Sender out to the JS wrapper so each
// `_write` call drops bytes straight onto it). Every tick the writer
// drains the mpsc into its internal sample buffer, then yields one
// 20 ms frame back to the caller. Underrun → None → tick emits silence.
//
// Drop / underrun / end policy (matches AudioReader for symmetry):
//   overflow  : JS-side bounded mpsc `try_send` fails; napi returns
//               false to `_write` which delays + retries. We bump a
//               `drops` counter for visibility but never block the tick.
//   underrun  : `next_frame_8k` returns None; upstream emits silence
//               for that 20 ms slot — same as any idle channel.
//   end       : JS drops its sender → `Disconnected` on the next drain
//               → `ended` flips true → we flush remaining buffered
//               samples then the orchestrator pulls the writer out of
//               Subsystems and the channel reverts to silence. Matches
//               how `play` reaches its natural end of file.

use tokio::sync::mpsc;

/// Frames = 50 × 20 ms = 1 s. Same as the reader — gives a GC pause /
/// Node event-loop stall comfortable headroom without unbounded growth.
pub const WRITER_QUEUE_DEPTH: usize = 50;

/// 20 ms at 8 kHz.
const FRAME_SAMPLES: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterFormat {
    /// Linear PCM-16 little-endian — the only format in v1.
    L16,
}

#[derive(Debug, Clone)]
// v1 locks these to (L16, 8000, 1) so they're not read yet, but the
// parser + struct are shaped the way the reader's config is shaped so
// the format matrix can be opened up without reshuffling the API.
#[allow(dead_code)]
pub struct WriterConfig {
    pub format: WriterFormat,
    /// Only 8000 Hz in v1 — matches the narrowband pipeline end-to-end.
    pub samplerate: u32,
    /// 1 (mono) in v1. Stereo de-interleaving lands when there's a
    /// concrete consumer that needs it.
    pub num_channels: u16,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            format: WriterFormat::L16,
            samplerate: 8000,
            num_channels: 1,
        }
    }
}

/// A registered writer. The facade holds the matching `Sender` (for JS
/// to push bytes onto); the actor holds this struct (owning the
/// `Receiver`). Dropping the Sender on the JS side signals end-of-stream.
pub struct AudioWriter {
    pub id: u64,
    cfg: WriterConfig,
    receiver: mpsc::Receiver<Vec<u8>>,
    /// Samples pulled off the mpsc but not yet emitted — `next_frame_8k`
    /// only yields when we have a full 160-sample frame buffered.
    buffered: Vec<i16>,
    /// Set once the Sender drops (JS called `.end()` or Writable was
    /// destroyed). Triggers tear-down once the buffered tail is
    /// flushed.
    ended: bool,
    drops: u64,
}

impl AudioWriter {
    pub fn new(id: u64, cfg: WriterConfig, receiver: mpsc::Receiver<Vec<u8>>) -> Self {
        Self { id, cfg, receiver, buffered: Vec::with_capacity(FRAME_SAMPLES * 2), ended: false, drops: 0 }
    }

    pub fn id(&self) -> u64 { self.id }
    #[allow(dead_code)]
    pub fn config(&self) -> &WriterConfig { &self.cfg }
    #[allow(dead_code)]
    pub fn drops(&self) -> u64 { self.drops }

    /// True when the JS side has finished AND we've flushed every
    /// sample it gave us. Used by the tick orchestrator to pull the
    /// writer out of Subsystems and let the channel revert to silence.
    pub fn is_drained_and_ended(&self) -> bool {
        self.ended && self.buffered.len() < FRAME_SAMPLES
    }

    /// Pull one 20 ms / 160-sample frame. Drains as much as possible
    /// from the mpsc first so the buffer is always at its "widest" point
    /// when the tick reads — minimises consecutive underrun frames when
    /// the consumer's chunks don't align to 20 ms.
    pub fn next_frame_8k(&mut self) -> Option<Vec<i16>> {
        self.drain_receiver();
        if self.buffered.len() >= FRAME_SAMPLES {
            Some(self.buffered.drain(..FRAME_SAMPLES).collect())
        } else if self.ended && !self.buffered.is_empty() {
            // End-of-stream tail: pad with silence so we deliver the
            // last real samples rather than losing a partial frame.
            let mut frame = std::mem::take(&mut self.buffered);
            frame.resize(FRAME_SAMPLES, 0);
            Some(frame)
        } else {
            None
        }
    }

    fn drain_receiver(&mut self) {
        loop {
            match self.receiver.try_recv() {
                Ok(bytes) => {
                    // L16 LE → i16. Any trailing odd byte is dropped —
                    // consumers should always write whole samples.
                    for chunk in bytes.chunks_exact(2) {
                        self.buffered.push(i16::from_le_bytes([chunk[0], chunk[1]]));
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    self.ended = true;
                    break;
                }
            }
        }
    }
}

/// Small builder so callers don't import the mpsc depth constant.
pub fn make_channel() -> (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>) {
    mpsc::channel(WRITER_QUEUE_DEPTH)
}

/// Monotonic id generator, shared across channels. `u64` is overkill
/// but matches the reader's shape.
static NEXT_WRITER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
pub fn next_writer_id() -> u64 {
    NEXT_WRITER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn underrun_returns_none() {
        let (_tx, rx) = make_channel();
        let mut w = AudioWriter::new(1, WriterConfig::default(), rx);
        assert!(w.next_frame_8k().is_none());
    }

    #[test]
    fn full_frame_emitted() {
        let (tx, rx) = make_channel();
        let mut w = AudioWriter::new(1, WriterConfig::default(), rx);
        // 320 bytes = 160 i16 samples = 20 ms @ 8 kHz.
        let mut bytes = Vec::with_capacity(320);
        for i in 0..160i16 { bytes.extend_from_slice(&i.to_le_bytes()); }
        tx.try_send(bytes).unwrap();
        let frame = w.next_frame_8k().unwrap();
        assert_eq!(frame.len(), 160);
        assert_eq!(frame[0], 0);
        assert_eq!(frame[159], 159);
    }

    #[test]
    fn chunks_assemble_into_frames() {
        // Writer must cope with arbitrary chunk sizes from JS — the
        // consumer might `.write()` 50 bytes at a time or 1000, we need
        // to buffer and slice at 20 ms boundaries.
        let (tx, rx) = make_channel();
        let mut w = AudioWriter::new(1, WriterConfig::default(), rx);
        // 300 + 40 bytes = 340 bytes = 170 samples → one full frame,
        // with 10 samples left behind for the next tick.
        tx.try_send(vec![1; 300]).unwrap();
        tx.try_send(vec![1; 40]).unwrap();
        let frame = w.next_frame_8k().unwrap();
        assert_eq!(frame.len(), 160);
        assert!(w.next_frame_8k().is_none()); // only 10 leftover
    }

    #[test]
    fn end_of_stream_flushes_partial_frame_with_silence() {
        let (tx, rx) = make_channel();
        let mut w = AudioWriter::new(1, WriterConfig::default(), rx);
        // 50 samples then drop the sender (simulates JS .end()).
        let mut bytes = Vec::with_capacity(100);
        for i in 0..50i16 { bytes.extend_from_slice(&i.to_le_bytes()); }
        tx.try_send(bytes).unwrap();
        drop(tx);

        let frame = w.next_frame_8k().expect("partial frame padded with silence");
        assert_eq!(frame.len(), 160);
        assert_eq!(frame[0], 0);
        assert_eq!(frame[49], 49);
        // Remainder should be zero-padded.
        assert!(frame[50..].iter().all(|&s| s == 0));
        // Drained — next call returns None and is_drained_and_ended is true.
        assert!(w.next_frame_8k().is_none());
        assert!(w.is_drained_and_ended());
    }
}
