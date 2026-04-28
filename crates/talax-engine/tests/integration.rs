//! Integration and unit tests for talax-engine.
//!
//! Covers all five subsystems:
//!   1. Full correction pipeline (dictionary -> n-gram -> heuristic)
//!   2. Dictionary corrector (word boundary, case preservation, longest-match)
//!   3. N-gram corrector (training, save/load roundtrip, correction behavior)
//!   4. Database (sessions, corrections, patterns, auto-apply, domain context, stats)
//!   5. Profile manager (create, list, delete, clone, reset)

use talax_engine::db::Database;
use talax_engine::pipeline::CorrectionPipeline;
use talax_engine::pipeline::dict_corrector::DictionaryCorrector;
use talax_engine::pipeline::ngram_corrector::NgramCorrector;
use talax_engine::profile::ProfileManager;

// ---------------------------------------------------------------------------
// Helper: file-backed test database with raw SQL access for setup.
// ---------------------------------------------------------------------------

/// Wraps a file-backed Database so tests can execute raw SQL for setup
/// via a second rusqlite connection to the same file.
struct TestDb {
    _tmp: tempfile::TempDir,
    path: std::path::PathBuf,
}

impl TestDb {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.db");
        // Create the database to initialize schema.
        let _db = Database::open(&path).unwrap();
        drop(_db);
        Self { _tmp: tmp, path }
    }

    /// Open a Database handle through the public API.
    fn open(&self) -> Database {
        Database::open(&self.path).unwrap()
    }

    /// Execute raw SQL for test setup via a separate rusqlite connection.
    fn exec_sql(&self, sql: &str) {
        let conn = rusqlite::Connection::open(&self.path).unwrap();
        conn.execute_batch(sql).unwrap();
    }
}

// ===========================================================================
// 1. Pipeline integration tests
// ===========================================================================

#[test]
fn pipeline_applies_auto_corrections_from_db() {
    let tdb = TestDb::new();
    // Insert a pattern with freq=10. After confidence recompute:
    // confidence = 10 / (10 + 0 + 1) = 0.909 > 0.85, and freq >= 3.
    tdb.exec_sql(
        "INSERT INTO correction_patterns (original, corrected, frequency, confidence, auto_apply)
         VALUES ('gdp', 'GCP', 10, 0.95, 1)",
    );

    let db = tdb.open();
    let mut pipeline = CorrectionPipeline::new();
    pipeline.reload(&db);

    let result = pipeline.process("the gdp is rising");
    assert_eq!(result.corrected, "the GCP is rising");
    assert!(!result.changes.is_empty());
    assert!(result.changes.iter().any(|c| c.layer == "dictionary"));
}

#[test]
fn pipeline_ignores_low_frequency_patterns() {
    let tdb = TestDb::new();
    // freq=1 => after rebuild, confidence = 1/(1+0+1) = 0.5, and freq < 3.
    // So auto_apply should be 0 after any rebuild.
    tdb.exec_sql(
        "INSERT INTO correction_patterns (original, corrected, frequency, confidence, auto_apply)
         VALUES ('gdp', 'GCP', 1, 0.5, 0)",
    );

    let db = tdb.open();
    let mut pipeline = CorrectionPipeline::new();
    pipeline.reload(&db);

    let result = pipeline.process("the gdp is rising");
    assert_eq!(result.corrected, "the gdp is rising");
    assert!(result.changes.is_empty());
}

#[test]
fn pipeline_processes_all_three_layers_in_order() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx_path = tmp.path().join("domain_context.json");
    std::fs::write(
        &ctx_path,
        r#"{"all_proper_nouns": ["Kubernetes"], "known_accent_patterns": {"kubernates": "Kubernetes"}}"#,
    )
    .unwrap();

    let tdb = TestDb::new();
    tdb.exec_sql(
        "INSERT INTO correction_patterns (original, corrected, frequency, confidence, auto_apply)
         VALUES ('gdp', 'GCP', 10, 0.95, 1)",
    );

    let mut db = tdb.open();
    db.set_domain_context_path(ctx_path);

    let mut pipeline = CorrectionPipeline::new();
    pipeline.reload(&db);

    // 7 words (>5), ngram untrained => heuristic layer fires.
    let result = pipeline.process("the gdp cluster runs kubernates pods daily");

    // Dictionary layer: "gdp" -> "GCP"
    assert!(
        result.corrected.contains("GCP"),
        "dictionary should fix gdp->GCP: {}",
        result.corrected
    );
    // Heuristic layer: "kubernates" -> "Kubernetes" via accent_patterns
    assert!(
        result.corrected.contains("Kubernetes"),
        "heuristic should fix kubernates->Kubernetes: {}",
        result.corrected
    );
    // Both layers reported.
    assert!(result.layers_used.contains(&"dictionary".to_string()));
    assert!(result.layers_used.contains(&"heuristic".to_string()));
}

