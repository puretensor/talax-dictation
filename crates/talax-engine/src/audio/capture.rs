use std::sync::Arc;
#[cfg(not(test))]
use std::sync::Mutex;
/// Audio capture via cpal.
///
/// Provides `AudioRecorder` which opens the default input device, captures
/// 16 kHz mono i16 PCM, and delivers chunks over a channel. If the hardware
/// does not natively support 16 kHz, the module resamples from the closest
/// available rate using linear interpolation.
///
/// All cpal device access is gated behind `#[cfg(not(test))]` so that unit
/// tests and CI builds succeed without a sound card.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use crate::audio::AudioConfig;

/// Errors that can occur during audio capture.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("no input device available")]
    NoDevice,
    #[error("no suitable input config found")]
    NoSuitableConfig,
    #[error("failed to build input stream: {0}")]
    BuildStream(String),
    #[error("failed to start stream: {0}")]
    PlayStream(String),
    #[error("recorder is not currently running")]
    NotRunning,
    #[error("recorder is already running")]
    AlreadyRunning,
}

/// Probe whether a usable default input device is available.
pub fn probe_default_input_device() -> Result<(), CaptureError> {
    #[cfg(test)]
    {
        Ok(())
    }

    #[cfg(not(test))]
    {
        use cpal::traits::{DeviceTrait, HostTrait};

        let host = cpal::default_host();
        let device = host.default_input_device().ok_or(CaptureError::NoDevice)?;
        let mut configs = device
            .supported_input_configs()
            .map_err(|_| CaptureError::NoSuitableConfig)?;

        if configs.next().is_some() {
            Ok(())
        } else {
            Err(CaptureError::NoSuitableConfig)
        }
    }
}

/// Resamples a buffer of i16 samples from `src_rate` to `dst_rate` using
/// linear interpolation. Both rates must be > 0.
pub(crate) fn resample_linear(samples: &[i16], src_rate: u32, dst_rate: u32) -> Vec<i16> {
    if src_rate == dst_rate || samples.is_empty() {
        return samples.to_vec();
    }

    let ratio = src_rate as f64 / dst_rate as f64;
    let out_len = ((samples.len() as f64) / ratio).ceil() as usize;
    let mut out = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos as usize;
        let frac = src_pos - idx as f64;

        let a = samples[idx] as f64;
        let b = if idx + 1 < samples.len() {
            samples[idx + 1] as f64
        } else {
            a
        };
        let interpolated = a + frac * (b - a);
        out.push(interpolated.round() as i16);
    }

    out
}

fn f32_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn u16_to_i16(sample: u16) -> i16 {
    (sample as i32 - 32_768) as i16
}

#[cfg(not(test))]
struct CapturedChunkSink<'a> {
    device_channels: usize,
    device_rate: u32,
    target_rate: u32,
    chunk_samples: usize,
    pending: &'a Arc<Mutex<Vec<i16>>>,
    acc_tx: &'a Sender<Vec<i16>>,
    chunk_tx: &'a Sender<Vec<i16>>,
}

#[cfg(not(test))]
fn process_captured_chunk(data: &[i16], sink: &CapturedChunkSink<'_>) {
    // Downmix to mono if needed.
    let mono: Vec<i16> = if sink.device_channels == 1 {
        data.to_vec()
    } else {
        data.chunks(sink.device_channels)
            .map(|frame| {
                let sum: i32 = frame.iter().map(|&s| s as i32).sum();
                (sum / sink.device_channels as i32) as i16
            })
            .collect()
    };

    // Resample if needed.
    let resampled = if sink.device_rate != sink.target_rate {
        resample_linear(&mono, sink.device_rate, sink.target_rate)
    } else {
        mono
    };

    let mut pending = match sink.pending.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    pending.extend_from_slice(&resampled);

    while pending.len() >= sink.chunk_samples {
        let chunk: Vec<i16> = pending.drain(..sink.chunk_samples).collect();
        let _ = sink.acc_tx.send(chunk.clone());
        let _ = sink.chunk_tx.send(chunk);
    }
}

/// Manages the lifecycle of an audio recording session.
///
/// Usage:
/// ```ignore
/// let mut recorder = AudioRecorder::new(AudioConfig::default());
/// let rx = recorder.start()?;
/// // ... receive chunks from rx ...
/// let all_samples = recorder.stop()?;
/// ```
pub struct AudioRecorder {
    config: AudioConfig,
    running: Arc<AtomicBool>,
    /// Sender side kept here so we can drop it on stop.
    chunk_tx: Option<Sender<Vec<i16>>>,
    /// Accumulator for all samples delivered so far (fed by a tap in the
    /// callback, not by draining the receiver).
    accumulator_rx: Option<Receiver<Vec<i16>>>,
    /// Handle to the background capture thread (non-test only).
    #[cfg(not(test))]
    _stream: Option<cpal::Stream>,
}

impl AudioRecorder {
    pub fn new(config: AudioConfig) -> Self {
        Self {
            config,
            running: Arc::new(AtomicBool::new(false)),
            chunk_tx: None,
            accumulator_rx: None,
            #[cfg(not(test))]
            _stream: None,
        }
    }

