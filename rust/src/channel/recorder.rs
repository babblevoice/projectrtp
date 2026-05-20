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
    ///
    /// Power calc uses the incoming narrowband (pre-interleave) samples —
    /// `power_samples` — not the wide frame that's being written. This
    /// matches C++ `codecx::power()` which operates on the narrowband
    /// slice, and avoids the DC blocker's state crossing L/R boundaries
    /// on stereo recordings (which biases the smoothed power and delays
    /// the below-power gate by ~200 ms).
    ///
    /// When `power_samples` is empty the caller has no narrowband slice
    /// to offer (e.g. the `write_raw` code path) and we fall back to
    /// using the frame itself — preserves backward compatibility for
    /// callers that don't plumb the narrowband through.
    pub async fn write_with_count(&mut self, samples: &[i16], channel_in_count: Option<u64>) -> std::io::Result<bool> {
        self.write_frame(samples, samples, channel_in_count).await
    }

    /// Same as `write_with_count` but lets the caller pass a narrowband
    /// (pre-interleave) sample slice to drive the DC blocker / RMS /
    /// MA. The frame written to disk can still be the full stereo
    /// interleaving — only the power path is changed.
    pub async fn write_frame(
        &mut self,
        samples: &[i16],
        power_samples: &[i16],
        channel_in_count: Option<u64>,
    ) -> std::io::Result<bool> {
        if self.state == RecorderState::Finished { return Ok(false); }
        if self.state == RecorderState::Paused { return Ok(false); }
        self.packets_observed += 1;

        let warmup_count = channel_in_count.unwrap_or(self.packets_observed);

        // Power source: prefer the explicit narrowband; if empty, fall
        // back to the frame itself for compatibility.
        let power_src: &[i16] = if !power_samples.is_empty() { power_samples } else { samples };

        let pkt_power: i32 = if warmup_count < 100 || power_src.is_empty() {
            0
        } else {
            let mut sum_sq: u64 = 0;
            for s in power_src {
                let filtered = self.dc_filter.execute(*s) as i64;
                sum_sq += (filtered * filtered) as u64;
            }
            ((sum_sq / power_src.len() as u64) as f64).sqrt() as i32
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

    /// Write samples directly to the WAV file, bypassing state, power calc,
    /// start-gate and finish-check logic. Used to flush the pre-buffer
    /// collected during `playrecord`'s play phase — those samples are
    /// "decided" audio (the caller's speech during/before the prompt) and
    /// should not be subject to the recorder's start-above-power gate.
    /// Matches C++ `writeraw`.
    ///
    /// Returns the number of samples written.
    pub async fn write_raw(&mut self, samples: &[i16]) -> std::io::Result<usize> {
        if self.state == RecorderState::Finished { return Ok(0); }
        if samples.is_empty() { return Ok(0); }
        self.writer
            .write_samples(samples)
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        self.samples_written += samples.len() as u64;
        Ok(samples.len())
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
        // The recorder has a 100-packet warmup during which `pkt_power`
        // is clamped to 0 — matches the C++ codecx.inpkcount gate. It
        // also pre-filters with a DC blocker, so a DC-constant input
        // like `vec![5000i16; 160]` still produces zero power once the
        // blocker's transient decays. Use an alternating sign so the
        // RMS-after-DC-filter is non-zero.
        let mut c = cfg("recorder_gate.wav");
        c.start_above_power = Some(50);
        let path = c.file.clone();

        let loud_ac: Vec<i16> = (0..160).map(|i| if i % 2 == 0 { 5000 } else { -5000 }).collect();

        let mut rec = Recorder::open(c).await.unwrap();
        // Silent frames during warmup — gate stays closed.
        for _ in 0..5 { rec.write(&vec![0i16; 160]).await.unwrap(); }
        assert_eq!(rec.state(), RecorderState::Pending);

        // Pre-warmup loud frames — gate still closed because power is
        // clamped to 0 until packets_observed ≥ 100.
        for _ in 0..50 { rec.write(&loud_ac).await.unwrap(); }
        assert_eq!(rec.state(), RecorderState::Pending);

        // Now cross the warmup and keep feeding — MA filter fills,
        // crosses 50, gate opens.
        for _ in 0..200 { rec.write(&loud_ac).await.unwrap(); }
        assert_eq!(rec.state(), RecorderState::Active);

        rec.close(FinishReason::Completed);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn write_raw_bypasses_start_gate() {
        // When start_above_power is set, normal write() should be gated —
        // silent samples fed via write() never reach the file. But write_raw
        // must bypass the gate and write regardless.
        let mut c = cfg("recorder_write_raw.wav");
        c.start_above_power = Some(50);
        let path = c.file.clone();

        let mut rec = Recorder::open(c).await.unwrap();
        let written = rec.write_raw(&vec![0i16; 800]).await.unwrap();
        assert_eq!(written, 800);
        // State stays Pending — write_raw doesn't touch the gate.
        assert_eq!(rec.state(), RecorderState::Pending);

        rec.close(FinishReason::Completed);

        let info = crate::soundfile::js_info(path.to_string_lossy().into_owned()).unwrap();
        // 800 samples × 2 bytes/sample = 1600 PCM bytes.
        assert_eq!(info.subchunksize, 1600);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn write_raw_noop_after_finish() {
        let mut c = cfg("recorder_write_raw_finished.wav");
        c.max_duration_ms = Some(10);
        let path = c.file.clone();

        let mut rec = Recorder::open(c).await.unwrap();
        // Push enough frames to hit max_duration.
        for _ in 0..5 { rec.write(&vec![100i16; 160]).await.unwrap(); }
        assert!(rec.is_finished());

        let written = rec.write_raw(&vec![0i16; 100]).await.unwrap();
        assert_eq!(written, 0);

        rec.close(FinishReason::Completed);
        let _ = std::fs::remove_file(&path);
    }
}
