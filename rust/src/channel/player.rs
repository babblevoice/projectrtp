// Player (soundsoup) — port of projectrtpsoundsoup.{cpp,h}.
//
// A soundsoup is a playlist: a sequence of files, each with optional
// start/stop trim and per-file loop count, plus an overall loop count for the
// whole list. Used by `channel.play(...)`.
//
// JSON→struct conversion lives at the JS boundary (channel/mod.rs); this
// module takes already-parsed `SoundSoupSpec` values so playback concerns
// aren't tangled with parsing concerns.

use std::path::PathBuf;

use super::super::soundfile::WavReader;

#[derive(Debug, Clone)]
pub struct SoundSoupFileSpec {
    /// Pick one encoding to prefer; unused for now — WavReader handles PCM-16.
    pub path: PathBuf,
    pub start_ms: Option<u64>,
    pub stop_ms: Option<u64>,
    pub max_loops: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct SoundSoupSpec {
    pub files: Vec<SoundSoupFileSpec>,
    /// `None` = play once. `Some(0)` = infinite. `Some(n)` = n overall loops.
    pub overall_loops: Option<u32>,
    pub interrupt: bool,
}

pub struct Player {
    spec: SoundSoupSpec,
    index: usize,
    reader: Option<WavReader>,
    file_loop: u32,
    overall_loop: u32,
    finished: bool,
}

/// One chunk of decoded audio (interleaved if multi-channel).
pub struct PlayFrame {
    pub samples: Vec<i16>,
    pub end_of_file: bool,
    pub end_of_soup: bool,
}

impl Player {
    pub fn new(spec: SoundSoupSpec) -> Self {
        Self {
            spec,
            index: 0,
            reader: None,
            file_loop: 0,
            overall_loop: 0,
            finished: false,
        }
    }

    pub fn is_finished(&self) -> bool { self.finished }
    pub fn interrupts(&self) -> bool { self.spec.interrupt }

    /// Read up to `samples_wanted` 16-bit samples, advancing through files
    /// and loops as needed. Returns fewer samples only when the soup finishes.
    pub async fn read(&mut self, samples_wanted: usize) -> PlayFrame {
        let mut out = Vec::with_capacity(samples_wanted);
        let mut end_of_file = false;
        let mut end_of_soup = false;

        while out.len() < samples_wanted && !self.finished {
            if self.reader.is_none() {
                if !self.open_current().await {
                    end_of_soup = true;
                    break;
                }
            }

            let need = samples_wanted - out.len();
            let read = match self.reader.as_mut().unwrap().read_samples(need).await {
                Ok(v) => v,
                Err(_) => Vec::new(),
            };

            if read.is_empty() {
                // EOF for this file.
                end_of_file = true;
                self.reader = None;
                self.advance().await;
                continue;
            }
            out.extend(read);
        }

        PlayFrame { samples: out, end_of_file, end_of_soup: end_of_soup || self.finished }
    }

    async fn open_current(&mut self) -> bool {
        let spec = match self.spec.files.get(self.index) {
            Some(s) => s.clone(),
            None => { self.finished = true; return false; }
        };
        let mut reader = match WavReader::open(&spec.path).await {
            Ok(r) => r,
            Err(_) => { self.finished = true; return false; }
        };
        if let Some(start) = spec.start_ms {
            let _ = reader.seek_ms(start).await;
        }
        self.reader = Some(reader);
        true
    }

    async fn advance(&mut self) {
        let cur = match self.spec.files.get(self.index).cloned() {
            Some(s) => s,
            None => { self.finished = true; return; }
        };
        self.file_loop += 1;

        let file_done = match cur.max_loops {
            Some(n) => self.file_loop >= n.max(1),
            None => true, // no per-file loop; move on
        };

        if !file_done {
            // Re-open same file for next iteration.
            self.reader = None;
            return;
        }

        self.file_loop = 0;
        self.index += 1;

        if self.index >= self.spec.files.len() {
            self.overall_loop += 1;
            let done = match self.spec.overall_loops {
                None => true,           // play-once
                Some(0) => false,        // infinite
                Some(n) => self.overall_loop >= n,
            };
            if done {
                self.finished = true;
            } else {
                self.index = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soundfile::WavWriter;

    async fn make_wav(name: &str, samples: &[i16]) -> PathBuf {
        let p = std::env::temp_dir().join(name);
        let _ = std::fs::remove_file(&p);
        let mut w = WavWriter::create(&p, 1, 8000).await.unwrap();
        w.write_samples(samples).await.unwrap();
        w.close().unwrap();
        p
    }

    #[tokio::test]
    async fn plays_single_file_once() {
        let p = make_wav("player_single.wav", &vec![1i16; 800]).await;
        let spec = SoundSoupSpec {
            files: vec![SoundSoupFileSpec { path: p.clone(), start_ms: None, stop_ms: None, max_loops: None }],
            overall_loops: None,
            interrupt: false,
        };
        let mut pl = Player::new(spec);
        let frame = pl.read(800).await;
        assert_eq!(frame.samples.len(), 800);
        assert!(frame.samples.iter().all(|&s| s == 1));
        // Next read should find the soup finished.
        let frame = pl.read(800).await;
        assert!(frame.end_of_soup);
        let _ = std::fs::remove_file(&p);
    }

    #[tokio::test]
    async fn loops_overall_count() {
        let p = make_wav("player_loop.wav", &vec![7i16; 100]).await;
        let spec = SoundSoupSpec {
            files: vec![SoundSoupFileSpec { path: p.clone(), start_ms: None, stop_ms: None, max_loops: None }],
            overall_loops: Some(3),
            interrupt: false,
        };
        let mut pl = Player::new(spec);

        let mut total = 0;
        loop {
            let frame = pl.read(50).await;
            total += frame.samples.len();
            if frame.end_of_soup && frame.samples.is_empty() { break; }
        }
        assert_eq!(total, 300, "3 loops × 100 samples");
        let _ = std::fs::remove_file(&p);
    }

    #[tokio::test]
    async fn advances_through_multiple_files() {
        let p1 = make_wav("player_multi1.wav", &vec![11i16; 100]).await;
        let p2 = make_wav("player_multi2.wav", &vec![22i16; 100]).await;
        let spec = SoundSoupSpec {
            files: vec![
                SoundSoupFileSpec { path: p1.clone(), start_ms: None, stop_ms: None, max_loops: None },
                SoundSoupFileSpec { path: p2.clone(), start_ms: None, stop_ms: None, max_loops: None },
            ],
            overall_loops: None,
            interrupt: false,
        };
        let mut pl = Player::new(spec);
        let frame = pl.read(200).await;
        assert_eq!(frame.samples.len(), 200);
        // First 100 = 11, next 100 = 22.
        assert!(frame.samples[..100].iter().all(|&s| s == 11));
        assert!(frame.samples[100..].iter().all(|&s| s == 22));
        let _ = std::fs::remove_file(&p1);
        let _ = std::fs::remove_file(&p2);
    }
}