    /// Start recording. Returns a `Receiver` that yields audio chunks
    /// (each chunk is a `Vec<i16>` of PCM samples at the configured rate).
    ///
    /// On test builds this returns immediately with a dummy receiver (no
    /// actual audio device is opened).
    pub fn start(&mut self) -> Result<Receiver<Vec<i16>>, CaptureError> {
        if self.running.load(Ordering::SeqCst) {
            return Err(CaptureError::AlreadyRunning);
        }

        let (chunk_tx, chunk_rx) = mpsc::channel::<Vec<i16>>();
        let (acc_tx, acc_rx) = mpsc::channel::<Vec<i16>>();

        self.running.store(true, Ordering::SeqCst);
        self.chunk_tx = Some(chunk_tx.clone());
        self.accumulator_rx = Some(acc_rx);

        #[cfg(not(test))]
        {
            if let Err(err) = self.start_cpal_stream(chunk_tx, acc_tx) {
                self.running.store(false, Ordering::SeqCst);
                self.chunk_tx = None;
                self.accumulator_rx = None;
                return Err(err);
            }
        }

        #[cfg(test)]
        {
            // In tests, store senders so inject_test_samples can use them.
            let _ = acc_tx;
            let _ = chunk_tx;
        }

        Ok(chunk_rx)
    }

    /// Stop recording. Returns all accumulated samples captured during the
    /// session, concatenated in order.
    pub fn stop(&mut self) -> Result<Vec<i16>, CaptureError> {
        if !self.running.load(Ordering::SeqCst) {
            return Err(CaptureError::NotRunning);
        }

        self.running.store(false, Ordering::SeqCst);

        // Drop the stream (stops the cpal callback).
        #[cfg(not(test))]
        {
            self._stream = None;
        }

        // Drop the sender so the accumulator channel closes.
        self.chunk_tx = None;

        // Drain the accumulator.
        let mut all_samples = Vec::new();
        if let Some(rx) = self.accumulator_rx.take() {
            while let Ok(chunk) = rx.try_recv() {
                all_samples.extend_from_slice(&chunk);
            }
        }

        Ok(all_samples)
    }

    /// Whether the recorder is currently capturing.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Access the current config.
    pub fn config(&self) -> &AudioConfig {
        &self.config
    }

    // -- cpal-specific implementation, compiled out in tests --

    #[cfg(not(test))]
    fn start_cpal_stream(
        &mut self,
        chunk_tx: Sender<Vec<i16>>,
        acc_tx: Sender<Vec<i16>>,
    ) -> Result<(), CaptureError> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host = cpal::default_host();
        let device = host.default_input_device().ok_or(CaptureError::NoDevice)?;

        let supported_configs = device
            .supported_input_configs()
            .map_err(|_| CaptureError::NoSuitableConfig)?;

        // Try to find a config that supports our target sample rate, or pick
        // the closest one for resampling.
        let target_channels = self.config.channels;

        let mut best_config = None;
        let mut best_distance: i64 = i64::MAX;

        for cfg in supported_configs {
            // Prefer matching channel count.
            let channels_ok = cfg.channels() == target_channels;
            let min = cfg.min_sample_rate().0 as i64;
            let max = cfg.max_sample_rate().0 as i64;
            let target = self.config.sample_rate as i64;

            let rate_distance = if target >= min && target <= max {
                0i64
            } else {
                std::cmp::min((target - min).abs(), (target - max).abs())
            };

            let distance = rate_distance + if channels_ok { 0 } else { 100_000 };
            if distance < best_distance {
                best_distance = distance;
                // Clamp to the supported range.
                let clamped_rate =
                    (target as u32).clamp(cfg.min_sample_rate().0, cfg.max_sample_rate().0);
                best_config = Some(cfg.with_sample_rate(cpal::SampleRate(clamped_rate)));
            }
        }

        let selected = best_config.ok_or(CaptureError::NoSuitableConfig)?;
        let device_rate = selected.sample_rate().0;
        let device_channels = selected.channels() as usize;
        let sample_format = selected.sample_format();
        let target_rate_val = self.config.sample_rate;
        let running = self.running.clone();

        let chunk_samples =
            (self.config.sample_rate as usize * self.config.chunk_duration_ms as usize) / 1000;

        let pending = Arc::new(Mutex::new(Vec::with_capacity(chunk_samples * 2)));
        let stream_config: cpal::StreamConfig = selected.clone().into();

