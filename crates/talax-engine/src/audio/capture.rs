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
use std::sync::{Arc, Mutex};

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
#[cfg(test)]
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

/// Linear resampler that retains its fractional source position and boundary
/// sample between device callbacks.
struct StreamingLinearResampler {
    src_rate: u64,
    dst_rate: u64,
    source_position: u64,
    input: Vec<i16>,
}

impl StreamingLinearResampler {
    fn new(src_rate: u32, dst_rate: u32) -> Self {
        debug_assert!(src_rate > 0 && dst_rate > 0);
        Self {
            src_rate: u64::from(src_rate),
            dst_rate: u64::from(dst_rate),
            source_position: 0,
            input: Vec::new(),
        }
    }

    fn process(&mut self, samples: &[i16]) -> Vec<i16> {
        self.input.extend_from_slice(samples);
        let mut output = Vec::new();

        while (self.source_position / self.dst_rate) as usize + 1 < self.input.len() {
            let index = (self.source_position / self.dst_rate) as usize;
            let fraction = (self.source_position % self.dst_rate) as f64 / self.dst_rate as f64;
            let a = self.input[index] as f64;
            let b = self.input[index + 1] as f64;
            output.push((a + fraction * (b - a)).round() as i16);
            self.source_position += self.src_rate;
        }

        // Samples before the next interpolation position are no longer
        // needed. For upsampling this deliberately retains the last sample
        // so the next callback can interpolate across the boundary.
        let consumed = ((self.source_position / self.dst_rate) as usize).min(self.input.len());
        if consumed > 0 {
            self.input.drain(..consumed);
            self.source_position -= consumed as u64 * self.dst_rate;
        }

        output
    }
}

fn f32_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn u16_to_i16(sample: u16) -> i16 {
    (sample as i32 - 32_768) as i16
}

/// Conversion parameters for a capture stream: device shape in, target shape out.
#[cfg_attr(test, allow(dead_code))]
#[derive(Debug, Clone, Copy)]
struct ChunkSpec {
    device_channels: usize,
    device_rate: u32,
    target_rate: u32,
    chunk_samples: usize,
}

#[cfg_attr(test, allow(dead_code))]
struct CaptureBuffer {
    pending: Vec<i16>,
    resampler: Option<StreamingLinearResampler>,
}

impl CaptureBuffer {
    fn new(spec: ChunkSpec) -> Self {
        Self {
            pending: Vec::with_capacity(spec.chunk_samples * 2),
            resampler: (spec.device_rate != spec.target_rate)
                .then(|| StreamingLinearResampler::new(spec.device_rate, spec.target_rate)),
        }
    }
}

