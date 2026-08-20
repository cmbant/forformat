//! The mutable text a full-mode pass operates on.
//!
//! Full mode works on a list of lines and re-derives statement structure
//! whenever a pass changes the line count. Keeping that shape makes each pass
//! independently testable while the Rust analyzer remains deliberately shallow.
//!
//! What is *not* inherited is indentation: passes never decide a column.  The
//! findent layout engine is run over the normalized text at the end, so
//! `indent_only(full(x)) == full(x)` (I2) holds by construction rather than by
//! testing.

use crate::{
    classify::{classify, StatementInfo},
    error::FormatError,
    source::{LogicalGroup, Newline, SourceBuffer},
};

/// A document as a list of lines without terminators, plus the terminator
/// policy to restore on output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub lines: Vec<Vec<u8>>,
    /// The dominant input line ending, restored on every normal full/normalize
    /// output line.
    pub newline: Newline,
    /// Whether the input ended with a line terminator.
    pub trailing_newline: bool,
    /// Exact authored terminators, parallel to `lines`. Kept separately from
    /// the dominant policy so canonicalization-only can promise that a token
    /// rewrite does not silently normalize mixed line endings.
    line_endings: Vec<Newline>,
    preserve_line_endings: bool,
}

impl Document {
    /// Split `source` into lines, recording the dominant terminator and the
    /// exact terminator of every physical line.
    ///
    /// Mixed terminators are normalized to the dominant one in ordinary full
    /// and normalize-only modes; canonicalization-only opts into exact
    /// restoration through the internal `preserve_original_line_endings` policy.
    pub fn from_bytes(source: &[u8]) -> Self {
        let mut lines = Vec::new();
        let mut line_endings = Vec::new();
        let mut crlf = 0usize;
        let mut lf = 0usize;
        let mut start = 0usize;
        for (i, byte) in source.iter().enumerate() {
            if *byte != b'\n' {
                continue;
            }
            let is_crlf = i > start && source[i - 1] == b'\r';
            let newline = if is_crlf {
                crlf += 1;
                Newline::CrLf
            } else {
                lf += 1;
                Newline::Lf
            };
            let end = if is_crlf { i - 1 } else { i };
            lines.push(source[start..end].to_vec());
            line_endings.push(newline);
            start = i + 1;
        }
        let trailing_newline = start == source.len() && !source.is_empty();
        if !trailing_newline {
            lines.push(source[start..].to_vec());
            line_endings.push(Newline::None);
        }
        Self {
            lines,
            newline: if crlf > lf {
                Newline::CrLf
            } else {
                Newline::Lf
            },
            trailing_newline,
            line_endings,
            preserve_line_endings: false,
        }
    }

    /// Keep each input physical line's exact terminator when rendering.
    ///
    /// This is only meaningful while the line count is unchanged. [`set_lines`]
    /// automatically drops the request if a structural pass changes that count.
    pub(crate) fn preserve_original_line_endings(&mut self) {
        self.preserve_line_endings = true;
    }

    /// Render the document with its terminator policy applied.
    pub fn to_bytes(&self) -> Vec<u8> {
        if self.preserve_line_endings && self.line_endings.len() == self.lines.len() {
            let mut out = Vec::with_capacity(self.lines.iter().map(|line| line.len() + 2).sum());
            for (line, newline) in self.lines.iter().zip(&self.line_endings) {
                out.extend_from_slice(line);
                match newline {
                    Newline::Lf => out.push(b'\n'),
                    Newline::CrLf => out.extend_from_slice(b"\r\n"),
                    Newline::None => {}
                }
            }
            return out;
        }

        let terminator: &[u8] = match self.newline {
            Newline::CrLf => b"\r\n",
            _ => b"\n",
        };
        let mut out = Vec::with_capacity(self.lines.iter().map(|line| line.len() + 2).sum());
        for (i, line) in self.lines.iter().enumerate() {
            out.extend_from_slice(line);
            if i + 1 < self.lines.len() || self.trailing_newline {
                out.extend_from_slice(terminator);
            }
        }
        out
    }

