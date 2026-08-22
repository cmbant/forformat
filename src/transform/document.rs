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

use std::io::{BufWriter, Write};

use crate::{
    classify::{classify, StatementInfo},
    error::FormatError,
    source::{LogicalGroup, Newline, SourceBuffer},
};

const WRITE_BUFFER_LIMIT: usize = 64 * 1024;

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

    fn exact_line_endings(&self) -> Option<&[Newline]> {
        (self.preserve_line_endings && self.line_endings.len() == self.lines.len())
            .then_some(self.line_endings.as_slice())
    }

    fn terminator(&self) -> &'static [u8] {
        match self.newline {
            Newline::CrLf => b"\r\n",
            _ => b"\n",
        }
    }

    fn serialized_len(&self) -> usize {
        let line_bytes = self.lines.iter().map(Vec::len).sum::<usize>();
        if let Some(line_endings) = self.exact_line_endings() {
            return line_bytes
                + line_endings
                    .iter()
                    .map(|newline| match newline {
                        Newline::Lf => 1,
                        Newline::CrLf => 2,
                        Newline::None => 0,
                    })
                    .sum::<usize>();
        }
        let terminators = self.lines.len().saturating_sub(1) + usize::from(self.trailing_newline);
        line_bytes + terminators * self.terminator().len()
    }

    /// Write the document with its terminator policy applied.
    ///
    /// Small outputs fit in one staging buffer. Larger ones use the same
    /// buffer with a fixed cap so an unbuffered writer does not receive a
    /// separate write for every line and terminator.
    pub fn write_to<W: Write>(&self, out: &mut W) -> Result<(), FormatError> {
        let capacity = self.serialized_len().min(WRITE_BUFFER_LIMIT);
        let mut buffered = BufWriter::with_capacity(capacity, out);
        if let Some(line_endings) = self.exact_line_endings() {
            for (line, newline) in self.lines.iter().zip(line_endings) {
                buffered.write_all(line).map_err(FormatError::Write)?;
                match newline {
                    Newline::Lf => buffered.write_all(b"\n").map_err(FormatError::Write)?,
                    Newline::CrLf => buffered.write_all(b"\r\n").map_err(FormatError::Write)?,
                    Newline::None => {}
                }
            }
        } else {
            let terminator = self.terminator();
            for (i, line) in self.lines.iter().enumerate() {
                buffered.write_all(line).map_err(FormatError::Write)?;
                if i + 1 < self.lines.len() || self.trailing_newline {
                    buffered.write_all(terminator).map_err(FormatError::Write)?;
                }
            }
        }
        buffered
            .into_inner()
            .map_err(|error| FormatError::Write(error.into_error()))?;
        Ok(())
    }

    /// Render the document with its terminator policy applied.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.serialized_len());
        if let Some(line_endings) = self.exact_line_endings() {
            for (line, newline) in self.lines.iter().zip(line_endings) {
                out.extend_from_slice(line);
                match newline {
                    Newline::Lf => out.push(b'\n'),
                    Newline::CrLf => out.extend_from_slice(b"\r\n"),
                    Newline::None => {}
                }
            }
            return out;
        }

        let terminator = self.terminator();
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

    /// Canonicalize an empty final line created by editing an unterminated tail.
    ///
    /// After an edit turns the final unterminated physical line into empty text,
    /// a preceding line terminator is also the file terminator: serializing two
    /// empty-adjacent lines would parse back as one physical line. Collapse that
    /// ambiguous in-memory shape so `Document::lines` and a fresh `SourceBuffer`
    /// continue to describe the same physical lines.
    pub(crate) fn canonicalize_empty_unterminated_tail(&mut self) -> bool {
        if self.trailing_newline
            || self.lines.len() <= 1
            || !self.lines.last().is_some_and(Vec::is_empty)
        {
            return false;
        }
        self.lines.pop();
        self.line_endings.pop();
        self.trailing_newline = true;
        true
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
    use super::{Document, WRITE_BUFFER_LIMIT};
    use crate::source::Newline;
    use std::io::{self, Write};

    #[derive(Default)]
    struct CountingWriter {
        bytes: Vec<u8>,
        writes: usize,
        flushes: usize,
    }

    impl Write for CountingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

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
    fn small_streamed_documents_use_one_underlying_write() {
        let source = b"x = 1\n".repeat(128);
        let document = Document::from_bytes(&source);
        let mut writer = CountingWriter::default();
        document.write_to(&mut writer).unwrap();
        assert_eq!(writer.bytes, source);
        assert_eq!(writer.writes, 1);
        assert_eq!(writer.flushes, 0);
    }

    #[test]
    fn large_streamed_documents_batch_underlying_writes() {
        let mut source = Vec::new();
        for _ in 0..4096 {
            source.extend_from_slice(&[b'x'; 63]);
            source.push(b'\n');
        }
        assert!(source.len() > WRITE_BUFFER_LIMIT);

        let document = Document::from_bytes(&source);
        let mut writer = CountingWriter::default();
        document.write_to(&mut writer).unwrap();
        assert_eq!(writer.bytes, source);
        assert!(writer.writes <= 8);
        assert_eq!(writer.flushes, 0);
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
        let mut written = Vec::new();
        document.write_to(&mut written).unwrap();
        assert_eq!(written, b"a\r\nB\nc\r\nlast");
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
