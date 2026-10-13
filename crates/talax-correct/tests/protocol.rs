//! Contract for the `talax-correct serve` JSON-lines protocol.
//!
//! ptdictd spawns one long-lived sidecar and writes one JSON object per line to
//! its stdin; the sidecar answers each request with exactly one JSON object per
//! line on stdout (logs go to stderr). Every response carries `"ok"`; a request
//! `"id"` is echoed back. Bad input is answered with `{"ok": false, "error": ..}`
//! and the sidecar keeps serving. EOF on stdin is a clean exit.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_talax-correct");
const TIMEOUT: Duration = Duration::from_secs(10);

struct Sidecar {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
}

impl Sidecar {
    fn start(profile: &Path, lexicon: Option<&Path>) -> Self {
        let mut cmd = Command::new(BIN);
        cmd.arg("serve").arg("--profile-dir").arg(profile);
        if let Some(l) = lexicon {
            cmd.arg("--lexicon").arg(l);
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn talax-correct");
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let stdin = child.stdin.take();
        Self { child, stdin, lines: rx }
    }

    fn raw(&mut self, line: &str) -> Value {
        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
        let reply = self.lines.recv_timeout(TIMEOUT).expect("one response line");
        serde_json::from_str(&reply).unwrap_or_else(|e| panic!("response is not JSON ({e}): {reply}"))
    }

    fn send(&mut self, req: Value) -> Value {
        self.raw(&req.to_string())
    }

    fn ok(&mut self, req: Value) -> Value {
        let r = self.send(req.clone());
        assert_eq!(r["ok"], json!(true), "request {req} failed: {r}");
        r
    }

    fn close(mut self) -> std::process::ExitStatus {
        drop(self.stdin.take());
        for _ in 0..50 {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        panic!("talax-correct did not exit on EOF");
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn profile_with_names(names: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("domain_context.json"),
        json!({ "all_proper_nouns": names }).to_string(),
    )
    .unwrap();
    dir
}

fn text(r: &Value) -> &str {
    r["text"].as_str().unwrap_or_else(|| panic!("no text in {r}"))
}

#[test]
fn health_reports_version_and_state() {
    let prof = profile_with_names(&["Tibo"]);
    let mut s = Sidecar::start(prof.path(), None);
    let r = s.ok(json!({"op": "health"}));
    assert_eq!(r["version"], json!(env!("CARGO_PKG_VERSION")));
    assert_eq!(r["rules"], json!(0));
    assert_eq!(r["proper_nouns"], json!(1));
    assert!(r["lexicon"].is_number());
    assert!(r["ngram_trained"].is_boolean());
    assert!(prof.path().join("corrections.db").exists(), "the profile database is created on start");
}

#[test]
fn request_id_is_echoed() {
    let prof = profile_with_names(&[]);
    let mut s = Sidecar::start(prof.path(), None);
    assert_eq!(s.ok(json!({"id": 7, "op": "health"}))["id"], json!(7));
    assert_eq!(s.ok(json!({"id": "a-1", "op": "health"}))["id"], json!("a-1"));
}

#[test]
fn teach_then_correct() {
    let prof = profile_with_names(&[]);
    let mut s = Sidecar::start(prof.path(), None);
    let r = s.ok(json!({"op": "teach", "original": "Teepo", "corrected": "Tibo"}));
    assert_eq!(r["rules"], json!(1));
    let r = s.ok(json!({"op": "correct", "text": "I asked Teepo."}));
    assert_eq!(text(&r), "I asked Tibo.");
    let changes = r["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["layer"], json!("dictionary"));
    assert_eq!(changes[0]["original"], json!("Teepo"));
    assert_eq!(changes[0]["corrected"], json!("Tibo"));
    assert_eq!(r["session_id"], Value::Null, "no session unless record is set");
}

#[test]
fn names_drive_the_heuristic_layer_and_layers_can_be_limited() {
    let prof = profile_with_names(&["Tibo"]);
    let mut s = Sidecar::start(prof.path(), None);
    assert_eq!(text(&s.ok(json!({"op": "correct", "text": "Tipo, too."}))), "Tibo, too.");
    let r = s.ok(json!({"op": "correct", "text": "Tipo, too.", "layers": ["dictionary"]}));
    assert_eq!(text(&r), "Tipo, too.");
    let r = s.send(json!({"op": "correct", "text": "x", "layers": ["nonsense"]}));
    assert_eq!(r["ok"], json!(false));
}

#[test]
fn reload_picks_up_a_changed_names_file() {
    let prof = profile_with_names(&[]);
    let mut s = Sidecar::start(prof.path(), None);
    assert_eq!(text(&s.ok(json!({"op": "correct", "text": "ask tipo"}))), "ask tipo");
    std::fs::write(
        prof.path().join("domain_context.json"),
        json!({ "all_proper_nouns": ["Tibo"] }).to_string(),
    )
    .unwrap();
    let r = s.ok(json!({"op": "reload"}));
    assert_eq!(r["proper_nouns"], json!(1));
    assert_eq!(text(&s.ok(json!({"op": "correct", "text": "ask tipo"}))), "ask Tibo");
}

#[test]
fn lexicon_protects_common_words() {
    let prof = profile_with_names(&["Apter"]);
    let lex = prof.path().join("words");
    std::fs::write(&lex, "after\nlunch\nemail\n").unwrap();
    let mut s = Sidecar::start(prof.path(), Some(&lex));
    assert_eq!(s.ok(json!({"op": "health"}))["lexicon"], json!(3));
    assert_eq!(text(&s.ok(json!({"op": "correct", "text": "after lunch"}))), "after lunch");
    assert_eq!(text(&s.ok(json!({"op": "correct", "text": "email Aptor"}))), "email Apter");
}

#[test]
fn recorded_takes_teach_after_three_consistent_fixes() {
    let prof = profile_with_names(&[]);
    let mut s = Sidecar::start(prof.path(), None);
    for _ in 0..3 {
        let r = s.ok(json!({"op": "correct", "text": "meet Teebo today", "record": true}));
        assert_eq!(text(&r), "meet Teebo today");
        let sid = r["session_id"].as_str().expect("record returns a session id").to_string();
        let l = s.ok(json!({"op": "learn", "session_id": sid, "text": "meet Tibo today"}));
        assert!(l["auto_apply"].is_number());
    }
    let r = s.ok(json!({"op": "correct", "text": "meet Teebo today"}));
    assert_eq!(text(&r), "meet Tibo today");
}

#[test]
fn learn_needs_a_known_session() {
    let prof = profile_with_names(&[]);
    let mut s = Sidecar::start(prof.path(), None);
    let r = s.send(json!({"op": "learn", "session_id": "nope", "text": "hello"}));
    assert_eq!(r["ok"], json!(false));
    assert!(r["error"].is_string());
}

#[test]
fn rules_and_forget() {
    let prof = profile_with_names(&[]);
    let mut s = Sidecar::start(prof.path(), None);
    s.ok(json!({"op": "teach", "original": "Teepo", "corrected": "Tibo"}));
    let r = s.ok(json!({"op": "rules"}));
    let rules = r["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0]["original"], json!("teepo"));
    assert_eq!(rules[0]["corrected"], json!("Tibo"));
    assert_eq!(rules[0]["manual"], json!(true));
    assert_eq!(rules[0]["auto_apply"], json!(true));
    let r = s.ok(json!({"op": "forget", "original": "teepo"}));
    assert_eq!(r["removed"], json!(1));
    assert_eq!(text(&s.ok(json!({"op": "correct", "text": "Teepo"}))), "Teepo");
}

#[test]
fn bad_input_is_answered_and_the_sidecar_keeps_serving() {
    let prof = profile_with_names(&[]);
    let mut s = Sidecar::start(prof.path(), None);
    for bad in [
        "this is not json".to_string(),
        json!({"op": "fly"}).to_string(),
        json!({"no_op": 1}).to_string(),
        json!({"op": "correct"}).to_string(),
        json!({"op": "teach", "original": "", "corrected": "x"}).to_string(),
        "[1, 2, 3]".to_string(),
    ] {
        let r = s.raw(&bad);
        assert_eq!(r["ok"], json!(false), "{bad} -> {r}");
        assert!(r["error"].as_str().is_some_and(|e| !e.is_empty()), "{bad} -> {r}");
    }
    s.ok(json!({"op": "health"}));
}

#[test]
fn rules_persist_across_restarts() {
    let prof = profile_with_names(&[]);
    let mut s = Sidecar::start(prof.path(), None);
    s.ok(json!({"op": "teach", "original": "Talak", "corrected": "TalaX"}));
    assert!(s.close().success());
    let mut s = Sidecar::start(prof.path(), None);
    assert_eq!(text(&s.ok(json!({"op": "correct", "text": "the Talak system"}))), "the TalaX system");
}

#[test]
fn eof_is_a_clean_exit() {
    let prof = profile_with_names(&[]);
    let mut s = Sidecar::start(prof.path(), None);
    s.ok(json!({"op": "health"}));
    assert!(s.close().success());
}

#[test]
fn unicode_and_long_text_round_trip() {
    let prof = profile_with_names(&["Rozière"]);
    let mut s = Sidecar::start(prof.path(), None);
    let r = s.ok(json!({"op": "correct", "text": "Baptiste Roziere og Heimir sögðu já"}));
    assert_eq!(text(&r), "Baptiste Rozière og Heimir sögðu já");
    let long = "the quick brown fox jumps over the lazy dog ".repeat(200);
    let r = s.ok(json!({"op": "correct", "text": long.trim_end()}));
    assert_eq!(text(&r), long.trim_end());
}
