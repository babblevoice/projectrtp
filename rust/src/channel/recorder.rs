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

use crate::firfilter::{DcFilter, MaFilter};
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
    /// Moving average of *per-packet peak power* (not per-sample). Matches
    /// the C++ pipeline (`incodec.power()` then `poweravg(power)` in
    /// projectrtpchannel.cpp). Per-packet makes the convergence time =
    /// `power_averaging_packets` * 20 ms, which is what tests expect.
    power_ma: MaFilter,
    /// DC-blocker applied sample-by-sample before the RMS sum, matching
    /// `dcpowerfilter` in `codecx::power()` (projectrtpcodecx.cpp:584).
    /// Its transient at amplitude-transitions is the main reason per-packet
    /// RMS tracks the envelope faster than a naive `sqrt(mean(s^2))`.
    dc_filter: DcFilter,
    /// Number of packets the recorder has observed (including warmup). The
    /// 100-packet threshold counts from recorder open, which is also how
    /// `codecx::inpkcount` counts in C++ (incremented when the codec is
    /// fed, which is post-jitter).
    packets_observed: u64,
    samples_written: u64,
    samples_since_active: u64,
    /// Last smoothed power value — fed to below-power / above-power checks.
    last_power_calc: i32,
    /// Peak smoothed power observed since the gate opened. The below-power
    /// finish fires only if `max_since_active > threshold && last < threshold`,
    /// matching the C++ gating in projectrtpchannel.cpp:917-925.
    max_since_active_power: i32,
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
            dc_filter: DcFilter::new(),
            packets_observed: 0,
            samples_written: 0,
            samples_since_active: 0,
            last_power_calc: 0,
            max_since_active_power: 0,
            finish_reason: None,
        })
    }

    pub fn state(&self) -> RecorderState { self.state }
    pub fn is_finished(&self) -> bool { self.state == RecorderState::Finished }
    pub fn finish_reason(&self) -> Option<&FinishReason> { self.finish_reason.as_ref() }
    pub fn file(&self) -> &std::path::Path { &self.cfg.file }
    pub fn num_channels(&self) -> u16 { self.cfg.num_channels }
    /// Total file size in bytes (header + PCM data written so far).
    pub fn file_size(&self) -> u64 { self.writer.bytes_written() + crate::soundfile::WAV_HEADER_LEN as u64 }

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
    /// The 100-packet warm-up counts from recorder open (same as C++
    /// `codecx::inpkcount` which increments post-jitter). Returns true if
    /// samples were written.
    #[allow(dead_code)]
    pub async fn write(&mut self, samples: &[i16]) -> std::io::Result<bool> {
        self.write_with_count(samples, None).await
    }

    /// Feed a frame with an external packet count (channel-wide `in_count`).
    /// The 100-packet warm-up uses this count when provided, matching the C++
    /// `codecx::inpkcount` which is shared across the channel's lifetime
    /// rather than per-recorder.
    pub async fn write_with_count(&mut self, samples: &[i16], channel_in_count: Option<u64>) -> std::io::Result<bool> {
        if self.state == RecorderState::Finished { return Ok(false); }
        if self.state == RecorderState::Paused { return Ok(false); }
        self.packets_observed += 1;

        let warmup_count = channel_in_count.unwrap_or(self.packets_observed);

        let pkt_power: i32 = if warmup_count < 100 || samples.is_empty() {
            0
        } else {
            let mut sum_sq: u64 = 0;
            for s in samples {
                let filtered = self.dc_filter.execute(*s) as i64;
                sum_sq += (filtered * filtered) as u64;
            }
            ((sum_sq / samples.len() as u64) as f64).sqrt() as i32
        };
        let pkt_power16 = pkt_power.min(i16::MAX as i32) as i16;
        self.last_power_calc = self.power_ma.execute(pkt_power16) as i32;

        // Start gate — open when smoothed peak crosses the threshold.
        if self.state == RecorderState::Pending {
            if matches!(self.cfg.start_above_power, Some(th) if self.last_power_calc > th) {
                self.state = RecorderState::Active;
                self.samples_since_active = 0;
                self.max_since_active_power = self.last_power_calc;
            } else {
                return Ok(false);
            }
        }

        // Track peak smoothed power since gate opened.
        if self.last_power_calc > self.max_since_active_power {
            self.max_since_active_power = self.last_power_calc;
        }

        // Write.
        self.writer
            .write_samples(samples)
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let n = samples.len() as u64;
        self.samples_written += n;
        self.samples_since_active += n;

        // active_ms = elapsed since gate open. Frame length is samples per
        // channel × channels, so divide by (sr * channels) to get seconds.
        let sr = self.cfg.sample_rate as u64 * self.cfg.num_channels as u64;
        let active_ms = if sr > 0 { self.samples_since_active * 1000 / sr } else { 0 };

        if let Some(min) = self.cfg.min_duration_ms {
            if active_ms < min {
                // Below min duration — skip finish-checks (matches C++
                // projectrtpchannel.cpp:908 `continue` if diff < minduration).
                return Ok(true);
            }
        }

        // Below-power finish: only after we've heard above-threshold once.
        if let Some(th) = self.cfg.finish_below_power {
            if self.max_since_active_power > th && self.last_power_calc < th {
                self.request_finish(FinishReason::BelowPowerThreshold);
                return Ok(true);
            }
        }

        // Max-duration finish.
        if let Some(max_ms) = self.cfg.max_duration_ms {
            if active_ms > max_ms {
                self.request_finish(FinishReason::MaxDurationReached);
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
