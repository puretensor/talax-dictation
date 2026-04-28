/// Energy-based Voice Activity Detection.
///
/// Uses RMS energy of audio chunks to distinguish speech from silence.
/// An adaptive noise floor tracks the ambient level, and a configurable
/// threshold above that floor triggers the Speaking state. State changes
/// are smoothed: N consecutive frames must agree before a transition fires.

/// Current state of the voice activity detector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadState {
    /// Silence detected (below energy threshold).
    Silence,
    /// Active speech detected (above energy threshold).
    Speaking,
    /// Transitioning between states (within the smoothing window).
    Transition,
}

/// Configuration for the VAD.
#[derive(Debug, Clone)]
pub struct VadConfig {
    /// Minimum RMS energy to consider as speech, expressed as an absolute
    /// threshold on the i16 sample range (0..32767). If the adaptive floor
    /// plus this value is exceeded, the frame is "active".
    pub energy_threshold: f64,
    /// How many consecutive frames must agree before the state actually
    /// changes from Speaking to Silence (or vice-versa).
    pub smoothing_frames: u32,
    /// Exponential moving average coefficient for the noise floor estimate.
    /// Closer to 1.0 = slower adaptation. Typical: 0.995.
    pub noise_floor_alpha: f64,
    /// Initial noise floor estimate. Will be overwritten quickly by
    /// the adaptive tracker.
    pub initial_noise_floor: f64,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            energy_threshold: 500.0,
            smoothing_frames: 3,
            noise_floor_alpha: 0.995,
            initial_noise_floor: 100.0,
        }
    }
}

/// Voice Activity Detector.
pub struct VoiceActivityDetector {
    config: VadConfig,
    /// Confirmed state (after smoothing).
    state: VadState,
    /// Candidate state that consecutive frames are voting for.
    candidate: VadState,
    /// How many consecutive frames have voted for `candidate`.
    candidate_count: u32,
    /// Adaptive noise floor (RMS of ambient).
    noise_floor: f64,
}

impl VoiceActivityDetector {
    pub fn new(config: VadConfig) -> Self {
        let noise_floor = config.initial_noise_floor;
        Self {
            config,
            state: VadState::Silence,
            candidate: VadState::Silence,
            candidate_count: 0,
            noise_floor,
        }
    }

    /// Create a detector with default settings.
    pub fn with_defaults() -> Self {
        Self::new(VadConfig::default())
    }

    /// Process one chunk of PCM i16 samples and return the current VAD state.
    pub fn process_chunk(&mut self, samples: &[i16]) -> VadState {
        if samples.is_empty() {
            return self.state;
        }

        let rms = Self::rms_energy(samples);
        let frame_active = rms > self.noise_floor + self.config.energy_threshold;

        // Update adaptive noise floor only during confirmed silence.
        if self.state == VadState::Silence && !frame_active {
            let alpha = self.config.noise_floor_alpha;
            self.noise_floor = alpha * self.noise_floor + (1.0 - alpha) * rms;
        }

        let proposed = if frame_active {
            VadState::Speaking
        } else {
            VadState::Silence
        };

        if proposed == self.state {
            // Agreeing with current state resets the candidate counter.
            self.candidate = self.state;
            self.candidate_count = 0;
            self.state
        } else if proposed == self.candidate {
            self.candidate_count += 1;
            if self.candidate_count >= self.config.smoothing_frames {
                self.state = proposed;
                self.candidate_count = 0;
                self.state
            } else {
                VadState::Transition
            }
        } else {
            // New candidate direction -- start counting from 1.
            self.candidate = proposed;
            self.candidate_count = 1;
            if self.config.smoothing_frames <= 1 {
                self.state = proposed;
                self.state
            } else {
                VadState::Transition
            }
        }
    }

    /// Current confirmed state.
    pub fn state(&self) -> VadState {
        self.state
    }

    /// Current noise floor estimate.
    pub fn noise_floor(&self) -> f64 {
        self.noise_floor
    }

    /// Compute the RMS energy of a slice of i16 samples.
    pub fn rms_energy(samples: &[i16]) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
        (sum_sq / samples.len() as f64).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: generate a constant-amplitude chunk.
    fn constant_chunk(amplitude: i16, len: usize) -> Vec<i16> {
        vec![amplitude; len]
    }

    #[test]
    fn rms_of_silence_is_zero() {
        assert_eq!(VoiceActivityDetector::rms_energy(&[0; 480]), 0.0);
    }

    #[test]
    fn rms_of_constant_signal() {
        let samples = constant_chunk(1000, 480);
        let rms = VoiceActivityDetector::rms_energy(&samples);
        assert!((rms - 1000.0).abs() < 0.01);
    }

