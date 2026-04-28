/// Audio capture, voice activity detection, and buffering.
///
/// This module provides the building blocks for push-to-talk recording:
///
/// - [`capture::AudioRecorder`] -- opens the system microphone via cpal,
///   captures 16 kHz mono PCM, and delivers chunks over a channel.
/// - [`vad::VoiceActivityDetector`] -- energy-based VAD with adaptive noise
///   floor and configurable smoothing.
/// - [`ring_buffer::RingBuffer`] -- fixed-size circular buffer for pre-roll
///   audio.
pub mod capture;
pub mod ring_buffer;
pub mod vad;

pub use capture::{AudioRecorder, CaptureError};
pub use ring_buffer::RingBuffer;
pub use vad::{VadConfig, VadState, VoiceActivityDetector};

/// Configuration for the audio capture pipeline.
#[derive(Debug, Clone)]
pub struct AudioConfig {
    /// Target sample rate in Hz. Default: 16000 (16 kHz, required by Whisper).
    pub sample_rate: u32,
    /// Number of audio channels. Default: 1 (mono).
    pub channels: u16,
    /// Duration of each audio chunk in milliseconds. Default: 30 ms.
    /// Smaller values give lower latency but higher overhead.
    pub chunk_duration_ms: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16_000,
            channels: 1,
            chunk_duration_ms: 30,
        }
    }
}

impl AudioConfig {
    /// Number of samples per chunk (sample_rate * chunk_duration_ms / 1000).
    pub fn chunk_samples(&self) -> usize {
        (self.sample_rate as usize * self.chunk_duration_ms as usize) / 1000
    }

    /// Duration of one sample in seconds.
    pub fn sample_duration_secs(&self) -> f64 {
        1.0 / self.sample_rate as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let cfg = AudioConfig::default();
        assert_eq!(cfg.sample_rate, 16_000);
        assert_eq!(cfg.channels, 1);
        assert_eq!(cfg.chunk_duration_ms, 30);
    }

    #[test]
    fn chunk_samples_calculation() {
        let cfg = AudioConfig::default();
        // 16000 * 30 / 1000 = 480 samples per chunk
        assert_eq!(cfg.chunk_samples(), 480);
    }

    #[test]
    fn custom_config() {
        let cfg = AudioConfig {
            sample_rate: 44100,
            channels: 2,
            chunk_duration_ms: 20,
        };
        // 44100 * 20 / 1000 = 882
        assert_eq!(cfg.chunk_samples(), 882);
    }
}