        let stream = match sample_format {
            cpal::SampleFormat::I16 => {
                let pending = Arc::clone(&pending);
                let acc_tx = acc_tx.clone();
                let chunk_tx = chunk_tx.clone();
                device
                    .build_input_stream(
                        &stream_config,
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            if !running.load(Ordering::Relaxed) {
                                return;
                            }
                            let sink = CapturedChunkSink {
                                device_channels,
                                device_rate,
                                target_rate: target_rate_val,
                                chunk_samples,
                                pending: &pending,
                                acc_tx: &acc_tx,
                                chunk_tx: &chunk_tx,
                            };
                            process_captured_chunk(data, &sink);
                        },
                        move |err| {
                            tracing::error!("audio capture error: {}", err);
                        },
                        None,
                    )
                    .map_err(|e| CaptureError::BuildStream(e.to_string()))?
            }
            cpal::SampleFormat::U16 => {
                let pending = Arc::clone(&pending);
                let running = self.running.clone();
                let acc_tx = acc_tx.clone();
                let chunk_tx = chunk_tx.clone();
                device
                    .build_input_stream(
                        &stream_config,
                        move |data: &[u16], _: &cpal::InputCallbackInfo| {
                            if !running.load(Ordering::Relaxed) {
                                return;
                            }
                            let converted: Vec<i16> =
                                data.iter().copied().map(u16_to_i16).collect();
                            let sink = CapturedChunkSink {
                                device_channels,
                                device_rate,
                                target_rate: target_rate_val,
                                chunk_samples,
                                pending: &pending,
                                acc_tx: &acc_tx,
                                chunk_tx: &chunk_tx,
                            };
                            process_captured_chunk(&converted, &sink);
                        },
                        move |err| {
                            tracing::error!("audio capture error: {}", err);
                        },
                        None,
                    )
                    .map_err(|e| CaptureError::BuildStream(e.to_string()))?
            }
            cpal::SampleFormat::F32 => {
                let pending = Arc::clone(&pending);
                let running = self.running.clone();
                let acc_tx = acc_tx.clone();
                let chunk_tx = chunk_tx.clone();
                device
                    .build_input_stream(
                        &stream_config,
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            if !running.load(Ordering::Relaxed) {
                                return;
                            }
                            let converted: Vec<i16> =
                                data.iter().copied().map(f32_to_i16).collect();
                            let sink = CapturedChunkSink {
                                device_channels,
                                device_rate,
                                target_rate: target_rate_val,
                                chunk_samples,
                                pending: &pending,
                                acc_tx: &acc_tx,
                                chunk_tx: &chunk_tx,
                            };
                            process_captured_chunk(&converted, &sink);
                        },
                        move |err| {
                            tracing::error!("audio capture error: {}", err);
                        },
                        None,
                    )
                    .map_err(|e| CaptureError::BuildStream(e.to_string()))?
            }
            other => {
                return Err(CaptureError::BuildStream(format!(
                    "unsupported input sample format: {other:?}"
                )));
            }
        };

        stream
            .play()
            .map_err(|e| CaptureError::PlayStream(e.to_string()))?;

        self._stream = Some(stream);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_identity() {
        let input = vec![1, 2, 3, 4, 5];
        let output = resample_linear(&input, 16000, 16000);
        assert_eq!(input, output);
    }

    #[test]
    fn resample_downsample_2x() {
        // 32000 -> 16000: should halve the number of samples.
        let input: Vec<i16> = (0..100).collect();
        let output = resample_linear(&input, 32000, 16000);
        assert_eq!(output.len(), 50);
        // First sample should be 0, last should be near 98.
        assert_eq!(output[0], 0);
    }

    #[test]
    fn resample_upsample_2x() {
        // 8000 -> 16000: should double.
        let input: Vec<i16> = vec![0, 100, 200, 300];
        let output = resample_linear(&input, 8000, 16000);
        assert_eq!(output.len(), 8);
        assert_eq!(output[0], 0);
        assert_eq!(output[2], 100);
    }

    #[test]
    fn resample_empty() {
        let output = resample_linear(&[], 44100, 16000);
        assert!(output.is_empty());
    }

    #[test]
    fn converts_f32_to_i16() {
        assert_eq!(f32_to_i16(0.0), 0);
        assert!(f32_to_i16(1.0) > 32_000);
        assert!(f32_to_i16(-1.0) < -32_000);
    }

    #[test]
    fn converts_u16_to_i16() {
        assert_eq!(u16_to_i16(32_768), 0);
        assert_eq!(u16_to_i16(0), -32_768);
        assert_eq!(u16_to_i16(u16::MAX), 32_767);
    }

    #[test]
    fn recorder_lifecycle() {
        let mut recorder = AudioRecorder::new(AudioConfig::default());
        assert!(!recorder.is_running());

        let _rx = recorder.start().unwrap();
        assert!(recorder.is_running());

        let samples = recorder.stop().unwrap();
        assert!(!recorder.is_running());
        // In test mode no actual audio is captured.
        assert!(samples.is_empty());
    }

    #[test]
    fn double_start_errors() {
        let mut recorder = AudioRecorder::new(AudioConfig::default());
        let _rx = recorder.start().unwrap();
        assert!(matches!(
            recorder.start(),
            Err(CaptureError::AlreadyRunning)
        ));
        let _ = recorder.stop();
    }

    #[test]
    fn stop_without_start_errors() {
        let mut recorder = AudioRecorder::new(AudioConfig::default());
        assert!(matches!(recorder.stop(), Err(CaptureError::NotRunning)));
    }
}