#[test]
fn pipeline_reload_picks_up_new_patterns() {
    let tdb = TestDb::new();
    let db = tdb.open();

    let mut pipeline = CorrectionPipeline::new();
    pipeline.reload(&db);

    // No patterns yet -- no corrections.
    let result = pipeline.process("the gdp is rising");
    assert_eq!(result.corrected, "the gdp is rising");

    // Add a qualifying pattern.
    tdb.exec_sql(
        "INSERT INTO correction_patterns (original, corrected, frequency, confidence, auto_apply)
         VALUES ('gdp', 'GCP', 10, 0.95, 1)",
    );

    // Reload picks up the new pattern.
    let db2 = tdb.open();
    pipeline.reload(&db2);
    let result = pipeline.process("the gdp is rising");
    assert_eq!(result.corrected, "the GCP is rising");
}

// ===========================================================================
// 2. Dictionary corrector tests
// ===========================================================================

#[test]
fn dict_word_boundary_matching() {
    let tdb = TestDb::new();
    tdb.exec_sql(
        "INSERT INTO correction_patterns (original, corrected, frequency, confidence, auto_apply)
         VALUES ('tea', 'T', 10, 0.95, 1)",
    );

    let db = tdb.open();
    let mut corrector = DictionaryCorrector::new();
    corrector.reload(&db);

    let (result, changes) = corrector.apply("GitHub is a digital platform for tea drinkers");
    // "tea" inside "GitHub" and "digital" should NOT match.
    assert!(
        result.contains("GitHub"),
        "GitHub should be untouched: {result}"
    );
    assert!(
        result.contains("digital"),
        "digital should be untouched: {result}"
    );
    // Only the standalone "tea" should be corrected.
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].original.to_lowercase(), "tea");
}

#[test]
fn dict_case_preservation_all_caps() {
    let tdb = TestDb::new();
    tdb.exec_sql(
        "INSERT INTO correction_patterns (original, corrected, frequency, confidence, auto_apply)
         VALUES ('gdp', 'GCP', 10, 0.95, 1)",
    );

    let db = tdb.open();
    let mut corrector = DictionaryCorrector::new();
    corrector.reload(&db);

    // All-caps "GDP" => case_match returns "GCP" (all-caps preserved).
    let (result, _) = corrector.apply("GDP is important");
    assert!(result.starts_with("GCP"), "GDP should become GCP: {result}");
}

#[test]
fn dict_case_preservation_lowercase() {
    let tdb = TestDb::new();
    tdb.exec_sql(
        "INSERT INTO correction_patterns (original, corrected, frequency, confidence, auto_apply)
         VALUES ('gdp', 'GCP', 10, 0.95, 1)",
    );

    let db = tdb.open();
    let mut corrector = DictionaryCorrector::new();
    corrector.reload(&db);

    // Lowercase "gdp" => case_match returns raw "GCP".
    let (result, _) = corrector.apply("gdp is important");
    assert!(result.starts_with("GCP"), "gdp should become GCP: {result}");
}

#[test]
fn dict_context_constrained_pattern_applies_with_token_context() {
    let tdb = TestDb::new();
    tdb.exec_sql(
        "INSERT INTO correction_patterns (original, corrected, frequency, confidence, context_before, auto_apply)
         VALUES ('v', 'V', 10, 0.95, 'node', 1)",
    );

    let db = tdb.open();
    let mut corrector = DictionaryCorrector::new();
    corrector.reload(&db);

    let (result, changes) = corrector.apply("node v is ready");
    assert!(
        changes.iter().any(|change| change.corrected == "V"),
        "context-constrained pattern should fire with token context"
    );
    assert_eq!(result, "node V is ready");
}

