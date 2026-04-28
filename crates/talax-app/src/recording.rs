//! Recording orchestrator: hotkey -> audio capture -> whisper -> pipeline -> inject.
//!
//! Audio capture runs on a dedicated thread (cpal Stream is !Send), communicating
//! via channels. The orchestrator itself is Send + Sync safe.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use talax_engine::audio::{AudioConfig, RingBuffer, VadState, VoiceActivityDetector};
use talax_engine::inject::{InjectionConfig, InjectionMode, TextInjector};
use talax_engine::pipeline::PipelineResult;
use talax_engine::whisper::transcriber::{TranscribeResult, Transcriber};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Current recording state, broadcast to the frontend via events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingState {
    Idle,
    Recording,
    Processing,
    Injecting,
    Error,
}

/// Payload emitted with recording state changes.
#[derive(Debug, Clone, Serialize)]
pub struct RecordingEvent {
    pub state: RecordingState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Payload emitted when transcription + correction completes.
#[derive(Debug, Clone, Serialize)]
pub struct TranscriptionEvent {
    pub session_id: String,
    pub raw: TranscribeResult,
    pub corrected: PipelineResult,
    pub processing_time_ms: u64,
}

// ---------------------------------------------------------------------------
// Recording thread handle
// ---------------------------------------------------------------------------

/// Handle to a running audio capture thread.
/// The capture thread creates its own AudioRecorder (which is !Send)
/// and returns filtered samples when stopped.
struct CaptureHandle {
    stop_flag: Arc<AtomicBool>,
    samples_rx: mpsc::Receiver<Vec<i16>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl CaptureHandle {
    /// Signal the capture thread to stop and collect all recorded samples.
    fn stop(mut self) -> Vec<i16> {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }

        let mut all_samples = Vec::new();
        while let Ok(chunk) = self.samples_rx.try_recv() {
            all_samples.extend_from_slice(&chunk);
        }
        all_samples
    }
}

#[derive(Debug, Clone, Copy)]
struct CaptureSettings {
    vad_enabled: bool,
    pre_roll_ms: u32,
    silence_stop_ms: u32,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            vad_enabled: true,
            pre_roll_ms: 300,
            silence_stop_ms: 700,
        }
    }
}

struct ChunkProcessor {
    settings: CaptureSettings,
    pre_roll: RingBuffer,
    trailing_silence: Vec<i16>,
    vad: VoiceActivityDetector,
    speech_started: bool,
    trailing_limit_samples: usize,
}

impl ChunkProcessor {
    fn new(audio_config: &AudioConfig, settings: CaptureSettings) -> Self {
        let pre_roll_samples =
            (audio_config.sample_rate as usize * settings.pre_roll_ms as usize) / 1000;
        let trailing_limit_samples =
            (audio_config.sample_rate as usize * settings.silence_stop_ms as usize) / 1000;

        Self {
            settings,
            pre_roll: RingBuffer::new(pre_roll_samples),
            trailing_silence: Vec::with_capacity(trailing_limit_samples),
            vad: VoiceActivityDetector::with_defaults(),
            speech_started: false,
            trailing_limit_samples,
        }
    }

    fn process_chunk(&mut self, chunk: &[i16], output: &mut Vec<i16>) {
        if chunk.is_empty() {
            return;
        }

        if !self.settings.vad_enabled {
            output.extend_from_slice(chunk);
            return;
        }

        self.pre_roll.push_chunk(chunk);
        let vad_state = self.vad.process_chunk(chunk);

        if !self.speech_started {
            if vad_state == VadState::Speaking {
                self.speech_started = true;
                output.extend(self.pre_roll.drain());
            }
            return;
        }

        if matches!(vad_state, VadState::Speaking | VadState::Transition) {
            if !self.trailing_silence.is_empty() {
                output.extend(self.trailing_silence.drain(..));
            }
            output.extend_from_slice(chunk);
            return;
        }

        self.trailing_silence.extend_from_slice(chunk);
        if self.trailing_silence.len() > self.trailing_limit_samples {
            let extra = self.trailing_silence.len() - self.trailing_limit_samples;
            self.trailing_silence.drain(..extra);
        }
    }
}

/// Spawn a capture thread that records audio and returns filtered samples.
fn spawn_capture_thread(settings: CaptureSettings) -> Result<CaptureHandle, String> {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<Vec<i16>>();
    let (startup_tx, startup_rx) = mpsc::channel::<Result<(), String>>();

    let flag = stop_flag.clone();
    let thread = std::thread::spawn(move || {
        use talax_engine::audio::capture::AudioRecorder;

        let config = AudioConfig::default();
        let mut recorder = AudioRecorder::new(config);
        let mut processor = ChunkProcessor::new(recorder.config(), settings);
        let mut filtered_samples = Vec::new();

        match recorder.start() {
            Ok(chunk_rx) => {
                let _ = startup_tx.send(Ok(()));

                while !flag.load(Ordering::Relaxed) {
                    match chunk_rx.recv_timeout(std::time::Duration::from_millis(50)) {
                        Ok(chunk) => processor.process_chunk(&chunk, &mut filtered_samples),
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }

                if recorder.stop().is_ok() {
                    while let Ok(chunk) = chunk_rx.try_recv() {
                        processor.process_chunk(&chunk, &mut filtered_samples);
                    }
                }

                let _ = tx.send(filtered_samples);
            }
            Err(err) => {
                let _ = startup_tx.send(Err(err.to_string()));
            }
        }
    });

    match startup_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            let _ = thread.join();
            return Err(err);
        }
        Err(_) => {
            let _ = thread.join();
            return Err("audio recorder startup timed out".to_string());
        }
    }

    Ok(CaptureHandle {
        stop_flag,
        samples_rx: rx,
        thread: Some(thread),
    })
}

