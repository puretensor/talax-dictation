//! Layer 1: Dictionary-based substitution corrector.
//!
//! Fastest layer -- applies high-confidence corrections from the learned
//! pattern database. Sub-millisecond for typical text lengths.

use std::sync::LazyLock;

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
            let match_ranges: Vec<(usize, usize)> = find_bounded(re, &result)
                .into_iter()
                .filter(|&(start, end)| context_matches(&result, start, end, rule))
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

/// A word character is exactly what the regex crate's `\w` matches (letters,
/// decimal digits, connector punctuation, combining marks, join controls), so
/// ordinary rules keep the boundaries `\b` gave them. The check is a
/// precompiled anchored regex on the single neighbour character.
fn is_word_char(c: char) -> bool {
    static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\w$").expect("valid regex"));
    let mut buf = [0u8; 4];
    WORD.is_match(c.encode_utf8(&mut buf))
}

/// Find matches of `re` whose neighbours are not word characters (or the text
/// edge). This equals `\b` on a side where the pattern ends in a word character
/// (so `api` cannot match inside `rapid`), and it also works where the pattern
/// ends in a symbol (`c++`, `.net`, `c#`), which `\b` can never satisfy next to
/// a space. The regex crate has no lookaround, so the check is done by hand.
fn find_bounded(re: &Regex, text: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut pos = 0;
    while pos <= text.len() {
        let Some(m) = re.find_at(text, pos) else {
            break;
        };
        let before_ok = text[..m.start()]
            .chars()
            .next_back()
            .is_none_or(|c| !is_word_char(c));
        let after_ok = text[m.end()..]
            .chars()
            .next()
            .is_none_or(|c| !is_word_char(c));
        if before_ok && after_ok {
            found.push((m.start(), m.end()));
            pos = if m.end() > m.start() {
                m.end()
            } else {
                m.end() + text[m.end()..].chars().next().map_or(1, char::len_utf8)
            };
        } else {
            // Retry from the next character so a rejected match cannot hide a valid one.
            pos = m.start() + text[m.start()..].chars().next().map_or(1, char::len_utf8);
        }
    }
    found
}

/// Build the (case-insensitive) pattern for a rule. Word boundaries are applied
/// by `find_bounded`, not by `\b`, so rules with symbol edges can match.
fn build_pattern(
    original: &str,
    _context_before: &Option<String>,
    _context_after: &Option<String>,
) -> Option<Regex> {
    let escaped = regex::escape(original);
    Regex::new(&format!(r"(?i){escaped}")).ok()
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

    fn corrected(original: &str, corrected: &str, text: &str) -> String {
        let mut corrector = DictionaryCorrector::new();
        corrector.rules.push(SubstitutionRule {
            original: original.to_string(),
            corrected: corrected.to_string(),
            frequency: 3,
            confidence: 1.0,
            context_before: None,
            context_after: None,
            compiled: build_pattern(original, &None, &None),
        });
        corrector.apply(text).0
    }

    #[test]
    fn symbol_edged_rules_match() {
        assert_eq!(
            corrected("c++", "C++", "I use c++ daily"),
            "I use C++ daily"
        );
        assert_eq!(corrected("c++", "C++", "c++"), "C++");
        assert_eq!(corrected("c++", "C++", "c++, c++."), "C++, C++.");
        assert_eq!(
            corrected(".net", ".NET", "the .net runtime"),
            "the .NET runtime"
        );
        assert_eq!(corrected("c#", "C#", "write c# code"), "write C# code");
        assert_eq!(
            corrected("node.js", "Node.js", "use node.js now"),
            "use Node.js now"
        );
    }

    #[test]
    fn rules_do_not_fire_inside_longer_words() {
        assert_eq!(corrected("api", "API", "rapid apis api"), "rapid apis API");
        assert_eq!(corrected("c++", "C++", "abc++ c++x"), "abc++ c++x");
        assert_eq!(corrected("c#", "C#", "ac# c#s"), "ac# c#s");
        assert_eq!(corrected(".net", ".NET", "asp.net .netx"), "asp.net .netx");
        assert_eq!(
            corrected("node.js", "Node.js", "xnode.js node.jsx"),
            "xnode.js node.jsx"
        );
    }

    #[test]
    fn word_char_agrees_with_regex_w() {
        let w = Regex::new(r"^\w$").unwrap();
        for c in ('\u{0}'..='\u{2FFF}').chain('\u{A000}'..='\u{FFFF}') {
            assert_eq!(
                is_word_char(c),
                w.is_match(&c.to_string()),
                "U+{:04X}",
                c as u32
            );
        }
        // letters, digits, underscore, connector, combining mark, join controls
        for c in [
            'a', 'Z', 'é', '7', '_', '\u{203F}', '\u{0301}', '\u{200C}', '\u{200D}',
        ] {
            assert!(is_word_char(c), "{c:?}");
        }
        // superscript two, vulgar fraction, punctuation, space
        for c in ['\u{00B2}', '\u{00BD}', '+', '.', ' ', '#'] {
            assert!(!is_word_char(c), "{c:?}");
        }
    }

    #[test]
    fn combining_mark_and_connector_neighbours_are_word_chars() {
        assert_eq!(
            corrected("cafe", "CAFE", "cafe\u{0301} cafe"),
            "cafe\u{0301} CAFE"
        );
        assert_eq!(
            corrected("cafe", "CAFE", "cafe\u{203F}x cafe"),
            "cafe\u{203F}x CAFE"
        );
        assert_eq!(corrected("cafe", "CAFE", "\u{203F}cafe"), "\u{203F}cafe");
    }

    #[test]
    fn superscript_digit_is_not_a_word_char() {
        assert_eq!(corrected("m", "M", "m\u{00B2}"), "M\u{00B2}");
        assert_eq!(corrected("m", "M", "\u{00B2}m"), "\u{00B2}M");
        assert_eq!(corrected("m", "M", "m2 m"), "m2 M");
    }

    #[test]
    fn symbol_edged_rule_needs_the_whole_token() {
        // A taught rule fires on the token the user taught, not inside a
        // longer token: ".net" is left alone in "asp.net", "@bob" in "a@bob".
        assert_eq!(corrected(".net", ".NET", "asp.net"), "asp.net");
        assert_eq!(corrected("@bob", "@Bob", "a@bob"), "a@bob");
        assert_eq!(corrected("@bob", "@Bob", "hi @bob!"), "hi @Bob!");
    }

    #[test]
    fn rejected_match_does_not_hide_a_later_one() {
        assert_eq!(corrected("aa", "B", "aaa aa"), "aaa B");
        assert_eq!(corrected("c++", "C++", "abc++ c++"), "abc++ C++");
    }

    #[test]
    fn case_handling_is_unchanged_for_symbol_rules() {
        assert_eq!(corrected("c++", "c++", "C++ is C++"), "C++ is C++");
        assert_eq!(corrected("gdp", "GCP", "GDP and gdp"), "GCP and GCP");
    }
}
