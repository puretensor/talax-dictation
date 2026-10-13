//! talax-correct: TalaX's correction pipeline as a long-lived JSON-lines sidecar.
//! Contract: crates/talax-correct/tests/protocol.rs.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use talax_engine::db::Database;
use talax_engine::pipeline::{CorrectionPipeline, Layers};

fn main() {
    if let Err(err) = run(std::env::args().skip(1).collect()) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let config = parse_args(&args)?;
    serve(config)
}

struct Config {
    profile_dir: PathBuf,
    lexicon: Option<PathBuf>,
    retain_days: i64,
}

fn parse_args(args: &[String]) -> Result<Config, String> {
    let usage = "usage: talax-correct serve --profile-dir DIR [--lexicon FILE] [--retain-days N]";
    let mut iter = args.iter();
    let Some(cmd) = iter.next() else {
        return Err(usage.to_string());
    };
    if cmd != "serve" {
        return Err(format!("unknown command '{cmd}'\n{usage}"));
    }

    let mut profile_dir = None;
    let mut lexicon = None;
    let mut retain_days: i64 = 7;

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--profile-dir" => {
                profile_dir = Some(PathBuf::from(expect_value("--profile-dir", &mut iter)?));
            }
            "--lexicon" => {
                lexicon = Some(PathBuf::from(expect_value("--lexicon", &mut iter)?));
            }
            "--retain-days" => {
                let raw = expect_value("--retain-days", &mut iter)?;
                retain_days = raw
                    .parse()
                    .map_err(|_| format!("--retain-days expects an integer, got '{raw}'"))?;
            }
            other => return Err(format!("unknown argument '{other}'\n{usage}")),
        }
    }

    let Some(profile_dir) = profile_dir else {
        return Err(format!("--profile-dir is required\n{usage}"));
    };
    Ok(Config {
        profile_dir,
        lexicon,
        retain_days,
    })
}

fn expect_value<'a>(
    flag: &str,
    iter: &mut std::slice::Iter<'a, String>,
) -> Result<&'a str, String> {
    iter.next()
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value"))
}

struct Server {
    db: Database,
    pipeline: CorrectionPipeline,
    /// Lexicon size at the last (re)load; health must not re-read a 235k-line file per call.
    lexicon_len: usize,
}

fn serve(config: Config) -> Result<(), String> {
    ensure_private_dir(&config.profile_dir)?;

    let mut db = Database::open(&config.profile_dir.join("corrections.db"))
        .map_err(|e| format!("open database: {e}"))?;
    db.set_domain_context_path(config.profile_dir.join("domain_context.json"));
    let lexicon = config
        .lexicon
        .unwrap_or_else(|| config.profile_dir.join("lexicon.txt"));
    db.set_lexicon_path(lexicon);

    db.prune_unreviewed_older_than(config.retain_days)
        .map_err(|e| format!("prune sessions: {e}"))?;

    let mut pipeline = CorrectionPipeline::new();
    pipeline.set_ngram_model_path(config.profile_dir.join("ngram.json"));
    pipeline
        .try_reload(&db)
        .map_err(|e| format!("load pipeline: {e}"))?;

    let lexicon_len = db.get_lexicon().len();
    let mut server = Server {
        db,
        pipeline,
        lexicon_len,
    };
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => break,
        };
        let reply = handle_line(&mut server, &line);
        writeln!(stdout, "{reply}").map_err(|e| format!("write response: {e}"))?;
        stdout.flush().map_err(|e| format!("flush response: {e}"))?;
    }
    Ok(())
}

fn ensure_private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create profile dir: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(dir)
            .map_err(|e| format!("stat profile dir: {e}"))?
            .permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(dir, perms).map_err(|e| format!("chmod profile dir: {e}"))?;
    }
    Ok(())
}

fn handle_line(server: &mut Server, line: &str) -> Value {
    let parsed = match serde_json::from_str::<Value>(line) {
        Ok(value) => value,
        Err(_) => return fail(None, "malformed JSON"),
    };
    let Value::Object(map) = parsed else {
        return fail(None, "request must be a JSON object");
    };
    let req = Value::Object(map);
    let id = req.get("id").cloned();
    match handle_op(server, &req) {
        Ok(mut body) => {
            if let Some(id) = id {
                body["id"] = id;
            }
            body
        }
        Err(err) => fail(id, &err),
    }
}

fn fail(id: Option<Value>, error: &str) -> Value {
    let mut body = json!({"ok": false, "error": error});
    if let Some(id) = id {
        body["id"] = id;
    }
    body
}

