//! SQLite database for corrections, patterns, and sessions.
//! Port of /opt/dictation/server/db.py

use rusqlite::{Connection, Result as SqlResult, params};
use std::path::Path;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    audio_file TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    duration_sec REAL DEFAULT 0,
    whisper_model TEXT DEFAULT 'unknown',
    total_segments INTEGER DEFAULT 0,
    total_corrections INTEGER DEFAULT 0,
    reviewed INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS segments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    segment_index INTEGER NOT NULL,
    start_time REAL DEFAULT 0,
    end_time REAL DEFAULT 0,
    original_text TEXT NOT NULL,
    corrected_text TEXT,
    reviewed INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS word_corrections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    segment_id INTEGER NOT NULL REFERENCES segments(id),
    original TEXT NOT NULL,
    corrected TEXT NOT NULL,
    position INTEGER DEFAULT 0,
    correction_type TEXT DEFAULT 'manual',
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS correction_patterns (
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

CREATE TABLE IF NOT EXISTS correct_usages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    word TEXT NOT NULL UNIQUE,
    frequency INTEGER DEFAULT 1,
    last_seen DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS audio_excerpts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    segment_id INTEGER REFERENCES segments(id),
    audio_file TEXT,
    start_time REAL,
    end_time REAL,
    transcript TEXT,
    extracted INTEGER DEFAULT 0,
    used_in_training INTEGER DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_corrections_original ON word_corrections(original);
CREATE INDEX IF NOT EXISTS idx_patterns_original ON correction_patterns(original);
CREATE INDEX IF NOT EXISTS idx_patterns_auto ON correction_patterns(auto_apply);
CREATE INDEX IF NOT EXISTS idx_segments_session ON segments(session_id);
CREATE INDEX IF NOT EXISTS idx_segments_reviewed ON segments(reviewed);
"#;

/// A correction pattern from the database.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CorrectionPattern {
    pub original: String,
    pub corrected: String,
    pub frequency: i64,
    pub confidence: f64,
    pub context_before: Option<String>,
    pub context_after: Option<String>,
}

/// Domain context loaded from JSON.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DomainContext {
    /// Lowercase -> correct casing
    pub proper_nouns: std::collections::HashMap<String, String>,
    /// Wrong -> correct (accent patterns)
    pub accent_patterns: std::collections::HashMap<String, String>,
}

pub struct Database {
    conn: Connection,
    domain_context_path: Option<std::path::PathBuf>,
}

