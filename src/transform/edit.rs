//! Span edits: how a normalization rule changes a line.
//!
//! A rule never rebuilds a line from tokens.  It finds the byte ranges it wants
//! to change and replaces exactly those, so every byte it did not name survives
//! untouched.  That is what makes I3 — protected source preserved byte for byte
//! — a property of the mechanism rather than of each rule's care.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Edit {
    range: Range<usize>,
    text: Vec<u8>,
}

/// A pending set of replacements over one line.
#[derive(Debug, Clone)]
pub struct EditBuffer<'a> {
    source: &'a [u8],
    edits: Vec<Edit>,
    skipped: usize,
}

impl<'a> EditBuffer<'a> {
    pub fn new(source: &'a [u8]) -> Self {
        Self {
            source,
            edits: Vec::new(),
            skipped: 0,
        }
    }

    pub fn source(&self) -> &'a [u8] {
        self.source
    }

    /// Replace `range` with `text`.  A range outside the source, or one that
    /// would rewrite bytes an earlier edit already claimed, is dropped: two
    /// rules disagreeing must not produce a scrambled line.
    pub fn replace(&mut self, range: Range<usize>, text: &[u8]) {
        if range.start > range.end || range.end > self.source.len() {
            self.skipped += 1;
            return;
        }
        if self.source[range.clone()] == *text {
            return;
        }
        self.edits.push(Edit {
            range,
            text: text.to_vec(),
        });
    }

    /// Insert `text` before `at`.
    pub fn insert(&mut self, at: usize, text: &[u8]) {
        self.replace(at..at, text);
    }

    /// Remove `range`.
    pub fn delete(&mut self, range: Range<usize>) {
        self.replace(range, b"");
    }

    /// True when nothing would change.
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// The number of edits dropped as out of range or overlapping.  Nonzero
    /// here means a rule is confused; the corpus check reports it rather than
    /// letting it silently alter output.
    pub fn skipped(&self) -> usize {
        self.skipped
    }

    /// Apply the edits left to right.
    pub fn finish(mut self) -> Vec<u8> {
        if self.edits.is_empty() {
            return self.source.to_vec();
        }
        // A stable sort keeps insertions at one offset in the order the rules
        // requested them.
        self.edits.sort_by_key(|edit| edit.range.start);
        let mut out = Vec::with_capacity(self.source.len() + 16);
        let mut cursor = 0usize;
        for edit in &self.edits {
            if edit.range.start < cursor {
                self.skipped += 1;
                continue;
            }
            out.extend_from_slice(&self.source[cursor..edit.range.start]);
            out.extend_from_slice(&edit.text);
            cursor = edit.range.end;
        }
        out.extend_from_slice(&self.source[cursor..]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::EditBuffer;

    #[test]
    fn edits_apply_left_to_right_regardless_of_request_order() {
        let mut edits = EditBuffer::new(b"a+b*c");
        edits.replace(3..4, b" * ");
        edits.replace(1..2, b" + ");
        assert_eq!(edits.finish(), b"a + b * c");
    }

    #[test]
    fn an_unedited_line_is_returned_unchanged() {
        let edits = EditBuffer::new(b"x = 1");
        assert!(edits.is_empty());
        assert_eq!(edits.finish(), b"x = 1");
    }

    #[test]
    fn overlapping_and_out_of_range_edits_are_dropped_not_applied() {
        let mut edits = EditBuffer::new(b"abcdef");
        edits.replace(1..4, b"X");
        edits.replace(2..5, b"Y");
        edits.replace(9..10, b"Z");
        let skipped_before = edits.skipped();
        let result = edits.finish();
        assert_eq!(result, b"aXef");
        assert_eq!(skipped_before, 1);
    }

    #[test]
    fn insertions_at_one_offset_keep_their_requested_order() {
        let mut edits = EditBuffer::new(b"ac");
        edits.insert(1, b"b");
        edits.insert(1, b"B");
        assert_eq!(edits.finish(), b"abBc");
    }

    #[test]
    fn a_replacement_equal_to_the_source_is_not_an_edit() {
        let mut edits = EditBuffer::new(b"x = 1");
        edits.replace(1..4, b" = ");
        assert!(edits.is_empty());
    }
}