fn handle_op(server: &mut Server, req: &Value) -> Result<Value, String> {
    let op = req
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| "missing or invalid op".to_string())?;
    match op {
        "health" => server.status(),
        "reload" => {
            server.reload()?;
            server.status()
        }
        "correct" => server.correct(req),
        "learn" => server.learn(req),
        "teach" => server.teach(req),
        "forget" => server.forget(req),
        "rules" => server.rules(),
        other => Err(format!("unknown op '{other}'")),
    }
}

impl Server {
    fn reload(&mut self) -> Result<(), String> {
        self.pipeline
            .try_reload(&self.db)
            .map_err(|e| e.to_string())?;
        self.lexicon_len = self.db.get_lexicon().len();
        Ok(())
    }

    /// After teach/forget: rules only (~1 ms instead of a full reload's lexicon read).
    fn reload_rules(&mut self) -> Result<(), String> {
        self.pipeline
            .reload_rules(&self.db)
            .map_err(|e| e.to_string())
    }

    fn status(&self) -> Result<Value, String> {
        let stats = self.db.get_stats().map_err(|e| e.to_string())?;
        Ok(json!({
            "ok": true,
            "version": env!("CARGO_PKG_VERSION"),
            "rules": stats.auto_apply_count,
            "proper_nouns": self.db.get_domain_context().proper_nouns.len(),
            "lexicon": self.lexicon_len,
            "ngram_trained": self.pipeline.ngram_corrector.is_trained(),
        }))
    }

    fn correct(&mut self, req: &Value) -> Result<Value, String> {
        let text = req
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing or invalid text".to_string())?;
        let layers = parse_layers(req)?;
        let record = match req.get("record") {
            None => false,
            Some(value) => value
                .as_bool()
                .ok_or_else(|| "invalid record".to_string())?,
        };

        let result = self.pipeline.process_with(text, layers);
        let changes: Vec<Value> = result
            .changes
            .iter()
            .map(|c| {
                json!({
                    "layer": c.layer,
                    "original": c.original,
                    "corrected": c.corrected,
                })
            })
            .collect();

        let session_id = if record {
            let id = new_session_id();
            self.db
                .create_session(&id, "", 0.0)
                .map_err(|e| e.to_string())?;
            self.db
                .add_segments(&id, &[(0.0, 0.0, text)])
                .map_err(|e| e.to_string())?;
            self.db
                .stage_corrections(&id, &[(0, result.corrected.as_str())])
                .map_err(|e| e.to_string())?;
            Value::String(id)
        } else {
            Value::Null
        };

        Ok(json!({
            "ok": true,
            "text": result.corrected,
            "changes": changes,
            "layers_used": result.layers_used,
            "session_id": session_id,
        }))
    }

    fn learn(&mut self, req: &Value) -> Result<Value, String> {
        let session_id = req
            .get("session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing or invalid session_id".to_string())?;
        let text = req
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing or invalid text".to_string())?;
        let outcome = self
            .db
            .learn_from_review(session_id, text)
            .map_err(|e| e.to_string())?;
        self.reload()?;
        Ok(json!({
            "ok": true,
            "patterns": outcome.patterns,
            "confirmed": outcome.confirmed,
            "auto_apply": outcome.auto_apply,
        }))
    }

    fn teach(&mut self, req: &Value) -> Result<Value, String> {
        let original = req
            .get("original")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing or invalid original".to_string())?;
        let corrected = req
            .get("corrected")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing or invalid corrected".to_string())?;
        self.db
            .teach_rule(original, corrected)
            .map_err(|e| e.to_string())?;
        self.reload_rules()?;
        let stats = self.db.get_stats().map_err(|e| e.to_string())?;
        Ok(json!({"ok": true, "rules": stats.auto_apply_count}))
    }

    fn forget(&mut self, req: &Value) -> Result<Value, String> {
        let original = req
            .get("original")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing or invalid original".to_string())?;
        let corrected = match req.get("corrected") {
            None => None,
            Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_str()
                    .ok_or_else(|| "invalid corrected".to_string())?,
            ),
        };
        let removed = self
            .db
            .forget_rule(original, corrected)
            .map_err(|e| e.to_string())?;
        self.reload_rules()?;
        Ok(json!({"ok": true, "removed": removed}))
    }

    fn rules(&self) -> Result<Value, String> {
        let rules = self.db.list_rules().map_err(|e| e.to_string())?;
        Ok(json!({"ok": true, "rules": rules}))
    }
}

fn parse_layers(req: &Value) -> Result<Layers, String> {
    let Some(value) = req.get("layers") else {
        return Ok(Layers::ALL);
    };
    let arr = value
        .as_array()
        .ok_or_else(|| "invalid layers".to_string())?;
    let mut names = Vec::with_capacity(arr.len());
    for item in arr {
        names.push(item.as_str().ok_or_else(|| "invalid layers".to_string())?);
    }
    Layers::from_names(&names)
}

fn new_session_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{ts:x}-{n:x}")
}