impl Database {
    /// Open or create a database at the given path.
    pub fn open(path: &Path) -> SqlResult<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn,
            domain_context_path: None,
        })
    }

    /// Open an in-memory database (for testing).
    pub fn open_memory() -> SqlResult<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn,
            domain_context_path: None,
        })
    }

    /// Set the path to the domain context JSON file.
    pub fn set_domain_context_path(&mut self, path: std::path::PathBuf) {
        self.domain_context_path = Some(path);
    }

    /// Get auto-apply correction patterns.
    pub fn get_auto_corrections(&self) -> SqlResult<Vec<CorrectionPattern>> {
        let mut stmt = self.conn.prepare(
            "SELECT original, corrected, frequency, confidence, context_before, context_after
             FROM correction_patterns WHERE auto_apply = 1
             ORDER BY frequency DESC",
        )?;

        stmt.query_map([], map_correction_pattern)?
            .collect::<SqlResult<Vec<_>>>()
    }

    /// Get all correction patterns.
    pub fn get_all_patterns(&self) -> SqlResult<Vec<CorrectionPattern>> {
        let mut stmt = self.conn.prepare(
            "SELECT original, corrected, frequency, confidence, context_before, context_after
             FROM correction_patterns ORDER BY frequency DESC",
        )?;

        stmt.query_map([], map_correction_pattern)?
            .collect::<SqlResult<Vec<_>>>()
    }

    /// Load domain context from JSON file.
    pub fn get_domain_context(&self) -> DomainContext {
        let Some(path) = &self.domain_context_path else {
            return DomainContext::default();
        };
        let Ok(data) = std::fs::read_to_string(path) else {
            return DomainContext::default();
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) else {
            return DomainContext::default();
        };

        let mut proper_nouns = std::collections::HashMap::new();
        if let Some(nouns) = json.get("all_proper_nouns").and_then(|v| v.as_array()) {
            for noun in nouns {
                if let Some(s) = noun.as_str() {
                    proper_nouns.insert(s.to_lowercase(), s.to_string());
                }
            }
        }

        let mut accent_patterns = std::collections::HashMap::new();
        if let Some(patterns) = json
            .get("known_accent_patterns")
            .and_then(|v| v.as_object())
        {
            for (wrong, right) in patterns {
                if let Some(correct) = right.as_str() {
                    accent_patterns.insert(wrong.to_lowercase(), correct.to_string());
                }
            }
        }

        DomainContext {
            proper_nouns,
            accent_patterns,
        }
    }

    /// Create a new session.
    pub fn create_session(&self, id: &str, audio_file: &str, duration: f64) -> SqlResult<()> {
        self.create_session_with_model(id, audio_file, duration, "unknown")
    }

    /// Create a new session with an explicit whisper model name.
    pub fn create_session_with_model(
        &self,
        id: &str,
        audio_file: &str,
        duration: f64,
        whisper_model: &str,
    ) -> SqlResult<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, audio_file, duration_sec, whisper_model) VALUES (?1, ?2, ?3, ?4)",
            params![id, audio_file, duration, whisper_model],
        )?;
        Ok(())
    }

    /// Add segments to a session.
    pub fn add_segments(&self, session_id: &str, segments: &[(f64, f64, &str)]) -> SqlResult<()> {
        let mut stmt = self.conn.prepare(
            "INSERT INTO segments (session_id, segment_index, start_time, end_time, original_text)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (i, (start, end, text)) in segments.iter().enumerate() {
            stmt.execute(params![session_id, i as i64, start, end, text])?;
        }
        // Update session segment count
        self.conn.execute(
            "UPDATE sessions SET total_segments = ?1 WHERE id = ?2",
            params![segments.len() as i64, session_id],
        )?;
        Ok(())
    }

    /// Save user corrections, update patterns, rebuild auto-apply flags.
    pub fn save_corrections(
        &self,
        session_id: &str,
        corrections: &[(usize, &str)],
    ) -> SqlResult<()> {
        for (segment_index, corrected_text) in corrections {
            // Get the segment
            let segment: Option<(i64, String)> = self
                .conn
                .query_row(
                    "SELECT id, original_text FROM segments WHERE session_id = ?1 AND segment_index = ?2",
                    params![session_id, *segment_index as i64],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();

            let Some((segment_id, original_text)) = segment else {
                continue;
            };

            // Update the segment
            self.conn.execute(
                "UPDATE segments SET corrected_text = ?1, reviewed = 1 WHERE id = ?2",
                params![corrected_text, segment_id],
            )?;

            // Compute aligned word-level diffs and insert correction patterns.
            let orig_words: Vec<&str> = original_text.split_whitespace().collect();
            let corr_words: Vec<&str> = corrected_text.split_whitespace().collect();
            let aligned = align_word_sequences(&orig_words, &corr_words);

            for (position, op) in aligned.into_iter().enumerate() {
                match op {
                    WordAlignment::Equal(word) => {
                        self.increment_correct_usage(word)?;
                    }
                    WordAlignment::Replace {
                        original,
                        corrected,
                    } => {
                        if original.is_empty() || corrected.is_empty() {
                            continue;
                        }

                        self.conn.execute(
                            "INSERT INTO word_corrections (segment_id, original, corrected, position)
                             VALUES (?1, ?2, ?3, ?4)",
                            params![segment_id, original, corrected, position as i64],
                        )?;

                        self.conn.execute(
                            "INSERT INTO correction_patterns (original, corrected, frequency, confidence, context_before, context_after)
                             VALUES (?1, ?2, 1, 0.5, '', '')
                             ON CONFLICT(original, corrected, context_before, context_after)
                             DO UPDATE SET frequency = frequency + 1, last_seen = CURRENT_TIMESTAMP",
                            params![original.to_lowercase(), corrected],
                        )?;
                    }
                    WordAlignment::Delete(_) | WordAlignment::Insert(_) => {}
                }
            }
        }

        self.refresh_session_review_state(session_id)?;

        // Rebuild auto-apply flags
        self.rebuild_auto_apply()?;
        Ok(())
    }

    /// Stage corrected text for segments without marking them as reviewed.
    pub fn stage_corrections(
        &self,
        session_id: &str,
        corrections: &[(usize, &str)],
    ) -> SqlResult<()> {
        let mut stmt = self.conn.prepare(
            "UPDATE segments
             SET corrected_text = ?1
             WHERE session_id = ?2 AND segment_index = ?3",
        )?;

        for (segment_index, corrected_text) in corrections {
            stmt.execute(params![corrected_text, session_id, *segment_index as i64])?;
        }

        Ok(())
    }

    /// Training texts derived from accepted reviewed output.
    pub fn get_training_texts(&self) -> SqlResult<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT corrected_text
             FROM segments
             WHERE reviewed = 1 AND corrected_text IS NOT NULL
             ORDER BY id ASC",
        )?;

        stmt.query_map([], |row| row.get::<_, String>(0))?
            .collect::<SqlResult<Vec<_>>>()
    }

    /// Rebuild auto_apply flags based on frequency and confidence thresholds.
    fn rebuild_auto_apply(&self) -> SqlResult<()> {
        // First, reset all
        self.conn
            .execute("UPDATE correction_patterns SET auto_apply = 0", [])?;

        // Recompute confidence using correct_usages
        // confidence = frequency / (frequency + correct_usages)
        self.conn.execute_batch(
            "UPDATE correction_patterns SET confidence =
                CAST(frequency AS REAL) / (frequency + COALESCE(
                    (SELECT cu.frequency FROM correct_usages cu WHERE cu.word = correction_patterns.original), 0
                ) + 1);
             UPDATE correction_patterns SET auto_apply = 1
                WHERE frequency >= 3 AND confidence >= 0.75;",
        )?;
        Ok(())
    }

    fn refresh_session_review_state(&self, session_id: &str) -> SqlResult<()> {
        self.conn.execute(
            "UPDATE sessions
             SET reviewed = CASE
                    WHEN EXISTS(
                        SELECT 1 FROM segments
                        WHERE session_id = ?1 AND reviewed = 1
                    ) THEN 1 ELSE 0 END,
                 total_corrections = (
                    SELECT COUNT(*)
                    FROM word_corrections wc
                    INNER JOIN segments s ON s.id = wc.segment_id
                    WHERE s.session_id = ?1
                 )
             WHERE id = ?1",
            params![session_id],
        )?;

        Ok(())
    }

    fn increment_correct_usage(&self, word: &str) -> SqlResult<()> {
        let normalized = normalize_learning_word(word);
        if normalized.is_empty() {
            return Ok(());
        }

        self.conn.execute(
            "INSERT INTO correct_usages (word, frequency)
             VALUES (?1, 1)
             ON CONFLICT(word)
             DO UPDATE SET frequency = frequency + 1, last_seen = CURRENT_TIMESTAMP",
            params![normalized],
        )?;

        Ok(())
    }

    /// List sessions ordered by creation time (most recent first).
    pub fn list_sessions(&self, limit: usize) -> SqlResult<Vec<SessionSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.created_at, s.duration_sec, s.total_segments, s.reviewed
             FROM sessions s
             ORDER BY s.created_at DESC
             LIMIT ?1",
        )?;

        stmt.query_map(params![limit as i64], |row| {
            let reviewed_int: i64 = row.get(4)?;
            Ok(SessionSummary {
                id: row.get(0)?,
                created_at: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                duration: row.get(2)?,
                segment_count: row.get(3)?,
                reviewed: reviewed_int != 0,
            })
        })?
        .collect::<SqlResult<Vec<_>>>()
    }

    /// Get full session detail including all segments.
    pub fn get_session_detail(&self, session_id: &str) -> SqlResult<Option<SessionDetail>> {
        let session = match self.conn.query_row(
            "SELECT id, COALESCE(created_at, ''), duration_sec, COALESCE(whisper_model, 'unknown')
             FROM sessions WHERE id = ?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ) {
            Ok(session) => session,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(err) => return Err(err),
        };

        let (id, created_at, duration, whisper_model) = session;

        let mut stmt = self.conn.prepare(
            "SELECT segment_index, original_text, corrected_text, start_time, end_time, reviewed
             FROM segments WHERE session_id = ?1 ORDER BY segment_index ASC",
        )?;

        let segments: Vec<SegmentDetail> = stmt
            .query_map(params![session_id], |row| {
                let reviewed_int: i64 = row.get(5)?;
                Ok(SegmentDetail {
                    segment_index: row.get(0)?,
                    original_text: row.get(1)?,
                    corrected_text: row.get(2)?,
                    start_time: row.get(3)?,
                    end_time: row.get(4)?,
                    reviewed: reviewed_int != 0,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?;

        Ok(Some(SessionDetail {
            id,
            created_at,
            duration,
            whisper_model,
            segments,
        }))
    }

    /// Get basic statistics.
    pub fn get_stats(&self) -> SqlResult<Stats> {
        let session_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?;
        let pattern_count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM correction_patterns", [], |r| r.get(0))?;
        let auto_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM correction_patterns WHERE auto_apply = 1",
            [],
            |r| r.get(0),
        )?;

        Ok(Stats {
            session_count,
            pattern_count,
            auto_apply_count: auto_count,
        })
    }
}

fn map_correction_pattern(row: &rusqlite::Row<'_>) -> SqlResult<CorrectionPattern> {
    Ok(CorrectionPattern {
        original: row.get(0)?,
        corrected: row.get(1)?,
        frequency: row.get(2)?,
        confidence: row.get(3)?,
        context_before: row.get(4)?,
        context_after: row.get(5)?,
    })
}

/// Summary of a dictation session (for list views).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub created_at: String,
    pub duration: f64,
    pub segment_count: i64,
    pub reviewed: bool,
}

/// A single segment within a session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SegmentDetail {
    pub segment_index: i64,
    pub original_text: String,
    pub corrected_text: Option<String>,
    pub start_time: f64,
    pub end_time: f64,
    pub reviewed: bool,
}

