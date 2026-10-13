//! Contract for the engine features ptdictd relies on through `talax-correct`.
//!
//! ptdictd (the HALbook on-device dictation daemon) runs TalaX's correction
//! pipeline on every take. These tests pin the behaviour that makes that safe
//! for a daily-driver dictation path:
//!   1. the heuristic layer never rewrites a known word ("after" -> "Apter"),
//!      never fuzzes an acronym, needs phonetic or frequency evidence for short
//!      tokens, and keeps the punctuation around the word it corrects;
//!   2. manual (taught) rules auto-apply at once and survive auto-apply
//!      rebuilds, and older databases migrate in place;
//!   3. callers can run a subset of the layers (Icelandic takes run the
//!      dictionary layer only).

use std::collections::HashSet;
use std::path::Path;

use talax_engine::db::Database;
use talax_engine::pipeline::{CorrectionPipeline, Layers};

fn write(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
}

/// A file-backed profile: corrections.db + domain_context.json + optional lexicon.
struct Profile {
    dir: tempfile::TempDir,
}

impl Profile {
    fn new(proper_nouns: &[&str], lexicon: Option<&[&str]>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let nouns: Vec<String> = proper_nouns.iter().map(|n| format!("{n:?}")).collect();
        write(
            &dir.path().join("domain_context.json"),
            &format!("{{\"all_proper_nouns\": [{}]}}", nouns.join(", ")),
        );
        if let Some(words) = lexicon {
            let mut body = String::from("# test lexicon\n");
            for w in words {
                body.push_str(w);
                body.push('\n');
            }
            write(&dir.path().join("lexicon.txt"), &body);
        }
        Self { dir }
    }

    fn db(&self) -> Database {
        let mut db = Database::open(&self.dir.path().join("corrections.db")).unwrap();
        db.set_domain_context_path(self.dir.path().join("domain_context.json"));
        db.set_lexicon_path(self.dir.path().join("lexicon.txt"));
        db
    }

    fn pipeline(&self) -> (Database, CorrectionPipeline) {
        let db = self.db();
        let mut p = CorrectionPipeline::new();
        p.set_ngram_model_path(self.dir.path().join("ngram.json"));
        p.try_reload(&db).unwrap();
        (db, p)
    }
}

fn correct(p: &CorrectionPipeline, text: &str) -> String {
    p.process(text).corrected
}

// ---------------------------------------------------------------------------
// 1. Heuristic guards
// ---------------------------------------------------------------------------

#[test]
fn known_words_are_never_rewritten_into_names() {
    let prof = Profile::new(&["Apter", "Tibo"], Some(&["after", "lunch", "call", "him"]));
    let (_db, p) = prof.pipeline();
    // Observed before this contract: "after" -> "Apter" (edit distance 1, score 2.0).
    assert_eq!(correct(&p, "I will call him after lunch."), "I will call him after lunch.");
    // A word that is not in the lexicon still gets the fuzzy match.
    assert_eq!(correct(&p, "email Alan Aptor today"), "email Alan Apter today");
}

#[test]
fn lexicon_file_is_read_lowercased_with_comments_ignored() {
    let prof = Profile::new(&[], Some(&["After", "  lunch  ", "# not-a-word"]));
    let words: HashSet<String> = prof.db().get_lexicon();
    assert!(words.contains("after"));
    assert!(words.contains("lunch"));
    assert!(!words.iter().any(|w| w.starts_with('#')));
}

#[test]
fn missing_lexicon_is_empty_not_an_error() {
    let prof = Profile::new(&["Tibo"], None);
    assert!(prof.db().get_lexicon().is_empty());
    let (_db, p) = prof.pipeline();
    assert_eq!(correct(&p, "ask tipo"), "ask Tibo");
}

#[test]
fn words_confirmed_in_review_count_as_known() {
    // No lexicon: the word becomes known because the user kept it in a reviewed segment.
    let prof = Profile::new(&["Apter"], None);
    let db = prof.db();
    db.create_session("s1", "", 1.0).unwrap();
    db.add_segments("s1", &[(0.0, 1.0, "see you after lunch")]).unwrap();
    db.save_corrections("s1", &[(0, "see you after lunch")]).unwrap();
    drop(db);
    let (_db, p) = prof.pipeline();
    assert_eq!(correct(&p, "see you after lunch"), "see you after lunch");
}

