//! Original token spans let correction layers preserve untouched whitespace.

use std::ops::Range;

pub(super) struct WordLayout<'a> {
    text: &'a str,
    spans: Vec<Range<usize>>,
}

impl<'a> WordLayout<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        let mut cursor = 0;
        let spans = text
            .split_whitespace()
            .map(|word| {
                let start = cursor + text[cursor..].find(word).expect("token in source");
                cursor = start + word.len();
                start..cursor
            })
            .collect();
        Self { text, spans }
    }

    /// A phrase replaced by one token consumes only its internal separators.
    pub(super) fn merge(&mut self, first: usize, count: usize) {
        self.spans[first].end = self.spans[first + count - 1].end;
        self.spans.drain(first + 1..first + count);
    }

    pub(super) fn render(&self, words: &[String]) -> String {
        assert_eq!(self.spans.len(), words.len());
        let mut result = String::with_capacity(self.text.len());
        let mut cursor = 0;
        for (span, word) in self.spans.iter().zip(words) {
            result.push_str(&self.text[cursor..span.start]);
            result.push_str(word);
            cursor = span.end;
        }
        result.push_str(&self.text[cursor..]);
        result
    }
}