/// Full detail of a session including all segments.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionDetail {
    pub id: String,
    pub created_at: String,
    pub duration: f64,
    pub whisper_model: String,
    pub segments: Vec<SegmentDetail>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Stats {
    pub session_count: i64,
    pub pattern_count: i64,
    pub auto_apply_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordAlignment<'a> {
    Equal(&'a str),
    Replace {
        original: &'a str,
        corrected: &'a str,
    },
    Delete(&'a str),
    Insert(&'a str),
}

fn align_word_sequences<'a>(
    original: &'a [&'a str],
    corrected: &'a [&'a str],
) -> Vec<WordAlignment<'a>> {
    let m = original.len();
    let n = corrected.len();
    let mut dp = vec![vec![0usize; n + 1]; m + 1];

    for (i, row) in dp.iter_mut().enumerate().take(m + 1) {
        row[0] = i;
    }
    for (j, cell) in dp[0].iter_mut().enumerate().take(n + 1) {
        *cell = j;
    }

    for i in 1..=m {
        for j in 1..=n {
            let substitution_cost = usize::from(original[i - 1] != corrected[j - 1]);
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + substitution_cost);
        }
    }

    let mut i = m;
    let mut j = n;
    let mut ops = Vec::new();

    while i > 0 || j > 0 {
        if i > 0 && j > 0 {
            let substitution_cost = usize::from(original[i - 1] != corrected[j - 1]);
            if dp[i][j] == dp[i - 1][j - 1] + substitution_cost {
                if substitution_cost == 0 {
                    ops.push(WordAlignment::Equal(original[i - 1]));
                } else {
                    ops.push(WordAlignment::Replace {
                        original: original[i - 1],
                        corrected: corrected[j - 1],
                    });
                }
                i -= 1;
                j -= 1;
                continue;
            }
        }

        if i > 0 && dp[i][j] == dp[i - 1][j] + 1 {
            ops.push(WordAlignment::Delete(original[i - 1]));
            i -= 1;
        } else if j > 0 {
            ops.push(WordAlignment::Insert(corrected[j - 1]));
            j -= 1;
        }
    }

    ops.reverse();
    ops
}

