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
    /// The dominant input line ending, restored on every output line.
    pub newline: Newline,
    /// Whether the input ended with a line terminator.
    pub trailing_newline: bool,
}

impl Document {
    /// Split `source` into lines, recording the dominant terminator.
    ///
    /// Mixed terminators are normalized to the dominant one in full mode; this
    /// is the Python contract.  Indent-only mode never builds a `Document`
    /// and keeps per-line terminators untouched.
    pub fn from_bytes(source: &[u8]) -> Self {
        let mut lines = Vec::new();
        let mut crlf = 0usize;
        let mut lf = 0usize;
        let mut start = 0usize;
        for (i, byte) in source.iter().enumerate() {
            if *byte != b'\n' {
                continue;
            }
            let is_crlf = i > start && source[i - 1] == b'\r';
            if is_crlf {
                crlf += 1;
            } else {
                lf += 1;
            }
            let end = if is_crlf { i - 1 } else { i };
            lines.push(source[start..end].to_vec());
            start = i + 1;
        }
        let trailing_newline = start == source.len() && !source.is_empty();
        if !trailing_newline {
            lines.push(source[start..].to_vec());
        }
        Self {
            lines,
            newline: if crlf > lf {
                Newline::CrLf
            } else {
                Newline::Lf
            },
            trailing_newline,
        }
    }

    /// Write the document with its terminator policy applied.
    pub fn write_to<W: std::io::Write>(&self, out: &mut W) -> Result<(), FormatError> {
        let terminator: &[u8] = match self.newline {
            Newline::CrLf => b"\r\n",
            _ => b"\n",
        };
        for (i, line) in self.lines.iter().enumerate() {
            out.write_all(line).map_err(FormatError::Write)?;
            if i + 1 < self.lines.len() || self.trailing_newline {
                out.write_all(terminator).map_err(FormatError::Write)?;
            }
        }
        Ok(())
    }

    /// Render the document with its terminator policy applied.
    pub fn to_bytes(&self) -> Vec<u8> {
        let terminator: &[u8] = match self.newline {
            Newline::CrLf => b"\r\n",
            _ => b"\n",
        };
        let mut out = Vec::with_capacity(self.lines.iter().map(|l| l.len() + 2).sum());
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
        let mut out = Vec::with_capacity(self.lines.iter().map(|l| l.len() + 1).sum());
        for (i, line) in self.lines.iter().enumerate() {
            out.extend_from_slice(line);
            if i + 1 < self.lines.len() || self.trailing_newline {
                out.push(b'\n');
            }
        }
        out
    }

    /// Replace the line list, keeping the terminator policy.
    pub fn set_lines(&mut self, lines: Vec<Vec<u8>>) {
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
            let mut written = Vec::new();
            document.write_to(&mut written).unwrap();
            assert_eq!(written, source, "streamed round trip of {source:?}");
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
