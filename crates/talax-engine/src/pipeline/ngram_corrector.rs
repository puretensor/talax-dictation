//! Layer 2: N-gram context model corrector.
//!
//! Lightweight trigram language model trained on the user's corrected text.
//! Port of /opt/dictation/server/ngram_corrector.py

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::db::CorrectionPattern;

use super::Change;

type Trigrams = HashMap<(String, String), HashMap<String, u32>>;
type Bigrams = HashMap<String, HashMap<String, u32>>;
type Unigrams = HashMap<String, u32>;

/// Runtime n-gram model state.
///
/// `trigrams` intentionally uses tuple keys for efficient lookup. That shape
/// cannot be represented as JSON object keys, so persistence uses
/// `PersistedNgramModel` below instead of serializing this struct directly.
#[derive(Default)]
pub struct NgramModel {
    pub trigrams: Trigrams,
    pub bigrams: Bigrams,
    pub unigrams: Unigrams,
    pub vocab: HashSet<String>,
    pub corrections_index: HashMap<String, Vec<String>>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedNgramModel {
    trigrams: Vec<PersistedTrigram>,
    bigrams: Bigrams,
    unigrams: Unigrams,
    vocab: HashSet<String>,
    corrections_index: HashMap<String, Vec<String>>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedTrigram {
    previous_previous: String,
    previous: String,
    next_counts: HashMap<String, u32>,
}

impl From<&NgramModel> for PersistedNgramModel {
    fn from(model: &NgramModel) -> Self {
        let trigrams = model
            .trigrams
            .iter()
            .map(
                |((previous_previous, previous), next_counts)| PersistedTrigram {
                    previous_previous: previous_previous.clone(),
                    previous: previous.clone(),
                    next_counts: next_counts.clone(),
                },
            )
            .collect();

        Self {
            trigrams,
            bigrams: model.bigrams.clone(),
            unigrams: model.unigrams.clone(),
            vocab: model.vocab.clone(),
            corrections_index: model.corrections_index.clone(),
        }
    }
}

impl From<PersistedNgramModel> for NgramModel {
    fn from(model: PersistedNgramModel) -> Self {
        let trigrams = model
            .trigrams
            .into_iter()
            .map(|entry| ((entry.previous_previous, entry.previous), entry.next_counts))
            .collect();

        Self {
            trigrams,
            bigrams: model.bigrams,
            unigrams: model.unigrams,
            vocab: model.vocab,
            corrections_index: model.corrections_index,
        }
    }
}

pub struct NgramCorrector {
    model: NgramModel,
    model_path: Option<PathBuf>,
}

impl Default for NgramCorrector {
    fn default() -> Self {
        Self::new()
    }
}

impl NgramCorrector {
    pub fn new() -> Self {
        Self {
            model: NgramModel::default(),
            model_path: None,
        }
    }

    pub fn with_model_path(path: PathBuf) -> Self {
        let mut corrector = Self::new();
        corrector.model_path = Some(path.clone());
        corrector.load_from(&path);
        corrector
    }

    pub fn reload(&mut self) {
        if let Some(path) = &self.model_path.clone() {
            self.load_from(path);
        }
    }

    pub fn set_model_path(&mut self, path: PathBuf) {
        self.model_path = Some(path);
    }

    pub fn is_trained(&self) -> bool {
        !self.model.unigrams.is_empty()
    }

    pub fn vocab(&self) -> &HashSet<String> {
        &self.model.vocab
    }

    pub fn word_frequencies(&self) -> HashMap<String, u32> {
        self.model.unigrams.clone()
    }

    /// Train on a collection of corrected transcription texts.
    pub fn train(&mut self, texts: &[String]) {
        self.model = NgramModel::default();

        for text in texts {
            let words: Vec<String> = std::iter::once("<s>".to_string())
                .chain(std::iter::once("<s>".to_string()))
                .chain(text.to_lowercase().split_whitespace().map(String::from))
                .chain(std::iter::once("</s>".to_string()))
                .collect();

            for i in 2..words.len() {
                let w0 = &words[i - 2];
                let w1 = &words[i - 1];
                let w2 = &words[i];

                *self
                    .model
                    .trigrams
                    .entry((w0.clone(), w1.clone()))
                    .or_default()
                    .entry(w2.clone())
                    .or_insert(0) += 1;

                *self
                    .model
                    .bigrams
                    .entry(w1.clone())
                    .or_default()
                    .entry(w2.clone())
                    .or_insert(0) += 1;

                *self.model.unigrams.entry(w2.clone()).or_insert(0) += 1;
                self.model.vocab.insert(w2.clone());
            }
        }
    }

    pub fn set_corrections_index_from_patterns(&mut self, patterns: &[CorrectionPattern]) {
        self.model.corrections_index.clear();

        for pattern in patterns {
            let key = pattern.original.to_lowercase();
            let entry = self.model.corrections_index.entry(key).or_default();
            if !entry
                .iter()
                .any(|candidate| candidate == &pattern.corrected)
            {
                entry.push(pattern.corrected.clone());
            }
        }
    }

    pub fn rebuild_from_training_data(&mut self, texts: &[String], patterns: &[CorrectionPattern]) {
        if texts.is_empty() {
            self.model = NgramModel::default();
            if let Some(path) = &self.model_path.clone() {
                self.load_from(path);
            }
        } else {
            self.train(texts);
        }

        self.set_corrections_index_from_patterns(patterns);

        if let Some(path) = &self.model_path {
            let _ = self.save(path);
        }
    }

    /// Interpolated trigram probability.
    fn score_word(&self, w2: &str, w0: &str, w1: &str) -> f64 {
        let (lambda1, lambda2, lambda3) = (0.6, 0.3, 0.1);
        let total_uni: u32 = self.model.unigrams.values().sum();
        let total_uni = total_uni.max(1) as f64;
        let vocab_size = self.model.vocab.len().max(1) as f64;

        let p_uni =
            (*self.model.unigrams.get(w2).unwrap_or(&0) as f64 + 1.0) / (total_uni + vocab_size);

        let p_bi = if let Some(bi_ctx) = self.model.bigrams.get(w1) {
            let bi_total: u32 = bi_ctx.values().sum();
            if bi_total > 0 {
                (*bi_ctx.get(w2).unwrap_or(&0) as f64 + 1.0) / (bi_total as f64 + vocab_size)
            } else {
                p_uni
            }
        } else {
            p_uni
        };

        let p_tri =
            if let Some(tri_ctx) = self.model.trigrams.get(&(w0.to_string(), w1.to_string())) {
                let tri_total: u32 = tri_ctx.values().sum();
                if tri_total > 0 {
                    (*tri_ctx.get(w2).unwrap_or(&0) as f64 + 1.0) / (tri_total as f64 + vocab_size)
                } else {
                    p_bi
                }
            } else {
                p_bi
            };

        lambda1 * p_tri + lambda2 * p_bi + lambda3 * p_uni
    }

    /// Apply high-confidence n-gram corrections.
    pub fn apply(&self, text: &str) -> (String, Vec<Change>) {
        let suggestions = self.suggest_corrections(text, 0.3);
        let mut words: Vec<String> = text.split_whitespace().map(String::from).collect();
        let mut changes = Vec::new();

        // Apply in reverse order to preserve positions
        let mut sorted = suggestions;
        sorted.sort_by_key(|change| std::cmp::Reverse(change.position));

        for s in sorted {
            if let (Some(pos), Some(orig_score), Some(corr_score)) =
                (s.position, s.original_score, s.corrected_score)
                && pos < words.len()
                && corr_score > orig_score * 3.0
            {
                words[pos] = s.corrected.clone();
                changes.push(s);
            }
        }

        (words.join(" "), changes)
    }

    fn suggest_corrections(&self, text: &str, threshold: f64) -> Vec<Change> {
        if self.model.unigrams.is_empty() {
            return Vec::new();
        }

        let words: Vec<String> = text
            .to_lowercase()
            .split_whitespace()
            .map(String::from)
            .collect();
        let mut padded = vec!["<s>".to_string(), "<s>".to_string()];
        padded.extend(words);
        padded.push("</s>".to_string());

        let mut suggestions = Vec::new();

        for i in 2..padded.len() - 1 {
            let (w0, w1, w2) = (&padded[i - 2], &padded[i - 1], &padded[i]);
            let score = self.score_word(w2, w0, w1);

            if score < threshold
                && let Some(candidates) = self.model.corrections_index.get(w2.as_str())
            {
                for candidate in candidates {
                    let cand_score = self.score_word(&candidate.to_lowercase(), w0, w1);
                    if cand_score > score * 2.0 {
                        suggestions.push(Change {
                            layer: "ngram".to_string(),
                            position: Some(i - 2),
                            original: w2.clone(),
                            corrected: candidate.clone(),
                            rule_freq: None,
                            original_score: Some(score),
                            corrected_score: Some(cand_score),
                        });
                    }
                }
            }
        }

        suggestions
    }

    /// Save model to a file using serde_json.
    pub fn save(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        let encoded = serde_json::to_vec(&PersistedNgramModel::from(&self.model))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, encoded)?;
        Ok(())
    }

    /// Load model from a serde_json file.
    pub fn load_from(&mut self, path: &std::path::Path) {
        if let Ok(data) = std::fs::read(path)
            && let Ok(model) = serde_json::from_slice::<PersistedNgramModel>(&data)
        {
            self.model = model.into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_train_and_query() {
        let mut corrector = NgramCorrector::new();
        corrector.train(&[
            "the quick brown fox jumps over the lazy dog".to_string(),
            "the quick brown fox runs through the field".to_string(),
        ]);
        assert!(corrector.is_trained());
        assert!(corrector.model.vocab.contains("fox"));
        assert!(corrector.model.vocab.contains("quick"));
    }

    #[test]
    fn test_corrections_index_from_patterns() {
        let mut corrector = NgramCorrector::new();
        corrector.set_corrections_index_from_patterns(&[
            CorrectionPattern {
                original: "teh".to_string(),
                corrected: "the".to_string(),
                frequency: 3,
                confidence: 1.0,
                context_before: None,
                context_after: None,
            },
            CorrectionPattern {
                original: "teh".to_string(),
                corrected: "The".to_string(),
                frequency: 3,
                confidence: 1.0,
                context_before: None,
                context_after: None,
            },
        ]);

        let candidates = corrector.model.corrections_index.get("teh").unwrap();
        assert_eq!(candidates.len(), 2);
    }
}