fn normalize_learning_word(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.')
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_memory() {
        let db = Database::open_memory().unwrap();
        let stats = db.get_stats().unwrap();
        assert_eq!(stats.session_count, 0);
    }

    #[test]
    fn fallible_read_apis_surface_schema_errors() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE sessions (id TEXT PRIMARY KEY);")
            .unwrap();
        let db = Database {
            conn,
            domain_context_path: None,
        };

        assert!(db.get_stats().is_err());
        assert!(db.get_all_patterns().is_err());
        assert!(db.get_auto_corrections().is_err());
        assert!(db.list_sessions(10).is_err());
        assert!(db.get_session_detail("sess1").is_err());
        assert!(db.get_training_texts().is_err());
    }

    #[test]
    fn test_create_session_and_segments() {
        let db = Database::open_memory().unwrap();
        db.create_session("test1", "/tmp/test.wav", 5.0).unwrap();
        db.add_segments("test1", &[(0.0, 2.5, "hello world"), (2.5, 5.0, "foo bar")])
            .unwrap();
        let stats = db.get_stats().unwrap();
        assert_eq!(stats.session_count, 1);
    }

    #[test]
    fn aligned_diff_treats_insertion_as_insertion() {
        let original = vec!["deploy", "cluster", "now"];
        let corrected = vec!["deploy", "the", "cluster", "now"];
        let aligned = align_word_sequences(&original, &corrected);

        assert_eq!(
            aligned,
            vec![
                WordAlignment::Equal("deploy"),
                WordAlignment::Insert("the"),
                WordAlignment::Equal("cluster"),
                WordAlignment::Equal("now"),
            ]
        );
    }

    #[test]
    fn save_corrections_records_negative_evidence_for_unchanged_words() {
        let db = Database::open_memory().unwrap();
        db.create_session("sess1", "", 1.0).unwrap();
        db.add_segments("sess1", &[(0.0, 1.0, "teh quick fox")])
            .unwrap();
        db.save_corrections("sess1", &[(0, "the quick fox")])
            .unwrap();

        let quick_freq: i64 = db
            .conn
            .query_row(
                "SELECT frequency FROM correct_usages WHERE word = 'quick'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let fox_freq: i64 = db
            .conn
            .query_row(
                "SELECT frequency FROM correct_usages WHERE word = 'fox'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(quick_freq, 1);
        assert_eq!(fox_freq, 1);
    }

    #[test]
    fn get_training_texts_returns_reviewed_outputs() {
        let db = Database::open_memory().unwrap();
        db.create_session("sess1", "", 1.0).unwrap();
        db.add_segments("sess1", &[(0.0, 1.0, "teh quick fox")])
            .unwrap();
        db.save_corrections("sess1", &[(0, "the quick fox")])
            .unwrap();

        assert_eq!(
            db.get_training_texts().unwrap(),
            vec!["the quick fox".to_string()]
        );
    }

    #[test]
    fn stage_corrections_does_not_mark_session_reviewed() {
        let db = Database::open_memory().unwrap();
        db.create_session("sess1", "", 1.0).unwrap();
        db.add_segments("sess1", &[(0.0, 1.0, "teh quick fox")])
            .unwrap();
        db.stage_corrections("sess1", &[(0, "the quick fox")])
            .unwrap();

        let detail = db.get_session_detail("sess1").unwrap().unwrap();
        assert_eq!(
            detail.segments[0].corrected_text.as_deref(),
            Some("the quick fox")
        );
        assert!(!detail.segments[0].reviewed);

        let sessions = db.list_sessions(10).unwrap();
        assert!(!sessions[0].reviewed);
    }
}
