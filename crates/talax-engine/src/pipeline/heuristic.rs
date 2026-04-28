//! Layer 3: Heuristic expander.
//!
//! Replaces the LLM layer with fast algorithmic corrections:
//! - Levenshtein fuzzy matching against known vocabulary
//! - Soundex phonetic matching for accent substitutions
//! - Double Metaphone for superior phonetic matching
//! - Compound word handling (split/join/hyphenate)
//! - Acronym detection and case restoration
//! - Number/code normalization for ASR output
//! - Context-aware candidate scoring

use std::collections::HashMap;

use super::Change;

/// Candidate correction with scoring metadata.
#[derive(Debug, Clone)]
struct Candidate {
    text: String,
    edit_distance: usize,
    phonetic_match: bool,
    length_diff: usize,
    frequency: Option<u32>,
}

impl Candidate {
    /// Compute a composite score. Lower is better.
    fn score(&self) -> f64 {
        let edit_cost = self.edit_distance as f64 * 2.0;
        let phonetic_bonus = if self.phonetic_match { -3.0 } else { 0.0 };
        let length_penalty = self.length_diff as f64 * 0.5;
        let freq_bonus = match self.frequency {
            Some(f) if f > 10 => -2.0,
            Some(f) if f > 3 => -1.0,
            _ => 0.0,
        };
        edit_cost + phonetic_bonus + length_penalty + freq_bonus
    }
}

pub struct HeuristicExpander {
    /// Known proper nouns and their correct casing (lowercase -> correct)
    proper_nouns: HashMap<String, String>,
    /// Known accent patterns: wrong -> correct
    accent_patterns: HashMap<String, String>,
    /// Known acronyms in domain context (lowercase -> correct casing)
    known_acronyms: HashMap<String, String>,
    /// Known compound words: "gpuserver" -> "gpu-server"
    compound_forms: HashMap<String, String>,
    /// Known bigram compounds: ("gpu", "server") -> "gpu-server"
    bigram_compounds: HashMap<(String, String), String>,
    /// Word frequency from n-gram model (lowercase -> count)
    word_frequencies: HashMap<String, u32>,
}

impl HeuristicExpander {
    pub fn new() -> Self {
        Self {
            proper_nouns: HashMap::new(),
            accent_patterns: HashMap::new(),
            known_acronyms: HashMap::new(),
            compound_forms: HashMap::new(),
            bigram_compounds: HashMap::new(),
            word_frequencies: HashMap::new(),
        }
    }

    /// Reload from domain context in the database.
    pub fn reload(&mut self, db: &crate::db::Database) {
        let ctx = db.get_domain_context();
        self.proper_nouns = ctx.proper_nouns;
        self.accent_patterns = ctx.accent_patterns;

        // Derive acronyms, compounds, and bigram compounds from proper nouns
        self.derive_acronyms();
        self.derive_compounds();
    }

    /// Set word frequencies from the n-gram model vocabulary.
    pub fn set_word_frequencies(&mut self, freqs: HashMap<String, u32>) {
        self.word_frequencies = freqs;
    }

    /// Add a known compound form explicitly.
    pub fn add_compound(&mut self, merged: &str, correct: &str) {
        self.compound_forms
            .insert(merged.to_lowercase(), correct.to_string());
    }

    /// Add a known bigram compound explicitly.
    pub fn add_bigram_compound(&mut self, w1: &str, w2: &str, correct: &str) {
        self.bigram_compounds
            .insert((w1.to_lowercase(), w2.to_lowercase()), correct.to_string());
    }

    /// Add a known acronym explicitly.
    pub fn add_acronym(&mut self, acronym: &str) {
        self.known_acronyms
            .insert(acronym.to_lowercase(), acronym.to_string());
    }

    /// Derive acronyms from proper nouns (2-5 letter all-uppercase entries).
    fn derive_acronyms(&mut self) {
        self.known_acronyms.clear();
        for correct in self.proper_nouns.values() {
            let trimmed = correct.trim();
            if trimmed.len() >= 2
                && trimmed.len() <= 5
                && trimmed
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                && trimmed.chars().any(|c| c.is_ascii_uppercase())
            {
                self.known_acronyms
                    .insert(trimmed.to_lowercase(), trimmed.to_string());
            }
        }
    }

    /// Derive compound word forms from proper nouns containing hyphens.
    fn derive_compounds(&mut self) {
        self.compound_forms.clear();
        self.bigram_compounds.clear();
        for correct in self.proper_nouns.values() {
            let trimmed = correct.trim();
            if trimmed.contains('-') {
                let parts: Vec<&str> = trimmed.split('-').collect();
                // Merged form: "gpu-server" -> key "gpuserver"
                let merged: String = parts.iter().map(|p| p.to_lowercase()).collect();
                self.compound_forms.insert(merged, trimmed.to_string());
                // Bigram form: ("gpu", "server") -> "gpu-server"
                if parts.len() == 2 {
                    self.bigram_compounds.insert(
                        (parts[0].to_lowercase(), parts[1].to_lowercase()),
                        trimmed.to_string(),
                    );
                }
            }
        }
    }