// ---------------------------------------------------------------------------
// RecordingOrchestrator
// ---------------------------------------------------------------------------

/// Coordinates the push-to-talk recording flow.
///
/// This struct is Send + Sync safe (audio capture runs on a separate thread).
pub struct RecordingOrchestrator {
    state: RecordingState,
    capture: Option<CaptureHandle>,
    transcriber: Option<Arc<Mutex<Transcriber>>>,
    model_path: Option<PathBuf>,
    injection_mode: InjectionMode,
    capture_settings: CaptureSettings,
}

impl RecordingOrchestrator {
    pub fn new() -> Self {
        Self {
            state: RecordingState::Idle,
            capture: None,
            transcriber: None,
            model_path: None,
            injection_mode: InjectionMode::Clipboard,
            capture_settings: CaptureSettings::default(),
        }
    }

    pub fn state(&self) -> RecordingState {
        self.state
    }

    pub fn is_model_loaded(&self) -> bool {
        self.transcriber.is_some()
    }

    pub fn set_injection_mode(&mut self, mode: InjectionMode) {
        self.injection_mode = mode;
    }

    pub fn set_capture_preferences(
        &mut self,
        vad_enabled: bool,
        pre_roll_ms: u32,
        silence_stop_ms: u32,
    ) {
        self.capture_settings = CaptureSettings {
            vad_enabled,
            pre_roll_ms,
            silence_stop_ms,
        };
    }

    pub fn set_loaded_transcriber(&mut self, transcriber: Transcriber) {
        self.transcriber = Some(Arc::new(Mutex::new(transcriber)));
    }

    pub fn clear_model(&mut self) {
        self.transcriber = None;
        self.model_path = None;
    }

    pub fn transcriber_handle(&self) -> Result<Arc<Mutex<Transcriber>>, String> {
        self.transcriber
            .as_ref()
            .cloned()
            .ok_or_else(|| "whisper model not loaded".to_string())
    }

    /// Set the path to the whisper model file. Does NOT load it yet.
    pub fn set_model_path(&mut self, path: PathBuf) {
        if self.model_path.as_ref() != Some(&path) {
            self.transcriber = None;
        }
        self.model_path = Some(path);
    }

    /// Begin audio capture on a dedicated thread.
    pub fn start_recording(&mut self) -> Result<(), String> {
        if self.state != RecordingState::Idle {
            return Err(format!("cannot start recording in {:?} state", self.state));
        }

        let handle = spawn_capture_thread(self.capture_settings)?;
        self.capture = Some(handle);
        self.state = RecordingState::Recording;
        Ok(())
    }

    /// Stop audio capture and return the recorded samples.
    pub fn stop_recording(&mut self) -> Result<Vec<i16>, String> {
        if self.state != RecordingState::Recording {
            return Err(format!("cannot stop recording in {:?} state", self.state));
        }

        let handle = self.capture.take().ok_or("no active capture")?;
        let samples = handle.stop();

        self.state = RecordingState::Processing;
        Ok(samples)
    }

    /// Inject corrected text into the active application.
    pub fn inject_text(&self, text: &str) -> Result<(), String> {
        let config = InjectionConfig {
            mode: self.injection_mode,
            ..Default::default()
        };
        let injector = TextInjector::new(config);
        injector.inject(text).map_err(|e| e.to_string())
    }

    /// Reset state back to idle.
    pub fn set_idle(&mut self) {
        self.state = RecordingState::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(amplitude: i16, len: usize) -> Vec<i16> {
        vec![amplitude; len]
    }

    #[test]
    fn chunk_processor_trims_leading_silence_and_keeps_preroll() {
        let audio = AudioConfig::default();
        let mut processor = ChunkProcessor::new(
            &audio,
            CaptureSettings {
                vad_enabled: true,
                pre_roll_ms: 120,
                silence_stop_ms: 700,
            },
        );
        let mut out = Vec::new();

        processor.process_chunk(&chunk(0, audio.chunk_samples()), &mut out);
        processor.process_chunk(&chunk(0, audio.chunk_samples()), &mut out);
        processor.process_chunk(&chunk(8_000, audio.chunk_samples()), &mut out);
        processor.process_chunk(&chunk(8_000, audio.chunk_samples()), &mut out);
        processor.process_chunk(&chunk(8_000, audio.chunk_samples()), &mut out);

        assert!(!out.is_empty());
        assert_eq!(out.len(), audio.chunk_samples() * 4);
    }

    #[test]
    fn chunk_processor_drops_trailing_silence() {
        let audio = AudioConfig::default();
        let mut processor = ChunkProcessor::new(
            &audio,
            CaptureSettings {
                vad_enabled: true,
                pre_roll_ms: 30,
                silence_stop_ms: 60,
            },
        );
        let mut out = Vec::new();

        for _ in 0..3 {
            processor.process_chunk(&chunk(9_000, audio.chunk_samples()), &mut out);
        }
        for _ in 0..4 {
            processor.process_chunk(&chunk(0, audio.chunk_samples()), &mut out);
        }

        assert_eq!(out.len(), audio.chunk_samples() * 3);
    }

    #[test]
    fn chunk_processor_passthrough_when_vad_disabled() {
        let audio = AudioConfig::default();
        let mut processor = ChunkProcessor::new(
            &audio,
            CaptureSettings {
                vad_enabled: false,
                pre_roll_ms: 0,
                silence_stop_ms: 0,
            },
        );
        let mut out = Vec::new();

        processor.process_chunk(&chunk(0, audio.chunk_samples()), &mut out);
        processor.process_chunk(&chunk(5_000, audio.chunk_samples()), &mut out);

        assert_eq!(out.len(), audio.chunk_samples() * 2);
    }
}