#[test]
fn acronyms_are_not_fuzzed_into_names() {
    let prof = Profile::new(&["vLLM", "Tibo"], Some(&["the", "is", "up"]));
    let (_db, p) = prof.pipeline();
    assert_eq!(correct(&p, "the LLM is up"), "the LLM is up");
}

#[test]
fn short_tokens_need_phonetic_or_frequency_evidence() {
    let prof = Profile::new(&["vLLM", "Tibo"], Some(&["the", "is", "up", "ask"]));
    let (_db, p) = prof.pipeline();
    // "llm" -> "vLLM" is one edit but sounds different: leave it.
    assert_eq!(correct(&p, "the llm is up"), "the llm is up");
    // "tipo" -> "Tibo" is one edit and sounds the same (TP / TP): correct it.
    assert_eq!(correct(&p, "ask tipo"), "ask Tibo");
}

#[test]
fn punctuation_around_a_corrected_word_is_kept() {
    let prof = Profile::new(&["Tibo", "OpenAI", "API"], Some(&["and", "too", "said", "the", "is", "up"]));
    let (_db, p) = prof.pipeline();
    assert_eq!(correct(&p, "Tipo, and Tipo, too."), "Tibo, and Tibo, too.");
    assert_eq!(correct(&p, "(Tipo) said"), "(Tibo) said");
    assert_eq!(correct(&p, "\"Tipo\" said"), "\"Tibo\" said");
    assert_eq!(correct(&p, "said tipo."), "said Tibo.");
    // Exact-match casing and acronyms keep their punctuation too.
    assert_eq!(correct(&p, "openai, and the api."), "OpenAI, and the API.");
}

#[test]
fn heuristic_changes_report_the_whole_token() {
    let prof = Profile::new(&["Tibo"], Some(&["too"]));
    let (_db, p) = prof.pipeline();
    let r = p.process("Tipo, too");
    let c = r.changes.iter().find(|c| c.layer == "heuristic").expect("a heuristic change");
    assert_eq!(c.original, "Tipo,");
    assert_eq!(c.corrected, "Tibo,");
}

// ---------------------------------------------------------------------------
// 2. Manual rules and migration
// ---------------------------------------------------------------------------

#[test]
fn taught_rule_applies_immediately() {
    let prof = Profile::new(&[], None);
    let db = prof.db();
    db.teach_rule("Teepo", "Tibo").unwrap();
    let (_db, p) = prof.pipeline();
    assert_eq!(correct(&p, "I asked Teepo."), "I asked Tibo.");
    assert_eq!(correct(&p, "teepo said"), "Tibo said");
}

#[test]
fn taught_multi_word_rule_applies() {
    let prof = Profile::new(&[], None);
    prof.db().teach_rule("Josh Rosen", "Joshua Rose").unwrap();
    let (_db, p) = prof.pipeline();
    assert_eq!(correct(&p, "email Josh Rosen today"), "email Joshua Rose today");
}

#[test]
fn taught_rule_survives_auto_apply_rebuild() {
    let prof = Profile::new(&[], None);
    let db = prof.db();
    db.teach_rule("Teepo", "Tibo").unwrap();
    // save_corrections rebuilds every auto_apply flag from frequency/confidence.
    db.create_session("s1", "", 1.0).unwrap();
    db.add_segments("s1", &[(0.0, 1.0, "hello world")]).unwrap();
    db.save_corrections("s1", &[(0, "hello there")]).unwrap();
    let rules = db.list_rules().unwrap();
    let r = rules.iter().find(|r| r.original == "teepo").expect("taught rule listed");
    assert!(r.manual);
    assert!(r.auto_apply);
    assert_eq!(r.corrected, "Tibo");
}