    /// Apply all heuristic corrections.
    pub fn apply(&self, text: &str) -> (String, Vec<Change>) {
        let mut words: Vec<String> = text.split_whitespace().map(String::from).collect();
        let mut changes = Vec::new();

        // Phase 0: Number/code normalization (multi-word patterns first)
        self.apply_number_normalization(&mut words, &mut changes);

        // Phase 1: Bigram compound detection (must happen before single-word passes)
        self.apply_bigram_compounds(&mut words, &mut changes);

        // Phase 2: Per-word corrections
        let mut i = 0;
        while i < words.len() {
            let word_lower = words[i].to_lowercase();

            // 2a. Direct accent pattern match
            if let Some(correct) = self.accent_patterns.get(&word_lower) {
                if *correct != words[i] {
                    changes.push(self.make_change(i, &words[i], correct));
                    words[i] = correct.clone();
                    i += 1;
                    continue;
                }
            }

            // 2b. Compound word splitting: "cpuzero" -> "cpu-node-0"
            if let Some(correct) = self.compound_forms.get(&word_lower) {
                if *correct != words[i] {
                    changes.push(self.make_change(i, &words[i], correct));
                    words[i] = correct.clone();
                    i += 1;
                    continue;
                }
            }

            // 2c. Acronym detection and case fix
            if let Some(correct) = self.check_acronym(&words[i]) {
                if correct != words[i] {
                    changes.push(self.make_change(i, &words[i], &correct));
                    words[i] = correct;
                    i += 1;
                    continue;
                }
            }

            // 2d. Context-aware scored matching against proper nouns
            if word_lower.len() >= 3 {
                if let Some(best) = self.best_scored_candidate(&word_lower) {
                    if best.text != words[i] {
                        changes.push(self.make_change(i, &words[i], &best.text));
                        words[i] = best.text;
                        i += 1;
                        continue;
                    }
                }
            }

            // 2e. Case restoration for exact matches
            if let Some(correct) = self.proper_nouns.get(&word_lower) {
                if words[i] != *correct {
                    changes.push(self.make_change(i, &words[i], correct));
                    words[i] = correct.clone();
                }
            }

            i += 1;
        }

        (words.join(" "), changes)
    }

    /// Check if a word is a known acronym that needs case correction.
    fn check_acronym(&self, word: &str) -> Option<String> {
        let lower = word.to_lowercase();
        if let Some(correct) = self.known_acronyms.get(&lower) {
            if word != correct.as_str() {
                return Some(correct.clone());
            }
        }
        None
    }

