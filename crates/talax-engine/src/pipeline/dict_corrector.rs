//! Layer 1: Dictionary-based substitution corrector.
//!
//! Fastest layer -- applies high-confidence corrections from the learned
//! pattern database. Sub-millisecond for typical text lengths.
//! Port of /opt/dictation/server/dict_corrector.py

use regex::Regex;
use rusqlite::Result as SqlResult;

use super::Change;

#[derive(Debug, Clone)]
pub struct SubstitutionRule {
    pub original: String,
    pub corrected: String,
    pub frequency: i64,
    pub confidence: f64,
    pub context_before: Option<String>,
    pub context_after: Option<String>,
    compiled: Option<Regex>,
}

pub struct DictionaryCorrector {
    rules: Vec<SubstitutionRule>,
}

impl Default for DictionaryCorrector {
    fn default() -> Self {
        Self::new()
    }
}

impl DictionaryCorrector {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn try_reload(&mut self, db: &crate::db::Database) -> SqlResult<()> {
        let patterns = db.get_auto_corrections()?;
        self.rules = patterns
            .into_iter()
            .map(|p| {
                let compiled = build_pattern(&p.original, &p.context_before, &p.context_after);
                SubstitutionRule {
                    original: p.original,
                    corrected: p.corrected,
                    frequency: p.frequency,
                    confidence: p.confidence,
                    context_before: p.context_before,
                    context_after: p.context_after,
                    compiled,
                }
            })
            .collect();

        // Sort longest-first to prevent shorter patterns interfering
        self.rules
            .sort_by_key(|rule| std::cmp::Reverse(rule.original.len()));

        Ok(())
    }

    /// Reload rules from the database.
    pub fn reload(&mut self, db: &crate::db::Database) {
        if let Err(err) = self.try_reload(db) {
            tracing::warn!("failed to reload dictionary corrector: {err}");
            self.rules.clear();
        }
    }

    /// Apply dictionary substitutions. Returns (corrected_text, changes).
    pub fn apply(&self, text: &str) -> (String, Vec<Change>) {
        let mut result = text.to_string();
        let mut changes = Vec::new();

        for rule in &self.rules {
            let Some(re) = &rule.compiled else {
                continue;
            };

            // Collect match positions first, then apply in reverse
            let match_ranges: Vec<(usize, usize)> = re
                .find_iter(&result)
                .filter(|m| context_matches(&result, m.start(), m.end(), rule))
                .map(|m| (m.start(), m.end()))
                .collect();

            for (start, end) in match_ranges.into_iter().rev() {
                let original_span = result[start..end].to_string();
                let replacement = case_match(&original_span, &rule.corrected);

                if replacement == original_span {
                    continue;
                }

                result.replace_range(start..end, &replacement);
                changes.push(Change {
                    layer: "dictionary".to_string(),
                    position: Some(start),
                    original: original_span,
                    corrected: replacement,
                    rule_freq: Some(rule.frequency),
                    original_score: None,
                    corrected_score: None,
                });
            }
        }

        (result, changes)
    }
}

/// Build a regex pattern with word boundaries, optionally with context.
fn build_pattern(
    original: &str,
    _context_before: &Option<String>,
    _context_after: &Option<String>,
) -> Option<Regex> {
    let escaped = regex::escape(original);
    Regex::new(&format!(r"(?i)\b{escaped}\b")).ok()
}

fn context_matches(text: &str, start: usize, end: usize, rule: &SubstitutionRule) -> bool {
    let before_ok = rule
        .context_before
        .as_ref()
        .map(|ctx| {
            let expected: Vec<String> = ctx
                .split_whitespace()
                .map(normalize_context_token)
                .filter(|token| !token.is_empty())
                .collect();
            if expected.is_empty() {
                return true;
            }

            let before_tokens = trailing_context_tokens(&text[..start], expected.len());
            before_tokens == expected
        })
        .unwrap_or(true);

    if !before_ok {
        return false;
    }

    rule.context_after
        .as_ref()
        .map(|ctx| {
            let expected: Vec<String> = ctx
                .split_whitespace()
                .map(normalize_context_token)
                .filter(|token| !token.is_empty())
                .collect();
            if expected.is_empty() {
                return true;
            }

            let after_tokens = leading_context_tokens(&text[end..], expected.len());
            after_tokens == expected
        })
        .unwrap_or(true)
}

fn trailing_context_tokens(text: &str, count: usize) -> Vec<String> {
    let tokens: Vec<String> = text
        .split_whitespace()
        .map(normalize_context_token)
        .filter(|token| !token.is_empty())
        .collect();

    let start = tokens.len().saturating_sub(count);
    tokens[start..].to_vec()
}

fn leading_context_tokens(text: &str, count: usize) -> Vec<String> {
    text.split_whitespace()
        .map(normalize_context_token)
        .filter(|token| !token.is_empty())
        .take(count)
        .collect()
}

fn normalize_context_token(token: &str) -> String {
    token
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.')
        .to_lowercase()
}

/// Preserve casing pattern from original where sensible.
fn case_match(original: &str, replacement: &str) -> String {
    if original.is_empty() || replacement.is_empty() {
        return replacement.to_string();
    }
    if original
        .chars()
        .all(|c| c.is_uppercase() || !c.is_alphabetic())
    {
        return replacement.to_uppercase();
    }
    let mut chars = original.chars();
    if let Some(first) = chars.next()
        && first.is_uppercase()
        && chars.all(|c| c.is_lowercase() || !c.is_alphabetic())
    {
        let mut r = replacement.to_string();
        if let Some(c) = r.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        return r;
    }
    replacement.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_case_match_upper() {
        assert_eq!(case_match("GDP", "GCP"), "GCP");
    }

    #[test]
    fn test_case_match_title() {
        assert_eq!(case_match("Hockingface", "Hugging Face"), "Hugging Face");
    }

    #[test]
    fn test_case_match_lower() {
        assert_eq!(case_match("gdp", "GCP"), "GCP");
    }

    #[test]
    fn context_matches_previous_word() {
        let rule = SubstitutionRule {
            original: "v".to_string(),
            corrected: "V".to_string(),
            frequency: 3,
            confidence: 1.0,
            context_before: Some("node".to_string()),
            context_after: None,
            compiled: build_pattern("v", &None, &None),
        };

        assert!(context_matches("node v ready", 5, 6, &rule));
        assert!(!context_matches("worker v ready", 7, 8, &rule));
    }

    #[test]
    fn context_matches_next_word() {
        let rule = SubstitutionRule {
            original: "api".to_string(),
            corrected: "API".to_string(),
            frequency: 3,
            confidence: 1.0,
            context_before: None,
            context_after: Some("server".to_string()),
            compiled: build_pattern("api", &None, &None),
        };

        assert!(context_matches("the api server", 4, 7, &rule));
        assert!(!context_matches("the api client", 4, 7, &rule));
    }
}