#[test]
fn dict_longest_match_first() {
    let tdb = TestDb::new();
    tdb.exec_sql(
        "INSERT INTO correction_patterns (original, corrected, frequency, confidence, auto_apply)
         VALUES ('pure', 'Pure', 10, 0.95, 1)",
    );
    tdb.exec_sql(
        "INSERT INTO correction_patterns (original, corrected, frequency, confidence, auto_apply)
         VALUES ('projectalpha', 'ProjectAlpha', 10, 0.95, 1)",
    );

    let db = tdb.open();
    let mut corrector = DictionaryCorrector::new();
    corrector.reload(&db);

    let (result, changes) = corrector.apply("projectalpha is great");
    // The longer "projectalpha"->"ProjectAlpha" should match first.
    assert!(
        result.contains("ProjectAlpha"),
        "expected ProjectAlpha: {result}"
    );
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].corrected, "ProjectAlpha");
}

#[test]
fn dict_no_rules_no_changes() {
    let corrector = DictionaryCorrector::new();
    let (result, changes) = corrector.apply("hello world");
    assert_eq!(result, "hello world");
    assert!(changes.is_empty());
}

// ===========================================================================
// 3. N-gram corrector tests
// ===========================================================================

#[test]
fn ngram_train_populates_vocabulary() {
    let mut corrector = NgramCorrector::new();
    assert!(!corrector.is_trained());

    corrector.train(&[
        "the quick brown fox jumps over the lazy dog".to_string(),
        "the quick brown fox runs through the field".to_string(),
    ]);

    assert!(corrector.is_trained());
    let vocab = corrector.vocab();
    assert!(vocab.contains("fox"));
    assert!(vocab.contains("quick"));
    assert!(vocab.contains("brown"));
    assert!(vocab.contains("lazy"));
    assert!(vocab.contains("field"));
    assert!(vocab.contains("</s>"));
}

#[test]
fn ngram_save_load_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let model_path = tmp.path().join("ngram.bin");

    let mut corrector = NgramCorrector::new();
    corrector.train(&[
        "the server runs on kubernetes with docker containers".to_string(),
        "the server deploys to kubernetes cluster nodes".to_string(),
        "kubernetes orchestrates the docker containers efficiently".to_string(),
    ]);

    assert!(corrector.is_trained());
    let vocab_before: Vec<String> = {
        let mut v: Vec<String> = corrector.vocab().iter().cloned().collect();
        v.sort();
        v
    };

    corrector.save(&model_path).unwrap();
    assert!(model_path.exists());

    // Load into a fresh corrector.
    let mut loaded = NgramCorrector::new();
    loaded.load_from(&model_path);

    assert!(loaded.is_trained());
    let vocab_after: Vec<String> = {
        let mut v: Vec<String> = loaded.vocab().iter().cloned().collect();
        v.sort();
        v
    };

    assert_eq!(
        vocab_before, vocab_after,
        "vocabulary should survive save/load roundtrip"
    );
}

#[test]
fn ngram_with_model_path_loads_on_construction() {
    let tmp = tempfile::tempdir().unwrap();
    let model_path = tmp.path().join("ngram.bin");

    let mut corrector = NgramCorrector::new();
    corrector.train(&["hello world foo bar".to_string()]);
    corrector.save(&model_path).unwrap();

    let loaded = NgramCorrector::with_model_path(model_path);
    assert!(loaded.is_trained());
    assert!(loaded.vocab().contains("hello"));
}

#[test]
fn ngram_high_probability_words_left_alone() {
    let mut corrector = NgramCorrector::new();
    let corpus: Vec<String> = (0..20)
        .map(|_| "the quick brown fox jumps over the lazy dog".to_string())
        .collect();
    corrector.train(&corpus);

    let (result, changes) = corrector.apply("the quick brown fox");
    assert_eq!(result, "the quick brown fox");
    assert!(
        changes.is_empty(),
        "high-probability text should have no changes"
    );
}

#[test]
fn ngram_untrained_produces_no_changes() {
    let corrector = NgramCorrector::new();
    assert!(!corrector.is_trained());

    let (result, changes) = corrector.apply("some random text here");
    assert_eq!(result, "some random text here");
    assert!(changes.is_empty());
}