    /// Find the best candidate correction using composite scoring.
    /// Returns None if no good candidate is found.
    fn best_scored_candidate(&self, word_lower: &str) -> Option<Candidate> {
        let word_metaphone = double_metaphone(word_lower);
        let mut candidates: Vec<Candidate> = Vec::new();

        for (lower, correct) in &self.proper_nouns {
            let edit_dist = levenshtein(word_lower, lower);

            // Skip if too far
            let max_dist = if word_lower.len() <= 4 { 1 } else { 2 };
            if edit_dist > max_dist || edit_dist == 0 {
                continue;
            }

            let target_metaphone = double_metaphone(lower);
            let phonetic_match = metaphone_match(&word_metaphone, &target_metaphone);

            let length_diff = (word_lower.len() as isize - lower.len() as isize).unsigned_abs();

            let freq = self.word_frequencies.get(lower).copied();

            candidates.push(Candidate {
                text: correct.clone(),
                edit_distance: edit_dist,
                phonetic_match,
                length_diff,
                frequency: freq,
            });
        }

        if candidates.is_empty() {
            return None;
        }

        // Sort by score (lower is better)
        candidates.sort_by(|a, b| {
            a.score()
                .partial_cmp(&b.score())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let best = &candidates[0];
        // Only accept if the score is good enough
        if best.score() <= 3.0 {
            Some(candidates.into_iter().next().unwrap())
        } else {
            None
        }
    }

    /// Apply bigram compound detection: adjacent words that form a known compound.
    fn apply_bigram_compounds(&self, words: &mut Vec<String>, changes: &mut Vec<Change>) {
        if words.len() < 2 {
            return;
        }
        let mut i = 0;
        while i + 1 < words.len() {
            let w1_lower = words[i].to_lowercase();
            let w2_lower = words[i + 1].to_lowercase();
            if let Some(compound) = self.bigram_compounds.get(&(w1_lower, w2_lower)) {
                let original = format!("{} {}", words[i], words[i + 1]);
                changes.push(Change {
                    layer: "heuristic".to_string(),
                    position: Some(i),
                    original,
                    corrected: compound.clone(),
                    rule_freq: None,
                    original_score: None,
                    corrected_score: None,
                });
                words[i] = compound.clone();
                words.remove(i + 1);
                // Don't increment i; re-check the new word against the next one
            } else {
                i += 1;
            }
        }
    }

    /// Normalize spoken numbers/codes in the word list.
    ///
    /// Handles patterns like:
    /// - "cpu dash node zero" -> "cpu-node-0" (example)
    /// - "v two" -> "v2"
    /// - "three point five" -> "3.5"
    fn apply_number_normalization(&self, words: &mut Vec<String>, changes: &mut Vec<Change>) {
        // Pass 1: Multi-word number patterns (scan left to right, replace in place)
        // "X point Y" -> "X.Y" (version/decimal numbers)
        let mut i = 0;
        while i + 2 < words.len() {
            let w0 = words[i].to_lowercase();
            let w1 = words[i + 1].to_lowercase();
            let w2 = words[i + 2].to_lowercase();

            if w1 == "point" {
                if let (Some(n0), Some(n2)) = (word_to_digit(&w0), word_to_digit(&w2)) {
                    let replacement = format!("{n0}.{n2}");
                    let original = format!("{} {} {}", words[i], words[i + 1], words[i + 2]);
                    changes.push(Change {
                        layer: "heuristic".to_string(),
                        position: Some(i),
                        original,
                        corrected: replacement.clone(),
                        rule_freq: None,
                        original_score: None,
                        corrected_score: None,
                    });
                    words[i] = replacement;
                    words.remove(i + 2);
                    words.remove(i + 1);
                    continue; // recheck at same position
                }
            }
            i += 1;
        }

        // Pass 2: "v/V + number_word" -> "v<digit>"
        let mut i = 0;
        while i + 1 < words.len() {
            let w0 = words[i].to_lowercase();
            let w1 = words[i + 1].to_lowercase();

            if w0 == "v" || w0 == "version" {
                if let Some(d) = word_to_digit(&w1) {
                    let replacement = format!("v{d}");
                    let original = format!("{} {}", words[i], words[i + 1]);
                    changes.push(Change {
                        layer: "heuristic".to_string(),
                        position: Some(i),
                        original,
                        corrected: replacement.clone(),
                        rule_freq: None,
                        original_score: None,
                        corrected_score: None,
                    });
                    words[i] = replacement;
                    words.remove(i + 1);
                    continue;
                }
            }

            // "dash" between identifiers: "cpu dash node zero" or "node dash n0"
            // Handle "X dash Y [Z]" -> "X-Y[Z]" with optional number word merging
            if w1 == "dash" && i + 2 < words.len() {
                let w2 = words[i + 2].to_lowercase();

                // Try 4-word pattern first: "X dash PREFIX NUMWORD" -> "X-PREFIX<digit>"
                // e.g., "cpu dash node zero" -> "cpu-node-0" (example)
                let (right_part, consume_count) = if i + 3 < words.len() && is_code_prefix(&w2) {
                    let w3 = words[i + 3].to_lowercase();
                    if let Some(d) = word_to_digit(&w3) {
                        (format!("{}{}", w2, d), 4) // consume 4 words
                    } else {
                        (normalize_trailing_number_word(&w2), 3)
                    }
                } else {
                    (normalize_trailing_number_word(&w2), 3)
                };

                let candidate = format!("{}-{}", w0, right_part);
                if self.proper_nouns.contains_key(&candidate.to_lowercase())
                    || self
                        .compound_forms
                        .contains_key(&candidate.to_lowercase().replace('-', ""))
                {
                    let original_parts: Vec<String> =
                        words[i..i + consume_count].iter().cloned().collect();
                    let original = original_parts.join(" ");
                    changes.push(Change {
                        layer: "heuristic".to_string(),
                        position: Some(i),
                        original,
                        corrected: candidate.clone(),
                        rule_freq: None,
                        original_score: None,
                        corrected_score: None,
                    });
                    words[i] = candidate;
                    // Remove consumed words in reverse order
                    for _ in 1..consume_count {
                        words.remove(i + 1);
                    }
                    continue;
                }
            }

            i += 1;
        }

        // Pass 3: Contextual number word -> digit in code-like positions.
        // If a number word appears adjacent to an alphanumeric token, convert it.
        // e.g., "n zero" -> "n0", "storage one" -> "storage-node-1"
        let mut i = 0;
        while i + 1 < words.len() {
            let w0 = words[i].to_lowercase();
            let w1 = words[i + 1].to_lowercase();

            // Pattern: short alpha token + number word -> merged identifier
            if is_code_prefix(&w0) {
                if let Some(d) = word_to_digit(&w1) {
                    let merged = format!("{}{}", w0, d);
                    // Only merge if the result is a known proper noun or looks like a code
                    if self.proper_nouns.contains_key(&merged) || is_likely_identifier(&merged) {
                        let correct = self
                            .proper_nouns
                            .get(&merged)
                            .cloned()
                            .unwrap_or_else(|| merged.clone());
                        let original = format!("{} {}", words[i], words[i + 1]);
                        changes.push(Change {
                            layer: "heuristic".to_string(),
                            position: Some(i),
                            original,
                            corrected: correct.clone(),
                            rule_freq: None,
                            original_score: None,
                            corrected_score: None,
                        });
                        words[i] = correct;
                        words.remove(i + 1);
                        continue;
                    }
                }
            }

            i += 1;
        }
    }

    /// Helper to build a Change record.
    fn make_change(&self, position: usize, original: &str, corrected: &str) -> Change {
        Change {
            layer: "heuristic".to_string(),
            position: Some(position),
            original: original.to_string(),
            corrected: corrected.to_string(),
            rule_freq: None,
            original_score: None,
            corrected_score: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Number/code utility functions
// ---------------------------------------------------------------------------

/// Map spoken number words to their digit character.
fn word_to_digit(word: &str) -> Option<char> {
    match word.to_lowercase().as_str() {
        "zero" | "oh" => Some('0'),
        "one" => Some('1'),
        "two" | "to" | "too" => Some('2'),
        "three" => Some('3'),
        "four" | "for" => Some('4'),
        "five" => Some('5'),
        "six" => Some('6'),
        "seven" => Some('7'),
        "eight" => Some('8'),
        "nine" => Some('9'),
        _ => None,
    }
}

/// If a word is a number word, return the version with the digit; otherwise return as-is.
fn normalize_trailing_number_word(word: &str) -> String {
    if let Some(d) = word_to_digit(word) {
        d.to_string()
    } else {
        word.to_string()
    }
}

/// Check if a string looks like a short code prefix (1-4 alpha chars).
fn is_code_prefix(s: &str) -> bool {
    !s.is_empty() && s.len() <= 4 && s.chars().all(|c| c.is_ascii_alphabetic())
}

/// Check if a string looks like an identifier (letters followed by digits or vice versa).
fn is_likely_identifier(s: &str) -> bool {
    let has_alpha = s.chars().any(|c| c.is_ascii_alphabetic());
    let has_digit = s.chars().any(|c| c.is_ascii_digit());
    has_alpha && has_digit && s.len() <= 10
}

// ---------------------------------------------------------------------------
// Levenshtein edit distance
// ---------------------------------------------------------------------------

/// Compute Levenshtein edit distance between two strings.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());

    let mut prev = (0..=n).collect::<Vec<_>>();
    let mut curr = vec![0; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

// ---------------------------------------------------------------------------
// Soundex
// ---------------------------------------------------------------------------

/// American Soundex algorithm for phonetic matching.
pub fn soundex(word: &str) -> String {
    let word = word.to_uppercase();
    let chars: Vec<char> = word.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if chars.is_empty() {
        return "0000".to_string();
    }

    let first = chars[0];
    let mut code = String::from(first);

    let to_code = |c: char| -> char {
        match c {
            'B' | 'F' | 'P' | 'V' => '1',
            'C' | 'G' | 'J' | 'K' | 'Q' | 'S' | 'X' | 'Z' => '2',
            'D' | 'T' => '3',
            'L' => '4',
            'M' | 'N' => '5',
            'R' => '6',
            _ => '0',
        }
    };

    let mut last_code = to_code(first);

    for &ch in &chars[1..] {
        let c = to_code(ch);
        if c != '0' && c != last_code {
            code.push(c);
            if code.len() == 4 {
                break;
            }
        }
        last_code = c;
    }

    while code.len() < 4 {
        code.push('0');
    }

    code
}

// ---------------------------------------------------------------------------
// Double Metaphone
// ---------------------------------------------------------------------------

/// Result of the Double Metaphone algorithm: primary and alternate codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaphoneResult {
    pub primary: String,
    pub alternate: String,
}

/// Check whether two Metaphone results have any overlapping code.
pub fn metaphone_match(a: &MetaphoneResult, b: &MetaphoneResult) -> bool {
    if a.primary.is_empty() || b.primary.is_empty() {
        return false;
    }
    if a.primary == b.primary {
        return true;
    }
    if !a.alternate.is_empty() && a.alternate == b.primary {
        return true;
    }
    if !b.alternate.is_empty() && a.primary == b.alternate {
        return true;
    }
    if !a.alternate.is_empty() && !b.alternate.is_empty() && a.alternate == b.alternate {
        return true;
    }
    false
}

/// Double Metaphone algorithm.
///
/// Produces a primary and alternate phonetic encoding. Handles:
/// - Silent letters (knife -> NF, psychology -> SKLJ)
/// - Digraphs (th -> 0/T, sh -> X, ch -> X/K)
/// - Variable pronunciation of C before e/i vs others
/// - Germanic/Slavic name patterns
pub fn double_metaphone(input: &str) -> MetaphoneResult {
    let word: String = input.to_uppercase();
    let chars: Vec<char> = word.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    let length = chars.len();

    if length == 0 {
        return MetaphoneResult {
            primary: String::new(),
            alternate: String::new(),
        };
    }

    let mut primary = String::new();
    let mut alternate = String::new();
    let mut pos: usize = 0;
    let max_len = 4; // Standard Metaphone code length

    let at = |i: usize| -> char { if i < length { chars[i] } else { '\0' } };

    let string_at = |start: usize, len: usize, targets: &[&str]| -> bool {
        if start + len > length {
            return false;
        }
        let substr: String = chars[start..start + len].iter().collect();
        targets.iter().any(|t| t.to_uppercase() == substr)
    };

    let is_vowel = |c: char| matches!(c, 'A' | 'E' | 'I' | 'O' | 'U');

    // Skip silent leading letters
    if string_at(0, 2, &["GN", "KN", "PN", "PS", "AE", "WR"]) {
        pos = 1;
    }

    // Special: leading X -> S
    if at(0) == 'X' {
        primary.push('S');
        alternate.push('S');
        pos = 1;
    }

    while pos < length && (primary.len() < max_len || alternate.len() < max_len) {
        let ch = at(pos);

        // Skip vowels unless at start
        if is_vowel(ch) {
            if pos == 0 {
                primary.push('A');
                alternate.push('A');
            }
            pos += 1;
            continue;
        }

        match ch {
            'B' => {
                primary.push('P');
                alternate.push('P');
                pos += if at(pos + 1) == 'B' { 2 } else { 1 };
            }
            'C' => {
                // Various C rules
                if pos > 1
                    && !is_vowel(at(pos - 2))
                    && string_at(pos - 1, 3, &["ACH"])
                    && at(pos + 2) != 'I'
                    && (at(pos + 2) != 'E' || string_at(pos - 2, 6, &["BACHER", "MACHER"]))
                {
                    primary.push('K');
                    alternate.push('K');
                    pos += 2;
                } else if pos == 0 && string_at(pos, 6, &["CAESAR"]) {
                    primary.push('S');
                    alternate.push('S');
                    pos += 2;
                } else if string_at(pos, 2, &["CH"]) {
                    // CH handling
                    primary.push('X');
                    alternate.push('X');
                    pos += 2;
                } else if string_at(pos, 2, &["CZ"]) {
                    primary.push('S');
                    alternate.push('X');
                    pos += 2;
                } else if string_at(pos, 3, &["CIA"]) {
                    primary.push('X');
                    alternate.push('X');
                    pos += 3;
                } else if string_at(pos, 2, &["CC"]) && !(pos == 1 && at(0) == 'M') {
                    if matches!(at(pos + 2), 'I' | 'E' | 'H') {
                        primary.push('K');
                        alternate.push('K');
                        primary.push('S');
                        alternate.push('S');
                    } else {
                        primary.push('K');
                        alternate.push('K');
                    }
                    pos += 2;
                } else if string_at(pos, 2, &["CK", "CG", "CQ"]) {
                    primary.push('K');
                    alternate.push('K');
                    pos += 2;
                } else if string_at(pos, 2, &["CI", "CE", "CY"]) {
                    primary.push('S');
                    alternate.push('S');
                    pos += 2;
                } else {
                    primary.push('K');
                    alternate.push('K');
                    pos += if string_at(pos + 1, 1, &["C", "K", "Q"]) {
                        2
                    } else {
                        1
                    };
                }
            }
            'D' => {
                if string_at(pos, 2, &["DG"]) {
                    if matches!(at(pos + 2), 'I' | 'E' | 'Y') {
                        primary.push('J');
                        alternate.push('J');
                        pos += 3;
                    } else {
                        primary.push('T');
                        alternate.push('T');
                        primary.push('K');
                        alternate.push('K');
                        pos += 2;
                    }
                } else if string_at(pos, 2, &["DT", "DD"]) {
                    primary.push('T');
                    alternate.push('T');
                    pos += 2;
                } else {
                    primary.push('T');
                    alternate.push('T');
                    pos += 1;
                }
            }
            'F' => {
                primary.push('F');
                alternate.push('F');
                pos += if at(pos + 1) == 'F' { 2 } else { 1 };
            }
            'G' => {
                if at(pos + 1) == 'H' {
                    if pos > 0 && !is_vowel(at(pos - 1)) {
                        primary.push('K');
                        alternate.push('K');
                        pos += 2;
                    } else if pos == 0 {
                        if at(pos + 2) == 'I' {
                            primary.push('J');
                            alternate.push('J');
                        } else {
                            primary.push('K');
                            alternate.push('K');
                        }
                        pos += 2;
                    } else {
                        // GH silent after vowel
                        pos += 2;
                    }
                } else if at(pos + 1) == 'N' {
                    // GN - G is silent
                    pos += if at(pos + 2) == 'E' || at(pos + 2) == '\0' {
                        2
                    } else {
                        1
                    };
                } else if matches!(at(pos + 1), 'E' | 'I' | 'Y') {
                    primary.push('K');
                    alternate.push('J');
                    pos += 2;
                } else {
                    if at(pos + 1) != '\0' || pos == 0 {
                        primary.push('K');
                        alternate.push('K');
                    }
                    pos += if at(pos + 1) == 'G' { 2 } else { 1 };
                }
            }
            'H' => {
                // H only if before a vowel and not after a vowel
                if is_vowel(at(pos + 1)) && (pos == 0 || !is_vowel(at(pos - 1))) {
                    primary.push('H');
                    alternate.push('H');
                    pos += 2;
                } else {
                    pos += 1;
                }
            }
            'J' => {
                primary.push('J');
                alternate.push('H'); // Spanish J
                pos += if at(pos + 1) == 'J' { 2 } else { 1 };
            }
            'K' => {
                primary.push('K');
                alternate.push('K');
                pos += if at(pos + 1) == 'K' { 2 } else { 1 };
            }
            'L' => {
                primary.push('L');
                alternate.push('L');
                pos += if at(pos + 1) == 'L' { 2 } else { 1 };
            }
            'M' => {
                primary.push('M');
                alternate.push('M');
                pos += if at(pos + 1) == 'M' { 2 } else { 1 };
            }
            'N' => {
                primary.push('N');
                alternate.push('N');
                pos += if at(pos + 1) == 'N' { 2 } else { 1 };
            }
            'P' => {
                if at(pos + 1) == 'H' {
                    primary.push('F');
                    alternate.push('F');
                    pos += 2;
                } else {
                    primary.push('P');
                    alternate.push('P');
                    pos += if at(pos + 1) == 'P' { 2 } else { 1 };
                }
            }
            'Q' => {
                primary.push('K');
                alternate.push('K');
                pos += if at(pos + 1) == 'U' { 2 } else { 1 };
            }
            'R' => {
                primary.push('R');
                alternate.push('R');
                pos += if at(pos + 1) == 'R' { 2 } else { 1 };
            }
            'S' => {
                if string_at(pos, 2, &["SH"]) {
                    primary.push('X');
                    alternate.push('X');
                    pos += 2;
                } else if string_at(pos, 3, &["SIO", "SIA"]) {
                    primary.push('X');
                    alternate.push('S');
                    pos += 3;
                } else if string_at(pos, 2, &["SC"]) {
                    if matches!(at(pos + 2), 'E' | 'I' | 'Y') {
                        primary.push('S');
                        alternate.push('S');
                    } else {
                        primary.push('S');
                        alternate.push('S');
                        primary.push('K');
                        alternate.push('K');
                    }
                    pos += 3;
                } else if string_at(pos, 2, &["SS", "SZ"]) {
                    primary.push('S');
                    alternate.push('S');
                    pos += 2;
                } else {
                    primary.push('S');
                    alternate.push('S');
                    pos += 1;
                }
            }
            'T' => {
                if string_at(pos, 2, &["TH"]) {
                    primary.push('0'); // theta
                    alternate.push('T');
                    pos += 2;
                } else if string_at(pos, 4, &["TION"]) {
                    primary.push('X');
                    alternate.push('X');
                    pos += 4;
                } else if string_at(pos, 2, &["TT", "TD"]) {
                    primary.push('T');
                    alternate.push('T');
                    pos += 2;
                } else {
                    primary.push('T');
                    alternate.push('T');
                    pos += 1;
                }
            }
            'V' => {
                primary.push('F');
                alternate.push('F');
                pos += if at(pos + 1) == 'V' { 2 } else { 1 };
            }
            'W' => {
                if is_vowel(at(pos + 1)) {
                    primary.push('A');
                    alternate.push('F');
                    pos += 1;
                } else {
                    pos += 1;
                }
            }
            'X' => {
                primary.push('K');
                alternate.push('K');
                primary.push('S');
                alternate.push('S');
                pos += if at(pos + 1) == 'X' { 2 } else { 1 };
            }
            'Z' => {
                primary.push('S');
                alternate.push('T');
                pos += if at(pos + 1) == 'Z' { 2 } else { 1 };
            }
            _ => {
                pos += 1;
            }
        }
    }

    // Truncate to max_len
    primary.truncate(max_len);
    alternate.truncate(max_len);

    MetaphoneResult { primary, alternate }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // === Levenshtein ===

    #[test]
    fn test_levenshtein() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("saturday", "sunday"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("same", "same"), 0);
    }

    #[test]
    fn test_levenshtein_empty() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", ""), 3);
    }

    // === Soundex ===

    #[test]
    fn test_soundex() {
        assert_eq!(soundex("Robert"), "R163");
        assert_eq!(soundex("Rupert"), "R163");
        assert_eq!(soundex("Ashcraft"), "A226");
    }

    #[test]
    fn test_soundex_empty() {
        assert_eq!(soundex(""), "0000");
    }

    // === Double Metaphone ===

    #[test]
    fn test_metaphone_basic() {
        // Simple words
        let r = double_metaphone("smith");
        assert_eq!(r.primary, "SM0"); // SM + TH->0
    }

    #[test]
    fn test_metaphone_silent_k() {
        // "knife" - silent K, starts at N
        let r = double_metaphone("knife");
        assert_eq!(r.primary.chars().next(), Some('N'));
    }

    #[test]
    fn test_metaphone_psychology() {
        // "psychology" - silent P, starts with S
        let r = double_metaphone("psychology");
        assert_eq!(r.primary.chars().next(), Some('S'));
    }

    #[test]
    fn test_metaphone_sh_digraph() {
        let r = double_metaphone("shaw");
        assert!(
            r.primary.starts_with('X'),
            "SH should map to X, got {}",
            r.primary
        );
    }

    #[test]
    fn test_metaphone_th_digraph() {
        let r = double_metaphone("the");
        // TH -> 0 (theta) in primary
        assert!(
            r.primary.contains('0'),
            "TH should produce 0, got {}",
            r.primary
        );
    }

    #[test]
    fn test_metaphone_c_before_e_vs_other() {
        // "cell" - C before E -> S
        let r_cell = double_metaphone("cell");
        assert!(
            r_cell.primary.starts_with('S'),
            "C before E should be S, got {}",
            r_cell.primary
        );

        // "cat" - C before A -> K
        let r_cat = double_metaphone("cat");
        assert!(
            r_cat.primary.starts_with('K'),
            "C before A should be K, got {}",
            r_cat.primary
        );
    }

    #[test]
    fn test_metaphone_ph() {
        let r = double_metaphone("phone");
        assert!(
            r.primary.starts_with('F'),
            "PH should map to F, got {}",
            r.primary
        );
    }

    #[test]
    fn test_metaphone_matching() {
        // "Smith" and "Smyth" should match phonetically
        let a = double_metaphone("smith");
        let b = double_metaphone("smyth");
        assert!(
            metaphone_match(&a, &b),
            "smith ({:?}) and smyth ({:?}) should match",
            a,
            b
        );
    }

    #[test]
    fn test_metaphone_no_match() {
        let a = double_metaphone("cat");
        let b = double_metaphone("dog");
        assert!(!metaphone_match(&a, &b));
    }

    #[test]
    fn test_metaphone_empty() {
        let r = double_metaphone("");
        assert_eq!(r.primary, "");
        assert_eq!(r.alternate, "");
    }

    // === Compound word handler ===

    #[test]
    fn test_compound_split_merged_word() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("gpu-server".to_string(), "gpu-server".to_string());
        exp.derive_compounds();

        let (result, changes) = exp.apply("check gpuserver status");
        assert_eq!(result, "check gpu-server status");
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].original, "gpuserver");
        assert_eq!(changes[0].corrected, "gpu-server");
    }

