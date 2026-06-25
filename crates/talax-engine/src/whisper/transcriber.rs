//! Whisper transcription wrapper around whisper-rs.
//!
//! Loads a GGML model once and reuses it across multiple transcription calls.
//! Audio must be mono f32 PCM at 16 kHz (whisper.cpp native format).

use std::path::Path;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum TranscriberError {
    #[error("failed to load whisper model: {0}")]
    ModelLoad(String),
    #[error("transcription failed: {0}")]
    Transcription(String),
    #[error("invalid audio: {0}")]
    InvalidAudio(String),
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// Parameters controlling a single transcription run.
#[derive(Debug, Clone, Default)]
pub struct TranscribeParams {
    /// Language code (e.g. `"en"`). `None` = auto-detect.
    pub language: Option<String>,
    /// `false` = transcribe in the original language,
    /// `true`  = translate to English.
    pub translate: bool,
    /// Number of threads for decoding. `None` = auto (based on CPU count).
    pub n_threads: Option<i32>,
    /// Print progress percentage to stderr via whisper.cpp.
    pub print_progress: bool,
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// A single transcription segment with timestamps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    /// Start time in seconds.
    pub start_time: f64,
    /// End time in seconds.
    pub end_time: f64,
    /// Transcribed text for this segment.
    pub text: String,
}

/// The full result of a transcription run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscribeResult {
    /// Individual segments with timestamps.
    pub segments: Vec<Segment>,
    /// All segment texts joined together.
    pub full_text: String,
    /// Wall-clock time spent on transcription, in milliseconds.
    pub processing_time_ms: u64,
}

// ---------------------------------------------------------------------------
// Transcriber
// ---------------------------------------------------------------------------

/// Holds a loaded Whisper model and exposes transcription methods.
///
/// The model is loaded once at construction time.  Each call to
/// [`Transcriber::transcribe`] creates a fresh `WhisperState` so multiple
/// calls are safe but not concurrent (whisper.cpp is not thread-safe within
/// a single context/state pair).
pub struct Transcriber {
    ctx: WhisperContext,
}

impl Transcriber {
    /// Load a whisper GGML model from disk.
    ///
    /// # Errors
    /// Returns [`TranscriberError::ModelLoad`] if the file does not exist or
    /// cannot be parsed as a valid GGML model.
    pub fn new(model_path: &Path) -> Result<Self, TranscriberError> {
        if !model_path.exists() {
            return Err(TranscriberError::ModelLoad(format!(
                "model file not found: {}",
                model_path.display()
            )));
        }

        let params = WhisperContextParameters::default();
        let ctx = WhisperContext::new_with_params(model_path, params)
            .map_err(|e| TranscriberError::ModelLoad(e.to_string()))?;

        Ok(Self { ctx })
    }

    /// Transcribe mono f32 PCM audio at 16 kHz.
    ///
    /// # Errors
    /// - [`TranscriberError::InvalidAudio`] if `audio` is empty.
    /// - [`TranscriberError::Transcription`] if whisper.cpp fails internally.
    pub fn transcribe(
        &self,
        audio: &[f32],
        params: &TranscribeParams,
    ) -> Result<TranscribeResult, TranscriberError> {
        if audio.is_empty() {
            return Err(TranscriberError::InvalidAudio(
                "audio buffer is empty".to_string(),
            ));
        }

        // -- build FullParams ------------------------------------------------
        let mut full_params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        // Language: None => auto-detect, Some(lang) => force that language.
        if let Some(ref lang) = params.language {
            full_params.set_language(Some(lang.as_str()));
        } else {
            full_params.set_language(None);
            full_params.set_detect_language(true);
        }

        full_params.set_translate(params.translate);
        full_params.set_print_progress(params.print_progress);
        full_params.set_print_realtime(false);
        full_params.set_print_special(false);
        full_params.set_print_timestamps(false);

        // Thread count: use caller value, or auto-select from available CPUs.
        let n_threads = params.n_threads.unwrap_or_else(default_thread_count);
        full_params.set_n_threads(n_threads);

        // -- create state & run ----------------------------------------------
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| TranscriberError::Transcription(e.to_string()))?;

        let start = Instant::now();

        state
            .full(full_params, audio)
            .map_err(|e| TranscriberError::Transcription(e.to_string()))?;

        let processing_time_ms = start.elapsed().as_millis() as u64;

        // -- collect segments ------------------------------------------------
        let n_segments = state.full_n_segments();
        let mut segments = Vec::with_capacity(n_segments as usize);
        let mut full_text = String::new();

        for i in 0..n_segments {
            let seg = state.get_segment(i).ok_or_else(|| {
                TranscriberError::Transcription(format!("failed to get segment {i}"))
            })?;

            // Timestamps from whisper.cpp are in centiseconds (10 ms units).
            let start_time = seg.start_timestamp() as f64 / 100.0;
            let end_time = seg.end_timestamp() as f64 / 100.0;
            let text = seg
                .to_str_lossy()
                .map_err(|e| TranscriberError::Transcription(e.to_string()))?
                .to_string();

            full_text.push_str(&text);

            segments.push(Segment {
                start_time,
                end_time,
                text,
            });
        }

