// Recorder — port of projectrtpchannelrecorder.{cpp,h}.
//
// Wraps soundfile::WavWriter with the channel-level policy:
//   - max duration → auto-finish
//   - power-threshold gating (barge-in, start-above / finish-below / max)
//   - pause / request-finish flags
//
// The actual WAV I/O is delegated to WavWriter (which already has Drop-based
// header finalization — see Task #5).

use std::path::PathBuf;

use crate::firfilter::MaFilter;
use crate::soundfile::WavWriter;

#[derive(Debug, Clone)]
pub struct RecorderConfig {
    pub file: PathBuf,
    pub num_channels: u16,
    pub sample_rate: u32,
    /// Hard stop once this many ms have been recorded.
    pub max_duration_ms: Option<u64>,
    /// Power thresholds — all optional, all in the same units as MaFilter.get().
    pub start_above_power: Option<i32>,
    pub finish_below_power: Option<i32>,
    pub max_since_start_power: Option<i32>,
    pub min_duration_ms: Option<u64>,
    pub power_averaging_packets: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderState {
    /// Waiting for start-above-power (or always-active if not configured).
    Pending,
    Active,
    Paused,
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    Completed,
    MaxDurationReached,
    BelowPowerThreshold,
    ChannelClosed,
    Requested,
}

pub struct Recorder {
    cfg: RecorderConfig,
    writer: WavWriter,
    state: RecorderState,
    power_ma: MaFilter,
    samples_written: u64,
    samples_since_active: u64,
    finish_reason: Option<FinishReason>,
}

impl Recorder {
    pub async fn open(cfg: RecorderConfig) -> std::io::Result<Self> {
        let writer = WavWriter::create(&cfg.file, cfg.num_channels, cfg.sample_rate)
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let mut power_ma = MaFilter::new();
        if let Some(n) = cfg.power_averaging_packets {
            power_ma.reset(n as usize);
        }
        let state = if cfg.start_above_power.is_some() {
            RecorderState::Pending
        } else {
            RecorderState::Active
        };
        Ok(Self {
            cfg,
            writer,
            state,
            power_ma,
            samples_written: 0,
            samples_since_active: 0,
            finish_reason: None,
        })
    }

    pub fn state(&self) -> RecorderState { self.state }
    pub fn is_finished(&self) -> bool { self.state == RecorderState::Finished }
    pub fn finish_reason(&self) -> Option<&FinishReason> { self.finish_reason.as_ref() }
    pub fn file(&self) -> &std::path::Path { &self.cfg.file }

    pub fn pause(&mut self) {
        if matches!(self.state, RecorderState::Active | RecorderState::Pending) {
            self.state = RecorderState::Paused;
        }
    }

    pub fn resume(&mut self) {
        if self.state == RecorderState::Paused {
            self.state = RecorderState::Active;
        }
    }

    pub fn request_finish(&mut self, reason: FinishReason) {
        if self.state != RecorderState::Finished {
            self.state = RecorderState::Finished;
            self.finish_reason.get_or_insert(reason);
        }
    }

    /// Feed a frame of samples (typically one 20 ms RTP tick's worth).
    /// Returns true if samples were written to disk.
    pub async fn write(&mut self, samples: &[i16]) -> std::io::Result<bool> {
        if self.state == RecorderState::Finished { return Ok(false); }
        if self.state == RecorderState::Paused { return Ok(false); }

        // Rolling power average.
        let mut power = 0i32;
        for s in samples {
            let p = self.power_ma.execute(s.unsigned_abs() as i16) as i32;
            power = power.max(p);
        }

        // Start gate.
        if self.state == RecorderState::Pending {
            if matches!(self.cfg.start_above_power, Some(th) if power >= th) {
                self.state = RecorderState::Active;
                self.samples_since_active = 0;
            } else {
                return Ok(false);
            }
        }

        // Write.
        self.writer
            .write_samples(samples)
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let n = samples.len() as u64;
        self.samples_written += n;
        self.samples_since_active += n;

        // Duration-based finish.
        if let Some(max_ms) = self.cfg.max_duration_ms {
            let sr = self.cfg.sample_rate as u64 * self.cfg.num_channels as u64;
            if sr > 0 {
                let ms = self.samples_written * 1000 / sr;
                if ms >= max_ms {
                    self.request_finish(FinishReason::MaxDurationReached);
                    return Ok(true);
                }
            }
        }

        // Quiet-trailing-tail finish.
        if let Some(th) = self.cfg.finish_below_power {
            let sr = self.cfg.sample_rate as u64 * self.cfg.num_channels as u64;
            let active_ms = if sr > 0 { self.samples_since_active * 1000 / sr } else { 0 };
            let min_ok = self.cfg.min_duration_ms.map_or(true, |m| active_ms >= m);
            if min_ok && power < th {
                self.request_finish(FinishReason::BelowPowerThreshold);
            }
        }

        Ok(true)
    }

    /// Explicitly close the underlying WAV file. Idempotent.
    pub fn close(&mut self, reason: FinishReason) {
        self.request_finish(reason);
        let _ = self.writer.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(name: &str) -> RecorderConfig {
        let p = std::env::temp_dir().join(name);
        let _ = std::fs::remove_file(&p);
        RecorderConfig {
            file: p,
            num_channels: 1,
            sample_rate: 8000,
            max_duration_ms: None,
            start_above_power: None,
            finish_below_power: None,
            max_since_start_power: None,
            min_duration_ms: None,
            power_averaging_packets: None,
        }
    }

    #[tokio::test]
    async fn writes_frames_and_finalizes_via_drop() {
        let mut c = cfg("recorder_basic.wav");
        c.max_duration_ms = Some(100);
        let path = c.file.clone();

        let mut rec = Recorder::open(c).await.unwrap();
        for _ in 0..10 {
            rec.write(&vec![100i16; 160]).await.unwrap();
            if rec.is_finished() { break; }
        }
        assert!(rec.is_finished());
        assert_eq!(rec.finish_reason(), Some(&FinishReason::MaxDurationReached));
        drop(rec); // RAII finalize on WavWriter

        // Verify file header reflects data.
        let info = crate::soundfile::js_info(path.to_string_lossy().into_owned()).unwrap();
        assert!(info.subchunksize > 0);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn start_gate_holds_until_power_above_threshold() {
        let mut c = cfg("recorder_gate.wav");
        c.start_above_power = Some(50);
        let path = c.file.clone();

        let mut rec = Recorder::open(c).await.unwrap();
        // Silent frames — should not advance to Active.
        for _ in 0..5 { rec.write(&vec![0i16; 160]).await.unwrap(); }
        assert_eq!(rec.state(), RecorderState::Pending);

        // Loud frames — should open the gate.
        for _ in 0..10 { rec.write(&vec![5000i16; 160]).await.unwrap(); }
        assert_eq!(rec.state(), RecorderState::Active);

        rec.close(FinishReason::Completed);
        let _ = std::fs::remove_file(&path);
    }
}