    /// Render with LF terminators, which is what the layout engine and the
    /// statement analyzer consume while passes are running.
    pub fn to_lf_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.lines.iter().map(|line| line.len() + 1).sum());
        for (i, line) in self.lines.iter().enumerate() {
            out.extend_from_slice(line);
            if i + 1 < self.lines.len() || self.trailing_newline {
                out.push(b'\n');
            }
        }
        out
    }

    /// Replace the line list, keeping the dominant terminator policy.
    pub fn set_lines(&mut self, lines: Vec<Vec<u8>>) {
        if lines.len() != self.lines.len() {
            self.preserve_line_endings = false;
        }
        self.lines = lines;
    }

    /// Rebuild the statement view of the current text.
    ///
    /// Every pass that changes the line count must call this before a later
    /// pass consults scope or statement information; §5.2 of the port plan
    /// records which Python passes force the same re-extraction.
    pub fn analyze(&self) -> Result<Analysis, FormatError> {
        Analysis::new(self.to_lf_bytes())
    }
}

/// The statement view of a document snapshot.
///
/// Owning the `SourceBuffer` keeps the borrow simple: passes mutate
/// `Document::lines` while reading a previously built `Analysis`, and the
/// pipeline decides when the two are allowed to diverge.
#[derive(Debug)]
pub struct Analysis {
    pub buffer: SourceBuffer,
    pub groups: Vec<LogicalGroup>,
    /// Classification of every statement of every group, parallel to
    /// `groups[i].statements`.
    pub infos: Vec<Vec<StatementInfo>>,
    /// For each physical line, the index of the group that owns it.
    pub line_group: Vec<usize>,
}

impl Analysis {
    pub fn new(bytes: Vec<u8>) -> Result<Self, FormatError> {
        let buffer = SourceBuffer::from_vec(bytes)?;
        let groups = LogicalGroup::assemble(&buffer);
        let mut infos = Vec::with_capacity(groups.len());
        let mut line_group = vec![0usize; buffer.lines.len()];
        for (index, group) in groups.iter().enumerate() {
            infos.push(
                group
                    .statements
                    .iter()
                    .map(|statement| classify(&statement.text))
                    .collect(),
            );
            for line in group.lines.clone() {
                if let Some(slot) = line_group.get_mut(line) {
                    *slot = index;
                }
            }
        }
        Ok(Self {
            buffer,
            groups,
            infos,
            line_group,
        })
    }

    /// The group owning a physical line, if the line is in range.
    pub fn group_of_line(&self, line: usize) -> Option<&LogicalGroup> {
        self.groups.get(*self.line_group.get(line)?)
    }

    /// The classification of the first statement of the group owning a line.
    pub fn info_of_line(&self, line: usize) -> Option<&StatementInfo> {
        self.infos.get(*self.line_group.get(line)?)?.first()
    }
}

#[cfg(test)]
mod tests {
    use super::Document;
    use crate::source::Newline;

    #[test]
    fn round_tripping_preserves_bytes_for_uniform_terminators() {
        for source in [
            b"a\nb\n".as_slice(),
            b"a\r\nb\r\n".as_slice(),
            b"a\nb".as_slice(),
            b"".as_slice(),
            b"\n".as_slice(),
            b"\xff\n".as_slice(),
        ] {
            let document = Document::from_bytes(source);
            assert_eq!(document.to_bytes(), source, "round trip of {source:?}");
        }
    }

    #[test]
    fn the_dominant_terminator_wins_for_mixed_input() {
        let mixed = Document::from_bytes(b"a\r\nb\r\nc\n");
        assert_eq!(mixed.newline, Newline::CrLf);
        assert_eq!(mixed.to_bytes(), b"a\r\nb\r\nc\r\n");
        assert_eq!(mixed.to_lf_bytes(), b"a\nb\nc\n");

        let mostly_lf = Document::from_bytes(b"a\nb\nc\r\n");
        assert_eq!(mostly_lf.newline, Newline::Lf);
        assert_eq!(mostly_lf.to_bytes(), b"a\nb\nc\n");
    }

    #[test]
    fn exact_mixed_terminators_can_be_preserved() {
        let source = b"a\r\nb\nc\r\nlast";
        let mut document = Document::from_bytes(source);
        document.preserve_original_line_endings();
        document.lines[1] = b"B".to_vec();
        assert_eq!(document.to_bytes(), b"a\r\nB\nc\r\nlast");
    }

    #[test]
    fn analysis_maps_every_physical_line_to_its_group() {
        let document = Document::from_bytes(b"program p\nx = a &\n  & + b\nend program\n");
        let analysis = document.analyze().unwrap();
        assert_eq!(analysis.groups.len(), 3);
        assert_eq!(analysis.line_group, [0, 1, 1, 2]);
        assert_eq!(analysis.group_of_line(2).unwrap().lines, 1..3);
        assert_eq!(
            analysis.info_of_line(0).unwrap().kind,
            crate::classify::StatementKind::Program
        );
    }
}