#[cfg(not(test))]
fn process_captured_chunk(
    data: &[i16],
    spec: ChunkSpec,
    buffer: &Arc<Mutex<CaptureBuffer>>,
    acc_tx: &Sender<Vec<i16>>,
    chunk_tx: &Sender<Vec<i16>>,
) {
    // Downmix to mono if needed.
    let mono: Vec<i16> = if spec.device_channels == 1 {
        data.to_vec()
    } else {
        data.chunks(spec.device_channels)
            .map(|frame| {
                let sum: i32 = frame.iter().map(|&s| s as i32).sum();
                (sum / spec.device_channels as i32) as i16
            })
            .collect()
    };

    let mut buffer = match buffer.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    let resampled = match buffer.resampler.as_mut() {
        Some(resampler) => resampler.process(&mono),
        None => mono,
    };
    buffer.pending.extend_from_slice(&resampled);

    while buffer.pending.len() >= spec.chunk_samples {
        let chunk: Vec<i16> = buffer.pending.drain(..spec.chunk_samples).collect();
        let _ = acc_tx.send(chunk.clone());
        let _ = chunk_tx.send(chunk);
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
    /// Sender for the accumulator tap, retained so stop can flush a tail.
    accumulator_tx: Option<Sender<Vec<i16>>>,
    /// Accumulator for all samples delivered so far (fed by a tap in the
    /// callback, not by draining the receiver).
    accumulator_rx: Option<Receiver<Vec<i16>>>,
    /// Capture state shared with the device callback and stop-time tail flush.
    capture_buffer: Option<Arc<Mutex<CaptureBuffer>>>,
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
            accumulator_tx: None,
            accumulator_rx: None,
            capture_buffer: None,
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
        self.accumulator_tx = Some(acc_tx.clone());
        self.accumulator_rx = Some(acc_rx);
        let spec = ChunkSpec {
            device_channels: self.config.channels as usize,
            device_rate: self.config.sample_rate,
            target_rate: self.config.sample_rate,
            chunk_samples: self.config.chunk_samples(),
        };
        let buffer = Arc::new(Mutex::new(CaptureBuffer::new(spec)));
        self.capture_buffer = Some(Arc::clone(&buffer));

        #[cfg(not(test))]
        {
            if let Err(err) = self.start_cpal_stream(chunk_tx, acc_tx, buffer) {
                self.running.store(false, Ordering::SeqCst);
                self.chunk_tx = None;
                self.accumulator_tx = None;
                self.accumulator_rx = None;
                self.capture_buffer = None;
                return Err(err);
            }
        }

        #[cfg(test)]
        {
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

        // A callback only publishes complete chunks. Once the stream has
        // stopped, publish the final short chunk instead of discarding up to
        // `chunk_duration_ms` of captured audio.
        if let Some(buffer) = self.capture_buffer.take() {
            let tail = {
                let mut buffer = buffer.lock().unwrap_or_else(|e| e.into_inner());
                std::mem::take(&mut buffer.pending)
            };
            if !tail.is_empty() {
                if let Some(accumulator_tx) = &self.accumulator_tx {
                    let _ = accumulator_tx.send(tail.clone());
                }
                if let Some(chunk_tx) = &self.chunk_tx {
                    let _ = chunk_tx.send(tail);
                }
            }
        }

        // Drop the senders so the channels close.
        self.accumulator_tx = None;
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

    #[cfg(test)]
    fn inject_pending_test_samples(&self, samples: &[i16]) {
        self.capture_buffer
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .pending
            .extend_from_slice(samples);
    }

    // -- cpal-specific implementation, compiled out in tests --

    #[cfg(not(test))]
    fn start_cpal_stream(
        &mut self,
        chunk_tx: Sender<Vec<i16>>,
        acc_tx: Sender<Vec<i16>>,
        buffer: Arc<Mutex<CaptureBuffer>>,
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
            let min = cfg.min_sample_rate() as i64;
            let max = cfg.max_sample_rate() as i64;
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
                    (target as u32).clamp(cfg.min_sample_rate(), cfg.max_sample_rate());
                best_config = Some(cfg.with_sample_rate(clamped_rate));
            }
        }

        let selected = best_config.ok_or(CaptureError::NoSuitableConfig)?;
        let device_rate = selected.sample_rate();
        let device_channels = selected.channels() as usize;
        let sample_format = selected.sample_format();
        let target_rate_val = self.config.sample_rate;
        let running = self.running.clone();

        let chunk_samples =
            (self.config.sample_rate as usize * self.config.chunk_duration_ms as usize) / 1000;
        let spec = ChunkSpec {
            device_channels,
            device_rate,
            target_rate: target_rate_val,
            chunk_samples,
        };

        *buffer.lock().unwrap_or_else(|e| e.into_inner()) = CaptureBuffer::new(spec);
        let stream_config: cpal::StreamConfig = selected.into();

        let stream = match sample_format {
            cpal::SampleFormat::I16 => {
                let buffer = Arc::clone(&buffer);
                let acc_tx = acc_tx.clone();
                let chunk_tx = chunk_tx.clone();
                device
                    .build_input_stream(
                        stream_config,
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            if !running.load(Ordering::Relaxed) {
                                return;
                            }
                            process_captured_chunk(data, spec, &buffer, &acc_tx, &chunk_tx);
                        },
                        move |err| {
                            tracing::error!("audio capture error: {}", err);
                        },
                        None,
                    )
                    .map_err(|e| CaptureError::BuildStream(e.to_string()))?
            }
            cpal::SampleFormat::U16 => {
                let buffer = Arc::clone(&buffer);
                let running = self.running.clone();
                let acc_tx = acc_tx.clone();
                let chunk_tx = chunk_tx.clone();
                device
                    .build_input_stream(
                        stream_config,
                        move |data: &[u16], _: &cpal::InputCallbackInfo| {
                            if !running.load(Ordering::Relaxed) {
                                return;
                            }
                            let converted: Vec<i16> =
                                data.iter().copied().map(u16_to_i16).collect();
                            process_captured_chunk(&converted, spec, &buffer, &acc_tx, &chunk_tx);
                        },
                        move |err| {
                            tracing::error!("audio capture error: {}", err);
                        },
                        None,
                    )
                    .map_err(|e| CaptureError::BuildStream(e.to_string()))?
            }
            cpal::SampleFormat::F32 => {
                let buffer = Arc::clone(&buffer);
                let running = self.running.clone();
                let acc_tx = acc_tx.clone();
                let chunk_tx = chunk_tx.clone();
                device
                    .build_input_stream(
                        stream_config,
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            if !running.load(Ordering::Relaxed) {
                                return;
                            }
                            let converted: Vec<i16> =
                                data.iter().copied().map(f32_to_i16).collect();
                            process_captured_chunk(&converted, spec, &buffer, &acc_tx, &chunk_tx);
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
    fn chunked_resampling_preserves_the_target_rate() {
        let input: Vec<i16> = (0..44_100)
            .map(|index| {
                let phase = 2.0 * std::f64::consts::PI * 440.0 * index as f64 / 44_100.0;
                (phase.sin() * 10_000.0).round() as i16
            })
            .collect();
        let expected = resample_linear(&input, 44_100, 16_000);
        let mut resampler = StreamingLinearResampler::new(44_100, 16_000);
        let output: Vec<i16> = input
            .chunks(128)
            .flat_map(|chunk| resampler.process(chunk))
            .collect();

        assert_eq!(output.len(), 16_000);
        assert!(
            output
                .iter()
                .zip(expected)
                .all(|(actual, expected)| (*actual as i32 - expected as i32).abs() <= 1)
        );
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

    #[test]
    fn stop_returns_a_partial_final_chunk() {
        let mut recorder = AudioRecorder::new(AudioConfig::default());
        let chunk_rx = recorder.start().unwrap();
        let tail = vec![7_i16; recorder.config().chunk_samples() - 1];
        recorder.inject_pending_test_samples(&tail);

        assert_eq!(recorder.stop().unwrap(), tail);
        assert_eq!(chunk_rx.recv().unwrap(), tail);
    }
}
