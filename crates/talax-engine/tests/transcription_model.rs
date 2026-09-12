//! Optional real-model regression. Supply a GGML English-capable model and
//! a short English speech fixture as mono 16 kHz signed 16-bit little-endian PCM.
//! `TALAX_TEST_MODEL=... TALAX_TEST_PCM=... cargo test -p talax-engine --test transcription_model -- --ignored`
use std::path::PathBuf;
use talax_engine::whisper::transcriber::{TranscribeParams, Transcriber};

#[test]
#[ignore = "requires TALAX_TEST_MODEL and TALAX_TEST_PCM fixtures"]
fn automatic_language_detection_still_transcribes_speech() {
    let model = PathBuf::from(std::env::var_os("TALAX_TEST_MODEL").expect("set TALAX_TEST_MODEL"));
    let pcm = PathBuf::from(std::env::var_os("TALAX_TEST_PCM").expect("set TALAX_TEST_PCM"));
    let bytes = std::fs::read(pcm).unwrap();
    assert_eq!(bytes.len() % 2, 0);
    let samples: Vec<i16> = bytes
        .chunks_exact(2)
        .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
        .collect();
    let transcriber = Transcriber::new(&model).unwrap();
    // Control: the fixture must actually contain recognizable speech.
    let explicit = transcriber
        .transcribe_from_i16(
            &samples,
            &TranscribeParams {
                language: Some("en".to_string()),
                n_threads: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        !explicit.full_text.trim().is_empty(),
        "invalid speech fixture"
    );

    let automatic = transcriber
        .transcribe_from_i16(
            &samples,
            &TranscribeParams {
                n_threads: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        !automatic.segments.is_empty(),
        "auto-detection must continue to decoding"
    );
    assert!(!automatic.full_text.trim().is_empty());
}
