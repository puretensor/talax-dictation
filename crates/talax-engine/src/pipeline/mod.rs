pub mod dict_corrector;
pub mod heuristic;
pub mod ngram_corrector;

use dict_corrector::DictionaryCorrector;
use heuristic::HeuristicExpander;
use ngram_corrector::NgramCorrector;

/// A single correction applied by the pipeline.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Change {
    pub layer: String,
    pub position: Option<usize>,
    pub original: String,
    pub corrected: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_freq: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corrected_score: Option<f64>,
}

/// Result of running the correction pipeline.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PipelineResult {
    pub corrected: String,
    pub changes: Vec<Change>,
    pub layers_used: Vec<String>,
}

/// Three-layer correction pipeline: Dictionary -> N-gram -> Heuristic.
pub struct CorrectionPipeline {
    pub dict_corrector: DictionaryCorrector,
    pub ngram_corrector: NgramCorrector,
    pub heuristic_expander: HeuristicExpander,
}

impl Default for CorrectionPipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl CorrectionPipeline {
    pub fn new() -> Self {
        Self {
            dict_corrector: DictionaryCorrector::new(),
            ngram_corrector: NgramCorrector::new(),
            heuristic_expander: HeuristicExpander::new(),
        }
    }

    pub fn set_ngram_model_path(&mut self, path: std::path::PathBuf) {
        self.ngram_corrector.set_model_path(path);
    }

    /// Reload all corrector state from the database/model files.
    pub fn try_reload(&mut self, db: &crate::db::Database) -> rusqlite::Result<()> {
        let patterns = db.get_all_patterns()?;
        let training_texts = db.get_training_texts()?;

        self.dict_corrector.try_reload(db)?;
        self.ngram_corrector
            .rebuild_from_training_data(&training_texts, &patterns);
        self.heuristic_expander.reload(db);
        self.heuristic_expander
            .set_word_frequencies(self.ngram_corrector.word_frequencies());
        Ok(())
    }

    /// Reload all corrector state from the database/model files.
    pub fn reload(&mut self, db: &crate::db::Database) {
        if let Err(err) = self.try_reload(db) {
            tracing::warn!("failed to reload correction pipeline: {err}");
        }
    }

    /// Run the full correction pipeline on ASR output.
    pub fn process(&self, text: &str) -> PipelineResult {
        let mut current = text.to_string();
        let mut all_changes = Vec::new();

        // Layer 1: Dictionary substitution (<1ms)
        let (corrected, dict_changes) = self.dict_corrector.apply(&current);
        current = corrected;
        all_changes.extend(dict_changes);

        // Layer 2: N-gram corrections (<50ms)
        if self.ngram_corrector.is_trained() {
            let (corrected, ngram_changes) = self.ngram_corrector.apply(&current);
            current = corrected;
            all_changes.extend(ngram_changes);
        }

        // Layer 3: Heuristic expander (<10ms)
        if self.needs_heuristic_review(&current) {
            let (corrected, heuristic_changes) = self.heuristic_expander.apply(&current);
            current = corrected;
            all_changes.extend(heuristic_changes);
        }

        let layers_used: Vec<String> = all_changes
            .iter()
            .map(|c| c.layer.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        PipelineResult {
            corrected: current,
            changes: all_changes,
            layers_used,
        }
    }

    fn needs_heuristic_review(&self, text: &str) -> bool {
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.is_empty() {
            return false;
        }
        if !self.ngram_corrector.is_trained() {
            return true;
        }
        let known = words
            .iter()
            .filter(|w| self.ngram_corrector.vocab().contains(&w.to_lowercase()))
            .count();
        let ratio = known as f64 / words.len() as f64;
        ratio < 0.8
    }
}