        Ok(TranscribeResult {
            segments,
            full_text,
            processing_time_ms,
        })
    }

    /// Convenience method: converts i16 PCM samples to f32 then transcribes.
    ///
    /// The conversion divides each sample by 32768.0 to normalise into
    /// the [-1.0, 1.0] range expected by whisper.cpp.
    pub fn transcribe_from_i16(
        &self,
        audio: &[i16],
        params: &TranscribeParams,
    ) -> Result<TranscribeResult, TranscriberError> {
        if audio.is_empty() {
            return Err(TranscriberError::InvalidAudio(
                "audio buffer is empty".to_string(),
            ));
        }

        let f32_audio: Vec<f32> = audio.iter().map(|&s| s as f32 / 32768.0).collect();
        self.transcribe(&f32_audio, params)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Pick a sensible default thread count: min(4, available_parallelism).
fn default_thread_count() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get().min(4) as i32)
        .unwrap_or(1)
}

// ---------------------------------------------------------------------------
// Conversion helper (public for reuse)
// ---------------------------------------------------------------------------

/// Convert i16 PCM samples to f32 in [-1.0, 1.0].
pub fn i16_samples_to_f32(samples: &[i16]) -> Vec<f32> {
    samples.iter().map(|&s| s as f32 / 32768.0).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_params_are_sane() {
        let p = TranscribeParams::default();
        assert!(p.language.is_none());
        assert!(!p.translate);
        assert!(p.n_threads.is_none());
        assert!(!p.print_progress);
    }

    #[test]
    fn params_with_language() {
        let p = TranscribeParams {
            language: Some("en".to_string()),
            translate: false,
            n_threads: Some(2),
            print_progress: true,
        };
        assert_eq!(p.language.as_deref(), Some("en"));
        assert_eq!(p.n_threads, Some(2));
        assert!(p.print_progress);
    }

    #[test]
    fn i16_to_f32_conversion_basic() {
        let samples: Vec<i16> = vec![0, 16384, -16384, 32767, -32768];
        let converted = i16_samples_to_f32(&samples);

        assert_eq!(converted.len(), 5);

        // 0 -> 0.0
        assert!((converted[0] - 0.0).abs() < f32::EPSILON);
        // 16384 -> 0.5
        assert!((converted[1] - 0.5).abs() < 0.001);
        // -16384 -> -0.5
        assert!((converted[2] - (-0.5)).abs() < 0.001);
        // 32767 -> ~1.0 (just under)
        assert!((converted[3] - 1.0).abs() < 0.001);
        // -32768 -> -1.0
        assert!((converted[4] - (-1.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn i16_to_f32_empty() {
        let empty: Vec<i16> = vec![];
        let converted = i16_samples_to_f32(&empty);
        assert!(converted.is_empty());
    }

    #[test]
    fn default_thread_count_is_positive() {
        let n = default_thread_count();
        assert!(n >= 1);
        assert!(n <= 4);
    }

    #[test]
    fn segment_serialization_roundtrip() {
        let seg = Segment {
            start_time: 1.5,
            end_time: 3.2,
            text: "Hello world".to_string(),
        };
        let json = serde_json::to_string(&seg).unwrap();
        let deser: Segment = serde_json::from_str(&json).unwrap();
        assert!((deser.start_time - 1.5).abs() < f64::EPSILON);
        assert!((deser.end_time - 3.2).abs() < f64::EPSILON);
        assert_eq!(deser.text, "Hello world");
    }

    #[test]
    fn transcribe_result_serialization_roundtrip() {
        let result = TranscribeResult {
            segments: vec![
                Segment {
                    start_time: 0.0,
                    end_time: 1.0,
                    text: "First".to_string(),
                },
                Segment {
                    start_time: 1.0,
                    end_time: 2.0,
                    text: " second".to_string(),
                },
            ],
            full_text: "First second".to_string(),
            processing_time_ms: 42,
        };
        let json = serde_json::to_string(&result).unwrap();
        let deser: TranscribeResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.segments.len(), 2);
        assert_eq!(deser.full_text, "First second");
        assert_eq!(deser.processing_time_ms, 42);
    }

    #[test]
    fn transcriber_new_rejects_missing_model() {
        let bad_path = Path::new("/tmp/nonexistent-whisper-model.bin");
        let result = Transcriber::new(bad_path);
        assert!(result.is_err());
        let err = result.err().unwrap();
        match err {
            TranscriberError::ModelLoad(msg) => {
                assert!(msg.contains("not found"), "got: {msg}");
            }
            other => panic!("expected ModelLoad, got: {other:?}"),
        }
    }
}
