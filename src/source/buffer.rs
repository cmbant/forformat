use super::{
    regions::LexState,
    syntax::{conditional_compilation_prefix, ConditionalPrefixKind, SourceStream},
    Newline, PhysicalLine, PhysicalLineKind,
};
use crate::error::FormatError;
use std::ops::Range;

/// The inline comment marker offset of one standalone slice, scanned from a
/// clean lexical state.  Callers that own a whole line sequence go through
/// [`SourceBuffer`], which carries the state across continuations instead.
pub use super::regions::comment_start;

#[derive(Debug, Clone)]
pub struct SourceBuffer<B = Vec<u8>> {
    pub bytes: B,
    pub lines: Vec<PhysicalLine>,
}

#[derive(Default)]
struct ConditionalStreamState {
    lex: LexState,
    continued: bool,
}

#[derive(Default)]
struct LexStreams {
    ordinary: LexState,
    conditional: ConditionalStreamState,
}

impl SourceBuffer<Vec<u8>> {
    /// Build a zero-copy source view over borrowed input.
    pub fn new<'a>(bytes: &'a [u8]) -> Result<SourceBuffer<&'a [u8]>, FormatError> {
        SourceBuffer::<&'a [u8]>::from_storage(bytes)
    }

    /// Build an owning source buffer, reusing the caller's allocation.
    pub fn from_vec(bytes: Vec<u8>) -> Result<Self, FormatError> {
        Self::from_storage(bytes)
    }
}

impl<B: AsRef<[u8]>> SourceBuffer<B> {
    fn from_storage(bytes: B) -> Result<Self, FormatError> {
        let source = bytes.as_ref();
        if source.len() > u32::MAX as usize {
            return Err(FormatError::InputTooLarge);
        }
        let mut lines = Vec::new();
        let mut start = 0usize;
        // Ordinary and conditional-compilation code carry independent lexical
        // state. A literal continued in one stream steps over a physical line
        // from the other stream, so that line must not close or reset it.
        let mut states = LexStreams::default();
        for (i, b) in source.iter().enumerate() {
            if *b == b'\n' {
                let is_crlf = i > start && source[i - 1] == b'\r';
                let end = if is_crlf { i - 1 } else { i };
                lines.push(Self::line(
                    &source[start..end],
                    start..end,
                    if is_crlf { Newline::CrLf } else { Newline::Lf },
                    &mut states,
                ));
                start = i + 1;
            }
        }
        if start < source.len() || source.is_empty() {
            lines.push(Self::line(
                &source[start..],
                start..source.len(),
                Newline::None,
                &mut states,
            ));
        }
        Ok(Self { bytes, lines })
    }

    fn line(
        content: &[u8],
        span: Range<usize>,
        newline: Newline,
        states: &mut LexStreams,
    ) -> PhysicalLine {
        let mut first = 0;
        while first < content.len() && (content[first] == b' ' || content[first] == b'\t') {
            first += 1;
        }
        let parsed_prefix = conditional_compilation_prefix(content);
        let stream = match parsed_prefix {
            Some(prefix) if prefix.kind == ConditionalPrefixKind::BlankSeparated => {
                SourceStream::Conditional
            }
            Some(prefix)
                if prefix.kind == ConditionalPrefixKind::CompactContinuation
                    && states.conditional.continued =>
            {
                SourceStream::Conditional
            }
            _ => SourceStream::Ordinary,
        };
        let conditional_start = stream.is_conditional().then(|| {
            parsed_prefix
                .expect("conditional stream has a parsed prefix")
                .body_start
        });
        let trimmed = &content[first..];
        let kind = if trimmed.is_empty() {
            PhysicalLineKind::Blank
        } else if stream.is_conditional() {
            PhysicalLineKind::Code
        } else if trimmed.starts_with(b"#") || trimmed.starts_with(b"??") {
            PhysicalLineKind::Preprocessor
        } else if trimmed.starts_with(b"!") {
            if is_fix(trimmed) {
                PhysicalLineKind::FindentFix
            } else {
                PhysicalLineKind::Comment
            }
        } else {
            PhysicalLineKind::Code
        };
        let code_offset = conditional_start.unwrap_or(first);
        let mut comment = None;
        if matches!(kind, PhysicalLineKind::Code | PhysicalLineKind::FindentFix) {
            let code = &content[code_offset..];
            match stream {
                SourceStream::Ordinary => {
                    if let Some(i) = super::regions::line_comment_start(&mut states.ordinary, code)
                    {
                        comment = Some((span.start + code_offset + i) as u32..span.end as u32);
                    }
                }
                SourceStream::Conditional => {
                    let stream = &mut states.conditional;
                    // A free-form character literal can only resume on a physical line
                    // whose first nonblank body byte is `&`. If malformed or inactive
                    // source puts another code-looking line in between, step over it
                    // without consuming either lexical or continuation state.
                    let can_scan =
                        !stream.lex.in_literal() || code.trim_ascii_start().starts_with(b"&");
                    if can_scan {
                        let scan = stream.lex.scan_line(code, |_| {});
                        if let Some(i) = scan.comment_start {
                            comment = Some((span.start + code_offset + i) as u32..span.end as u32);
                        }
                        stream.continued = scan.continued;
                        if !stream.continued {
                            stream.lex = LexState::default();
                        }
                    }
                }
            }
        }
        // Non-code lines leave both stream states alone. A continued statement
        // steps over blank, comment and directive lines, so one appearing
        // inside an open protected context neither closes it nor lexes as part
        // of it; the literal resumes on the next code line of the same stream.
        let code_end = comment.as_ref().map_or(span.end, |r| r.start as usize);
        let code_start = span.start + code_offset;
        PhysicalLine {
            span: span.start as u32..span.end as u32,
            newline,
            kind,
            code_span: code_start as u32..code_end as u32,
            comment_span: comment,
            omp: stream.is_conditional(),
        }
    }