#[test]
fn ngram_load_nonexistent_path_stays_untrained() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("does_not_exist.bin");

    let loaded = NgramCorrector::with_model_path(missing);
    assert!(
        !loaded.is_trained(),
        "loading from missing file should leave model untrained"
    );
}

// ===========================================================================
// 4. Database tests
// ===========================================================================

#[test]
fn db_save_corrections_creates_patterns() {
    let db = Database::open_memory().unwrap();

    db.create_session("sess1", "/tmp/audio.wav", 10.0).unwrap();
    db.add_segments(
        "sess1",
        &[(0.0, 5.0, "the gdp is rising"), (5.0, 10.0, "hello world")],
    )
    .unwrap();

    db.save_corrections("sess1", &[(0, "the GCP is rising")])
        .unwrap();

    let patterns = db.get_all_patterns();
    assert!(
        patterns
            .iter()
            .any(|p| p.original == "gdp" && p.corrected == "GCP"),
        "expected gdp->GCP pattern, got: {:?}",
        patterns
    );
}

#[test]
fn db_save_corrections_increments_frequency() {
    let db = Database::open_memory().unwrap();

    for i in 0..4 {
        let sid = format!("sess{i}");
        db.create_session(&sid, "/tmp/audio.wav", 5.0).unwrap();
        db.add_segments(&sid, &[(0.0, 5.0, "the gdp")]).unwrap();
        db.save_corrections(&sid, &[(0, "the GCP")]).unwrap();
    }

    let patterns = db.get_all_patterns();
    let matching: Vec<_> = patterns
        .iter()
        .filter(|p| p.original == "gdp" && p.corrected == "GCP")
        .collect();
    // The upsert uses empty-string context columns so ON CONFLICT triggers,
    // incrementing frequency on a single row.
    assert_eq!(
        matching.len(),
        1,
        "should have exactly 1 pattern row with incremented frequency: {:?}",
        matching
    );
    assert_eq!(
        matching[0].frequency, 4,
        "frequency should be 4 after 4 corrections"
    );
}

#[test]
fn db_auto_apply_rebuild_via_save_corrections() {
    // Repeated accepted substitutions increment frequency on a single row.
    // Auto-apply now requires frequency >= 3 and confidence >= 0.75.
    let db = Database::open_memory().unwrap();

    for i in 0..3 {
        let sid = format!("sess{i}");
        db.create_session(&sid, "/tmp/audio.wav", 5.0).unwrap();
        db.add_segments(&sid, &[(0.0, 5.0, "teh quick")]).unwrap();
        db.save_corrections(&sid, &[(0, "the quick")]).unwrap();
    }

    // freq=3, confidence=3/(3+0+1)=0.75 => auto-apply.
    let auto = db.get_auto_corrections();
    assert!(
        auto.iter()
            .any(|p| p.original == "teh" && p.corrected == "the"),
        "teh->the should be auto-apply after 3 corrections: {:?}",
        auto
    );

    // Verify it's a single row with freq=3.
    let all = db.get_all_patterns();
    let matching: Vec<_> = all
        .iter()
        .filter(|p| p.original == "teh" && p.corrected == "the")
        .collect();
    assert_eq!(matching.len(), 1, "should have 1 pattern row");
    assert_eq!(matching[0].frequency, 3);
}

#[test]
fn db_auto_apply_rebuild_with_file_backed_db() {
    // Use the TestDb (file-backed) to manually set a high-frequency pattern,
    // then verify that rebuild_auto_apply (triggered by save_corrections)
    // correctly sets auto_apply=1.
    let tdb = TestDb::new();
    tdb.exec_sql(
        "INSERT INTO correction_patterns (original, corrected, frequency, confidence, context_before, context_after)
         VALUES ('teh', 'the', 10, 0.5, '', '')",
    );

    // Trigger rebuild_auto_apply by calling save_corrections.
    let db = tdb.open();
    db.create_session("trigger_sess", "/tmp/a.wav", 1.0)
        .unwrap();
    db.add_segments("trigger_sess", &[(0.0, 1.0, "dummy text")])
        .unwrap();
    db.save_corrections("trigger_sess", &[(0, "dummy text")])
        .unwrap();

    let auto = db.get_auto_corrections();
    assert!(
        auto.iter()
            .any(|p| p.original == "teh" && p.corrected == "the"),
        "teh->the with freq=10 should be auto-apply after rebuild: {:?}",
        auto
    );
}

