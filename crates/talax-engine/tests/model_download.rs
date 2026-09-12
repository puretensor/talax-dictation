//! Optional network regression using the smallest catalogue model.
//! `TALAX_TEST_DOWNLOAD_DIR=/path/to/scratch cargo test -p talax-engine --test model_download -- --ignored`
use talax_engine::whisper::model_manager::ModelManager;
use talax_engine::whisper::transcriber::Transcriber;

#[test]
#[ignore = "downloads tiny.en; requires TALAX_TEST_DOWNLOAD_DIR scratch directory"]
fn official_tiny_model_download_passes_integrity_and_loads() {
    let scratch = std::env::var_os("TALAX_TEST_DOWNLOAD_DIR")
        .expect("set TALAX_TEST_DOWNLOAD_DIR to a scratch directory");
    let directory = tempfile::tempdir_in(scratch).unwrap();
    let manager = ModelManager::new(directory.path().to_path_buf()).unwrap();
    let path = manager.download("tiny.en", |_, _| {}).unwrap();
    assert!(manager.is_downloaded("tiny.en"));
    assert!(path.is_file());
    assert!(!directory.path().join("ggml-tiny.en.bin.part").exists());
    Transcriber::new(&path).expect("verified model must load in the native engine");
}