    pub fn line_bytes(&self, line: &PhysicalLine) -> &[u8] {
        let bytes = self.bytes.as_ref();
        &bytes[line.span.start as usize..line.span.end as usize]
    }

    pub fn code_bytes(&self, line: &PhysicalLine) -> &[u8] {
        let bytes = self.bytes.as_ref();
        &bytes[line.code_span.start as usize..line.code_span.end as usize]
    }

    pub fn newline(&self, index: usize) -> Newline {
        let line = &self.lines[index];
        if line.newline != Newline::None {
            return line.newline;
        }
        index
            .checked_sub(1)
            .and_then(|previous| self.lines.get(previous))
            .map_or(Newline::Lf, |previous| previous.newline)
    }
}

pub fn is_fix(s: &[u8]) -> bool {
    let mut i = 0;
    while i < s.len() && (s[i] == b' ' || s[i] == b'\t') {
        i += 1;
    }
    s[i..].starts_with(b"!") && {
        let t = &s[i + 1..];
        let mut j = 0;
        while j < t.len() && (t[j] == b' ' || t[j] == b'\t') {
            j += 1;
        }
        t[j..].len() >= b"findentfix:".len()
            && t[j..j + b"findentfix:".len()].eq_ignore_ascii_case(b"findentfix:")
    }
}

#[cfg(test)]
mod tests {
    use super::{comment_start, Newline, PhysicalLineKind, SourceBuffer};

    #[test]
    fn borrowed_source_buffer_reuses_input_bytes() {
        let source = b"program p\nend program p\n".to_vec();
        let buffer = SourceBuffer::new(&source).unwrap();
        assert_eq!(buffer.bytes.as_ptr(), source.as_ptr());
        assert_eq!(buffer.bytes.len(), source.len());
    }

    #[test]
    fn comment_scanning_ignores_quotes_and_hollerith_constants() {
        assert_eq!(comment_start(b"x='!'; y=\"!\" ! real"), Some(13));
        assert_eq!(comment_start(b"x=3H;!; ! real"), Some(8));
        assert_eq!(comment_start(b"x='a''!b' ! real"), Some(10));
    }

    #[test]
    fn conditional_compilation_accepts_initial_and_contextual_compact_sentinels() {
        let buffer =
            SourceBuffer::new(b"!$ x = 1\n!$\ty = 2 ! note\n!$ call f( &\n!$& arg = 1)\n").unwrap();
        for line in &buffer.lines {
            assert!(line.is_conditional_compilation());
        }
        assert_eq!(buffer.code_bytes(&buffer.lines[0]), b"x = 1");
        assert_eq!(buffer.code_bytes(&buffer.lines[1]), b"y = 2 ");
        assert!(buffer.lines[1].comment_span.is_some());
        assert_eq!(buffer.code_bytes(&buffer.lines[2]), b"call f( &");
        assert_eq!(buffer.code_bytes(&buffer.lines[3]), b"& arg = 1)");

        let standalone = SourceBuffer::new(
            b"!$& standalone
",
        )
        .unwrap();
        assert_eq!(standalone.lines[0].kind, PhysicalLineKind::Comment);
        assert!(!standalone.lines[0].is_conditional_compilation());
    }