#[test]
fn db_auto_apply_not_set_below_threshold() {
    let db = Database::open_memory().unwrap();

    // Only 2 corrections -- freq=2 which is < 3.
    for i in 0..2 {
        let sid = format!("sess{i}");
        db.create_session(&sid, "/tmp/audio.wav", 5.0).unwrap();
        db.add_segments(&sid, &[(0.0, 5.0, "teh quick")]).unwrap();
        db.save_corrections(&sid, &[(0, "the quick")]).unwrap();
    }

    let auto = db.get_auto_corrections();
    assert!(
        auto.is_empty(),
        "with only 2 corrections, nothing should be auto-apply: {:?}",
        auto
    );
}

#[test]
fn db_domain_context_from_json() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx_path = tmp.path().join("domain_context.json");

    std::fs::write(
        &ctx_path,
        r#"{
            "all_proper_nouns": ["ProjectAlpha", "Kubernetes", "NVIDIA"],
            "known_accent_patterns": {
                "kubernates": "Kubernetes",
                "invida": "NVIDIA"
            }
        }"#,
    )
    .unwrap();

    let mut db = Database::open_memory().unwrap();
    db.set_domain_context_path(ctx_path);

    let ctx = db.get_domain_context();
    assert_eq!(ctx.proper_nouns.len(), 3);
    assert_eq!(
        ctx.proper_nouns.get("projectalpha").unwrap(),
        "ProjectAlpha"
    );
    assert_eq!(ctx.proper_nouns.get("kubernetes").unwrap(), "Kubernetes");
    assert_eq!(ctx.proper_nouns.get("nvidia").unwrap(), "NVIDIA");

    assert_eq!(ctx.accent_patterns.len(), 2);
    assert_eq!(ctx.accent_patterns.get("kubernates").unwrap(), "Kubernetes");
    assert_eq!(ctx.accent_patterns.get("invida").unwrap(), "NVIDIA");
}

#[test]
fn db_domain_context_returns_default_without_path() {
    let db = Database::open_memory().unwrap();
    let ctx = db.get_domain_context();
    assert!(ctx.proper_nouns.is_empty());
    assert!(ctx.accent_patterns.is_empty());
}

#[test]
fn db_domain_context_returns_default_for_missing_file() {
    let mut db = Database::open_memory().unwrap();
    db.set_domain_context_path(std::path::PathBuf::from("/nonexistent/path.json"));
    let ctx = db.get_domain_context();
    assert!(ctx.proper_nouns.is_empty());
    assert!(ctx.accent_patterns.is_empty());
}

#[test]
fn db_statistics() {
    let db = Database::open_memory().unwrap();

    let stats = db.get_stats();
    assert_eq!(stats.session_count, 0);
    assert_eq!(stats.pattern_count, 0);
    assert_eq!(stats.auto_apply_count, 0);

    db.create_session("s1", "/tmp/a.wav", 5.0).unwrap();
    db.create_session("s2", "/tmp/b.wav", 3.0).unwrap();

    // Build patterns through save_corrections to get real stats.
    db.add_segments("s1", &[(0.0, 5.0, "teh quick")]).unwrap();
    db.save_corrections("s1", &[(0, "the quick")]).unwrap();

    let stats = db.get_stats();
    assert_eq!(stats.session_count, 2);
    assert_eq!(stats.pattern_count, 1);
    // freq=1 < 3, so auto_apply_count should be 0.
    assert_eq!(stats.auto_apply_count, 0);
}

#[test]
fn db_segments_counted_on_session() {
    let db = Database::open_memory().unwrap();
    db.create_session("s1", "/tmp/a.wav", 10.0).unwrap();
    db.add_segments(
        "s1",
        &[
            (0.0, 3.0, "segment one"),
            (3.0, 6.0, "segment two"),
            (6.0, 10.0, "segment three"),
        ],
    )
    .unwrap();

    let stats = db.get_stats();
    assert_eq!(stats.session_count, 1);
}

// ===========================================================================
// 5. Profile manager tests
// ===========================================================================

