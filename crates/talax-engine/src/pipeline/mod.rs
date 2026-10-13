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

/// Which correction layers to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layers {
    pub dictionary: bool,
    pub ngram: bool,
    pub heuristic: bool,
}

impl Layers {
    pub const ALL: Self = Self {
        dictionary: true,
        ngram: true,
        heuristic: true,
    };

    pub const DICTIONARY_ONLY: Self = Self {
        dictionary: true,
        ngram: false,
        heuristic: false,
    };

    pub fn from_names(names: &[&str]) -> Result<Self, String> {
        let mut layers = Self {
            dictionary: false,
            ngram: false,
            heuristic: false,
        };
        for name in names {
            match *name {
                "dictionary" => layers.dictionary = true,
                "ngram" => layers.ngram = true,
                "heuristic" => layers.heuristic = true,
                other => return Err(format!("unknown layer '{other}'")),
            }
        }
        Ok(layers)
    }
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
        let patterns = db.get_auto_corrections()?;
        let training_texts = db.get_training_texts()?;

        self.dict_corrector.try_reload(db)?;
        self.ngram_corrector
            .rebuild_from_training_data(&training_texts, &patterns);
        self.heuristic_expander.reload(db);
        self.heuristic_expander
            .set_word_frequencies(self.ngram_corrector.word_frequencies());

        let mut known = db.get_lexicon();
        known.extend(db.get_correct_usage_words()?);
        known.extend(self.ngram_corrector.vocab().iter().cloned());
        self.heuristic_expander.set_known_words(known);
        Ok(())
    }

    /// Reload all corrector state from the database/model files.
    pub fn reload(&mut self, db: &crate::db::Database) {
        if let Err(err) = self.try_reload(db) {
            tracing::warn!("failed to reload correction pipeline: {err}");
        }
    }

    /// Reload only what a rule change affects (dictionary rules and the n-gram corrections
    /// index). Cheap: unlike `try_reload` it does not re-read the lexicon or retrain the
    /// n-gram model, so a host can teach many rules in a row.
    pub fn reload_rules(&mut self, db: &crate::db::Database) -> rusqlite::Result<()> {
        self.dict_corrector.try_reload(db)?;
        let patterns = db.get_auto_corrections()?;
        self.ngram_corrector
            .set_corrections_index_from_patterns(&patterns);
        Ok(())
    }

    /// Run the full correction pipeline on ASR output.
    pub fn process(&self, text: &str) -> PipelineResult {
        self.process_with(text, Layers::ALL)
    }

    /// Run a selected subset of correction layers.
    pub fn process_with(&self, text: &str, layers: Layers) -> PipelineResult {
        let mut current = text.to_string();
        let mut all_changes = Vec::new();

        if layers.dictionary {
            let (corrected, dict_changes) = self.dict_corrector.apply(&current);
            current = corrected;
            all_changes.extend(dict_changes);
        }

        if layers.ngram && self.ngram_corrector.is_trained() {
            let (corrected, ngram_changes) = self.ngram_corrector.apply(&current);
            current = corrected;
            all_changes.extend(ngram_changes);
        }

        if layers.heuristic {
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
}