#[test]
fn teaching_twice_keeps_one_rule() {
    let prof = Profile::new(&[], None);
    let db = prof.db();
    db.teach_rule("Teepo", "Tibo").unwrap();
    db.teach_rule("teepo", "Tibo").unwrap();
    let n = db.list_rules().unwrap().iter().filter(|r| r.original == "teepo").count();
    assert_eq!(n, 1);
}

#[test]
fn forget_removes_taught_rule() {
    let prof = Profile::new(&[], None);
    let db = prof.db();
    db.teach_rule("Teepo", "Tibo").unwrap();
    db.teach_rule("Tipo", "Tibo").unwrap();
    assert_eq!(db.forget_rule("TEEPO", None).unwrap(), 1);
    assert_eq!(db.forget_rule("tipo", Some("Somebody")).unwrap(), 0);
    assert_eq!(db.forget_rule("tipo", Some("Tibo")).unwrap(), 1);
    let (_db, p) = prof.pipeline();
    assert_eq!(correct(&p, "Teepo"), "Teepo");
}

#[test]
fn learned_rules_are_listed_as_not_manual() {
    let prof = Profile::new(&[], None);
    let db = prof.db();
    for i in 0..3 {
        let id = format!("s{i}");
        db.create_session(&id, "", 1.0).unwrap();
        db.add_segments(&id, &[(0.0, 1.0, "meet teebo today")]).unwrap();
        db.save_corrections(&id, &[(0, "meet Tibo today")]).unwrap();
    }
    let rules = db.list_rules().unwrap();
    let r = rules.iter().find(|r| r.original == "teebo").expect("learned rule listed");
    assert!(!r.manual);
    assert!(r.auto_apply);
    assert_eq!(r.frequency, 3);
}

#[test]
fn database_from_before_manual_rules_migrates_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrections.db");
    {
        // The v1.13 correction_patterns table, without the `manual` column.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE correction_patterns (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                original TEXT NOT NULL,
                corrected TEXT NOT NULL,
                frequency INTEGER DEFAULT 1,
                confidence REAL DEFAULT 0.5,
                context_before TEXT,
                context_after TEXT,
                last_seen DATETIME DEFAULT CURRENT_TIMESTAMP,
                auto_apply INTEGER DEFAULT 0,
                UNIQUE(original, corrected, context_before, context_after)
             );
             INSERT INTO correction_patterns (original, corrected, frequency, confidence, context_before, context_after, auto_apply)
             VALUES ('aptor', 'Apter', 5, 0.9, '', '', 1);",
        )
        .unwrap();
    }
    let db = Database::open(&path).unwrap();
    db.teach_rule("Teepo", "Tibo").unwrap();
    let rules = db.list_rules().unwrap();
    let old = rules.iter().find(|r| r.original == "aptor").expect("old row kept");
    assert!(!old.manual);
    assert!(old.auto_apply);
    assert!(rules.iter().any(|r| r.original == "teepo" && r.manual));
    drop(db);
    // Re-opening an already migrated database is a no-op.
    Database::open(&path).unwrap();
}

// ---------------------------------------------------------------------------
// 3. Layer selection
// ---------------------------------------------------------------------------

#[test]
fn dictionary_only_skips_the_heuristic_layer() {
    let prof = Profile::new(&["Tibo"], None);
    prof.db().teach_rule("Talak", "TalaX").unwrap();
    let (_db, p) = prof.pipeline();
    let r = p.process_with("tipo and Talak", Layers::DICTIONARY_ONLY);
    assert_eq!(r.corrected, "tipo and TalaX");
    assert!(r.changes.iter().all(|c| c.layer == "dictionary"));
    // process() is process_with(ALL).
    assert_eq!(p.process_with("tipo and Talak", Layers::ALL).corrected, "Tibo and TalaX");
    assert_eq!(p.process("tipo and Talak").corrected, "Tibo and TalaX");
}

#[test]
fn layers_can_be_built_from_names() {
    let l = Layers::from_names(&["dictionary", "heuristic"]).unwrap();
    assert!(l.dictionary && l.heuristic && !l.ngram);
    assert!(Layers::from_names(&["dictionary", "bogus"]).is_err());
}