    #[test]
    fn hollerith_payload_ampersand_does_not_open_conditional_stream() {
        let buffer = SourceBuffer::new(
            b"!$ x = 1H&
!$& standalone
",
        )
        .unwrap();
        assert!(buffer.lines[0].is_conditional_compilation());
        assert_eq!(buffer.code_bytes(&buffer.lines[0]), b"x = 1H&");
        assert_eq!(buffer.lines[1].kind, PhysicalLineKind::Comment);
        assert!(!buffer.lines[1].is_conditional_compilation());
        assert_eq!(buffer.line_bytes(&buffer.lines[1]), b"!$& standalone");
    }

    #[test]
    fn lexical_state_is_independent_between_sentinel_streams() {
        let ordinary = SourceBuffer::new(b"x = 'ab &\n!$ y = 2\n&cd!ef'\n").unwrap();
        assert!(ordinary.lines[2].comment_span.is_none());
        assert_eq!(ordinary.code_bytes(&ordinary.lines[2]), b"&cd!ef'");

        let conditional = SourceBuffer::new(b"!$ x = 'ab &\ny = 2\n!$&cd!ef'\n").unwrap();
        assert!(conditional.lines[2].comment_span.is_none());
        assert_eq!(conditional.code_bytes(&conditional.lines[2]), b"&cd!ef'");
    }

    #[test]
    fn open_literal_steps_over_non_continuation_code_in_its_stream() {
        let buffer = SourceBuffer::new(b"!$ s = 'abc &\n!$ y = 2\n!$ &def!ghi'\n").unwrap();
        assert!(buffer.lines[2].comment_span.is_none());
        assert_eq!(buffer.code_bytes(&buffer.lines[2]), b"&def!ghi'");
    }

    #[test]
    fn source_spans_preserve_mixed_newlines_and_lone_carriage_returns() {
        let buffer = SourceBuffer::new(b" a\r\nb\nc\r\n").unwrap();
        assert_eq!(buffer.lines.len(), 3);
        assert_eq!(buffer.lines[0].newline, Newline::CrLf);
        assert_eq!(buffer.lines[1].newline, Newline::Lf);
        assert_eq!(buffer.lines[2].newline, Newline::CrLf);
        assert_eq!(buffer.line_bytes(&buffer.lines[0]), b" a");

        let lone = SourceBuffer::new(b"a\rb").unwrap();
        assert_eq!(lone.lines.len(), 1);
        assert_eq!(buffer.line_bytes(&buffer.lines[0]), b" a");
        assert_eq!(lone.line_bytes(&lone.lines[0]), b"a\rb");
    }

    #[test]
    fn malformed_editor_lines_keep_ordered_in_bounds_spans() {
        let source = b"#define X \\\r\n\n! comment \xff\r\n &\r\n\xff\n";
        let buffer = SourceBuffer::new(source).unwrap();
        assert_eq!(buffer.lines.len(), 5);
        for line in &buffer.lines {
            assert!(line.span.start <= line.span.end);
            assert!(line.code_span.start <= line.code_span.end);
            assert!(line.span.end as usize <= source.len());
            assert!(line.code_span.end <= line.span.end);
            if let Some(comment) = &line.comment_span {
                assert!(comment.start <= comment.end);
                assert!(comment.end <= line.span.end);
            }
        }
        assert_eq!(buffer.line_bytes(&buffer.lines[3]), b" &");
        assert_eq!(buffer.code_bytes(&buffer.lines[4]), b"\xff");
    }

    #[test]
    fn comment_scanning_handles_doubled_quotes_and_non_ascii_bytes() {
        assert_eq!(comment_start(b"x='a''!b'\xff ! real"), Some(11));
        assert_eq!(comment_start(b"x=\"a\"\xff ! real"), Some(7));
        assert_eq!(comment_start(b"x=3h\xff! real"), None);
    }
}