    #[test]
    fn rms_of_alternating_signal() {
        // +A, -A alternating should still have RMS = A.
        let mut samples = vec![0i16; 480];
        for (i, s) in samples.iter_mut().enumerate() {
            *s = if i % 2 == 0 { 5000 } else { -5000 };
        }
        let rms = VoiceActivityDetector::rms_energy(&samples);
        assert!((rms - 5000.0).abs() < 0.01);
    }

    #[test]
    fn starts_in_silence() {
        let vad = VoiceActivityDetector::with_defaults();
        assert_eq!(vad.state(), VadState::Silence);
    }

    #[test]
    fn silence_stays_silent() {
        let mut vad = VoiceActivityDetector::with_defaults();
        // Feed low-energy frames.
        for _ in 0..10 {
            let state = vad.process_chunk(&constant_chunk(50, 480));
            assert_ne!(state, VadState::Speaking);
        }
        assert_eq!(vad.state(), VadState::Silence);
    }

    #[test]
    fn loud_signal_transitions_to_speaking() {
        let config = VadConfig {
            energy_threshold: 500.0,
            smoothing_frames: 3,
            noise_floor_alpha: 0.995,
            initial_noise_floor: 0.0,
        };
        let mut vad = VoiceActivityDetector::new(config);

        // First loud frame starts the candidate.
        let s1 = vad.process_chunk(&constant_chunk(5000, 480));
        assert_eq!(s1, VadState::Transition);

        // Second loud frame continues.
        let s2 = vad.process_chunk(&constant_chunk(5000, 480));
        assert_eq!(s2, VadState::Transition);

        // Third loud frame meets the smoothing threshold.
        let s3 = vad.process_chunk(&constant_chunk(5000, 480));
        assert_eq!(s3, VadState::Speaking);
    }

    #[test]
    fn speaking_to_silence_transition() {
        let config = VadConfig {
            energy_threshold: 500.0,
            smoothing_frames: 2,
            noise_floor_alpha: 0.995,
            initial_noise_floor: 0.0,
        };
        let mut vad = VoiceActivityDetector::new(config);

        // Get into Speaking state.
        for _ in 0..5 {
            vad.process_chunk(&constant_chunk(5000, 480));
        }
        assert_eq!(vad.state(), VadState::Speaking);

        // First silent frame starts transition.
        let s1 = vad.process_chunk(&constant_chunk(0, 480));
        assert_eq!(s1, VadState::Transition);

        // Second silent frame completes it.
        let s2 = vad.process_chunk(&constant_chunk(0, 480));
        assert_eq!(s2, VadState::Silence);
    }

    #[test]
    fn interrupted_transition_resets() {
        let config = VadConfig {
            energy_threshold: 500.0,
            smoothing_frames: 3,
            noise_floor_alpha: 0.995,
            initial_noise_floor: 0.0,
        };
        let mut vad = VoiceActivityDetector::new(config);

        // Two loud frames (not enough for smoothing_frames=3).
        vad.process_chunk(&constant_chunk(5000, 480));
        vad.process_chunk(&constant_chunk(5000, 480));

        // Interrupted by silence -- should reset candidate.
        let s = vad.process_chunk(&constant_chunk(0, 480));
        assert_eq!(vad.state(), VadState::Silence);
        // The return can be Silence (matching confirmed) or Transition
        // depending on the candidate flip. State must be Silence.
        assert_ne!(s, VadState::Speaking);
    }

    #[test]
    fn noise_floor_adapts_during_silence() {
        let config = VadConfig {
            energy_threshold: 500.0,
            smoothing_frames: 3,
            noise_floor_alpha: 0.9,
            initial_noise_floor: 0.0,
        };
        let mut vad = VoiceActivityDetector::new(config);

        // Feed moderate "ambient" that is below threshold.
        for _ in 0..50 {
            vad.process_chunk(&constant_chunk(200, 480));
        }

        // Noise floor should have moved toward 200.
        assert!(vad.noise_floor() > 150.0, "noise floor should adapt upward");
    }

    #[test]
    fn smoothing_frames_one_gives_instant_switch() {
        let config = VadConfig {
            energy_threshold: 500.0,
            smoothing_frames: 1,
            noise_floor_alpha: 0.995,
            initial_noise_floor: 0.0,
        };
        let mut vad = VoiceActivityDetector::new(config);

        let s = vad.process_chunk(&constant_chunk(5000, 480));
        assert_eq!(s, VadState::Speaking);
    }

    #[test]
    fn empty_chunk_returns_current_state() {
        let mut vad = VoiceActivityDetector::with_defaults();
        assert_eq!(vad.process_chunk(&[]), VadState::Silence);
    }
}