    #[test]
    fn test_compound_bigram_join() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("gpu-server".to_string(), "gpu-server".to_string());
        exp.derive_compounds();

        let (result, changes) = exp.apply("restart gpu server now");
        assert_eq!(result, "restart gpu-server now");
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].original, "gpu server");
        assert_eq!(changes[0].corrected, "gpu-server");
    }

    #[test]
    fn test_compound_node_zero() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("cpu-node-0".to_string(), "cpu-node-0".to_string());
        exp.derive_compounds();

        // Merged form
        let (result, _) = exp.apply("check cpunode0 status");
        assert_eq!(result, "check cpu-node-0 status");
    }

    #[test]
    fn test_compound_explicit_add() {
        let mut exp = HeuristicExpander::new();
        exp.add_compound("cpuzero", "cpu-node-0");

        let (result, changes) = exp.apply("ssh into cpuzero");
        assert_eq!(result, "ssh into cpu-node-0");
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn test_bigram_compound_explicit() {
        let mut exp = HeuristicExpander::new();
        exp.add_bigram_compound("uptime", "kuma", "Uptime-Kuma");

        let (result, changes) = exp.apply("check uptime kuma status");
        assert_eq!(result, "check Uptime-Kuma status");
        assert_eq!(changes.len(), 1);
    }

    // === Acronym detection ===

    #[test]
    fn test_acronym_lowercase_to_upper() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("gpu".to_string(), "GPU".to_string());
        exp.derive_acronyms();

        let (result, changes) = exp.apply("check the gpu status");
        assert_eq!(result, "check the GPU status");
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].original, "gpu");
        assert_eq!(changes[0].corrected, "GPU");
    }

    #[test]
    fn test_acronym_mixed_case() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("vram".to_string(), "VRAM".to_string());
        exp.derive_acronyms();

        let (result, _) = exp.apply("total Vram usage");
        assert_eq!(result, "total VRAM usage");
    }

    #[test]
    fn test_acronym_already_correct() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("gpu".to_string(), "GPU".to_string());
        exp.derive_acronyms();

        let (result, changes) = exp.apply("the GPU is busy");
        assert_eq!(result, "the GPU is busy");
        assert!(changes.is_empty());
    }

    #[test]
    fn test_acronym_explicit_add() {
        let mut exp = HeuristicExpander::new();
        exp.add_acronym("NVMe");

        let (result, _) = exp.apply("check nvme drives");
        assert_eq!(result, "check NVMe drives");
    }

    #[test]
    fn test_acronym_with_digits() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("cx6".to_string(), "CX6".to_string());
        exp.derive_acronyms();

        let (result, _) = exp.apply("the cx6 interface");
        assert_eq!(result, "the CX6 interface");
    }

    // === Number/code normalization ===

    #[test]
    fn test_number_version_v_two() {
        let exp = HeuristicExpander::new();
        let (result, changes) = exp.apply("running v two now");
        assert_eq!(result, "running v2 now");
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].original, "v two");
        assert_eq!(changes[0].corrected, "v2");
    }

    #[test]
    fn test_number_version_word() {
        let exp = HeuristicExpander::new();
        let (result, _) = exp.apply("use version three");
        assert_eq!(result, "use v3");
    }

    #[test]
    fn test_number_decimal() {
        let exp = HeuristicExpander::new();
        let (result, changes) = exp.apply("three point five gigahertz");
        assert_eq!(result, "3.5 gigahertz");
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].corrected, "3.5");
    }

    #[test]
    fn test_number_dash_pattern() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("cpu-node-0".to_string(), "cpu-node-0".to_string());
        exp.derive_compounds();

        let (result, _) = exp.apply("ssh cpu dash node zero");
        // "cpu dash node zero" -> first pass doesn't know "n zero" directly,
        // but "n" + "zero" -> pass 3 merges "n" + "zero" -> "n0" if known
        // and "cpu dash n0" -> matched through dash pattern
        // The exact handling depends on ordering, but the compound should be detected.
        assert!(
            result.contains("cpu-node-0"),
            "Expected cpu-node-0 in result, got: {}",
            result
        );
    }

    #[test]
    fn test_number_code_prefix_merge() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("node1".to_string(), "node1".to_string());

        let (result, changes) = exp.apply("check node one status");
        assert_eq!(result, "check node1 status");
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn test_number_n_zero() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns.insert("n0".to_string(), "n0".to_string());

        let (result, _) = exp.apply("node n zero is down");
        assert_eq!(result, "node n0 is down");
    }

    // === Context-aware scoring ===

    #[test]
    fn test_scored_matching_edit_distance() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("tensor".to_string(), "Tensor".to_string());

        // "tenser" is edit distance 1 from "tensor"
        let (result, changes) = exp.apply("the tenser module");
        assert_eq!(result, "the Tensor module");
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn test_scored_matching_phonetic_bonus() {
        let mut exp = HeuristicExpander::new();
        // Two candidates at same edit distance, one with phonetic match
        exp.proper_nouns
            .insert("grafana".to_string(), "Grafana".to_string());

        let (result, _) = exp.apply("check grafena dashboard");
        assert_eq!(result, "check Grafana dashboard");
    }

    #[test]
    fn test_scored_matching_frequency_bonus() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("ceph".to_string(), "Ceph".to_string());
        exp.proper_nouns
            .insert("chef".to_string(), "Chef".to_string());
        // Give Ceph a high frequency, so it wins over Chef even though both
        // are edit distance 1 from "cesh"
        let mut freqs = HashMap::new();
        freqs.insert("ceph".to_string(), 50);
        freqs.insert("chef".to_string(), 1);
        exp.set_word_frequencies(freqs);

        let (result, _) = exp.apply("the cesh cluster");
        assert_eq!(result, "the Ceph cluster");
    }

    #[test]
    fn test_scored_no_correction_when_too_far() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("prometheus".to_string(), "Prometheus".to_string());

        // "problem" is too far from "prometheus"
        let (result, changes) = exp.apply("this is a problem");
        assert_eq!(result, "this is a problem");
        assert!(changes.is_empty());
    }

    // === Accent pattern pass-through ===

    #[test]
    fn test_accent_pattern() {
        let mut exp = HeuristicExpander::new();
        exp.accent_patterns
            .insert("hockingface".to_string(), "Hugging Face".to_string());

        let (result, changes) = exp.apply("use hockingface models");
        assert_eq!(result, "use Hugging Face models");
        assert_eq!(changes.len(), 1);
    }

    // === Case restoration ===

    #[test]
    fn test_case_restoration() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("nvidia".to_string(), "NVIDIA".to_string());

        let (result, changes) = exp.apply("the nvidia driver");
        // Acronym detection should handle this
        assert_eq!(result, "the NVIDIA driver");
        assert_eq!(changes.len(), 1);
    }

    // === Integration / combined passes ===

    #[test]
    fn test_combined_acronym_and_compound() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("gpu".to_string(), "GPU".to_string());
        exp.proper_nouns
            .insert("gpu-server".to_string(), "gpu-server".to_string());
        exp.derive_acronyms();
        exp.derive_compounds();

        let (result, changes) = exp.apply("gpu on gpu server");
        assert_eq!(result, "GPU on gpu-server");
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn test_combined_number_and_acronym() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("node1".to_string(), "node1".to_string());
        exp.add_acronym("GPU");

        let (result, _) = exp.apply("gpu on node one");
        assert_eq!(result, "GPU on node1");
    }

    #[test]
    fn test_no_changes_on_correct_text() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("gpu".to_string(), "GPU".to_string());
        exp.proper_nouns
            .insert("gpu-server".to_string(), "gpu-server".to_string());
        exp.derive_acronyms();
        exp.derive_compounds();

        let (result, changes) = exp.apply("GPU on gpu-server");
        assert_eq!(result, "GPU on gpu-server");
        assert!(changes.is_empty());
    }

    #[test]
    fn test_empty_input() {
        let exp = HeuristicExpander::new();
        let (result, changes) = exp.apply("");
        assert_eq!(result, "");
        assert!(changes.is_empty());
    }

    #[test]
    fn test_single_word_input() {
        let mut exp = HeuristicExpander::new();
        exp.proper_nouns
            .insert("gpu".to_string(), "GPU".to_string());
        exp.derive_acronyms();

        let (result, _) = exp.apply("gpu");
        assert_eq!(result, "GPU");
    }

    // === word_to_digit helper ===

    #[test]
    fn test_word_to_digit() {
        assert_eq!(word_to_digit("zero"), Some('0'));
        assert_eq!(word_to_digit("one"), Some('1'));
        assert_eq!(word_to_digit("two"), Some('2'));
        assert_eq!(word_to_digit("three"), Some('3'));
        assert_eq!(word_to_digit("four"), Some('4'));
        assert_eq!(word_to_digit("five"), Some('5'));
        assert_eq!(word_to_digit("six"), Some('6'));
        assert_eq!(word_to_digit("seven"), Some('7'));
        assert_eq!(word_to_digit("eight"), Some('8'));
        assert_eq!(word_to_digit("nine"), Some('9'));
        assert_eq!(word_to_digit("oh"), Some('0'));
        assert_eq!(word_to_digit("hello"), None);
    }

    // === is_code_prefix / is_likely_identifier helpers ===

    #[test]
    fn test_is_code_prefix() {
        assert!(is_code_prefix("n"));
        assert!(is_code_prefix("node"));
        assert!(is_code_prefix("srv"));
        assert!(!is_code_prefix(""));
        assert!(!is_code_prefix("12345"));
        assert!(!is_code_prefix("verylongprefix"));
    }

    #[test]
    fn test_is_likely_identifier() {
        assert!(is_likely_identifier("node1"));
        assert!(is_likely_identifier("n0"));
        assert!(is_likely_identifier("v2"));
        assert!(!is_likely_identifier("hello"));
        assert!(!is_likely_identifier("123"));
    }
}