#[test]
fn profile_create_list_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = ProfileManager::new(tmp.path().to_path_buf());

    assert!(pm.list_profiles().is_empty());

    pm.create_profile("english").unwrap();
    pm.create_profile("icelandic").unwrap();

    let mut profiles = pm.list_profiles();
    profiles.sort();
    assert_eq!(profiles, vec!["english", "icelandic"]);

    // Each profile has the expected files.
    assert!(tmp.path().join("english/corrections.db").exists());
    assert!(tmp.path().join("english/profile.toml").exists());
    assert!(tmp.path().join("english/domain_context.json").exists());

    pm.delete_profile("icelandic").unwrap();
    let profiles = pm.list_profiles();
    assert_eq!(profiles, vec!["english"]);
}

#[test]
fn profile_create_duplicate_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = ProfileManager::new(tmp.path().to_path_buf());

    pm.create_profile("test").unwrap();
    let result = pm.create_profile("test");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("already exists"));
}

#[test]
fn profile_delete_nonexistent_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = ProfileManager::new(tmp.path().to_path_buf());

    let result = pm.delete_profile("ghost");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("does not exist"));
}

#[test]
fn profile_clone_and_independence() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = ProfileManager::new(tmp.path().to_path_buf());

    pm.create_profile("original").unwrap();
    let orig_db = pm.open_db("original").unwrap();
    orig_db.create_session("s1", "/tmp/a.wav", 5.0).unwrap();
    drop(orig_db);

    pm.clone_profile("original", "cloned").unwrap();

    // Clone should have the original's data.
    let cloned_db = pm.open_db("cloned").unwrap();
    assert_eq!(
        cloned_db.get_stats().session_count,
        1,
        "clone should have the original's session"
    );

    // Modify the clone.
    cloned_db.create_session("s2", "/tmp/b.wav", 3.0).unwrap();
    drop(cloned_db);

    // Original should be unaffected.
    let orig_db = pm.open_db("original").unwrap();
    assert_eq!(
        orig_db.get_stats().session_count,
        1,
        "original should be unaffected by clone modifications"
    );

    // Clone should now have 2.
    let cloned_db = pm.open_db("cloned").unwrap();
    assert_eq!(cloned_db.get_stats().session_count, 2);
}

#[test]
fn profile_clone_updates_name_in_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = ProfileManager::new(tmp.path().to_path_buf());

    pm.create_profile("source").unwrap();
    pm.clone_profile("source", "target").unwrap();

    let toml = std::fs::read_to_string(tmp.path().join("target/profile.toml")).unwrap();
    assert!(
        toml.contains("name = \"target\""),
        "cloned profile.toml should have updated name: {toml}"
    );
    assert!(
        !toml.contains("name = \"source\""),
        "cloned profile.toml should not retain source name: {toml}"
    );
}

#[test]
fn profile_reset_clears_data() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = ProfileManager::new(tmp.path().to_path_buf());

    pm.create_profile("resettable").unwrap();
    let db = pm.open_db("resettable").unwrap();
    db.create_session("s1", "/tmp/a.wav", 5.0).unwrap();
    db.add_segments("s1", &[(0.0, 5.0, "hello world")]).unwrap();
    assert_eq!(db.get_stats().session_count, 1);
    drop(db);

    pm.reset_profile("resettable").unwrap();

    let db = pm.open_db("resettable").unwrap();
    assert_eq!(db.get_stats().session_count, 0);
    assert_eq!(db.get_stats().pattern_count, 0);
}

#[test]
fn profile_default_creation_has_all_files() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = ProfileManager::new(tmp.path().to_path_buf());

    let dir = pm.create_profile("default").unwrap();
    assert!(dir.join("corrections.db").exists());
    assert!(dir.join("profile.toml").exists());
    assert!(dir.join("domain_context.json").exists());

    let toml = std::fs::read_to_string(dir.join("profile.toml")).unwrap();
    assert!(toml.contains("name = \"default\""));

    let ctx: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("domain_context.json")).unwrap())
            .unwrap();
    assert!(ctx.get("all_proper_nouns").is_some());
}

#[test]
fn profile_active_tracking() {
    let tmp = tempfile::tempdir().unwrap();
    let mut pm = ProfileManager::new(tmp.path().to_path_buf());

    assert!(pm.active().is_none());

    pm.set_active("english");
    assert_eq!(pm.active(), Some("english"));

    pm.set_active("icelandic");
    assert_eq!(pm.active(), Some("icelandic"));
}
