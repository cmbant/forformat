use super::continuation::leading_ampersand;
use crate::{
    config::{FormatConfig, FormatMode},
    error::FormatError,
    source::{syntax::conditional_compilation_prefix, Newline, PhysicalLineKind, SourceBuffer},
};
use std::io::Write;

static SPACES: [u8; 128] = [b' '; 128];

pub fn newline_bytes(n: Newline) -> &'static [u8] {
    match n {
        Newline::Lf | Newline::None => b"\n",
        Newline::CrLf => b"\r\n",
    }
}

/// Per-group emission policy.
///
/// These two flags used to be expressed by cloning `FormatConfig` once per
/// logical group and mutating a field.  They are separated so the config can
/// grow keyword and symbol tables without the clone becoming a real cost.
#[derive(Debug, Clone, Copy)]
pub struct EmitStyle<'a> {
    pub config: &'a FormatConfig,
    /// False for CPP directive groups, whose source indentation is structural
    /// noise, and for `-i-`.
    pub apply_indent: bool,
    /// Redundant-whitespace reduction, disabled for Hollerith-bearing groups
    /// because their payload length is positional.
    pub remred: bool,
}

impl<'a> EmitStyle<'a> {
    /// The style for an ordinary group: indentation as configured, whitespace
    /// reduction as configured.
    pub fn new(config: &'a FormatConfig) -> Self {
        Self {
            config,
            apply_indent: config.apply_indent,
            remred: config.ws_remred || config.ws_remred_value != 0,
        }
    }
}

/// Where one physical line goes.  This is the part of the layout plan the
/// emitter consumes; the planner decides it, ahead of any byte being written.
#[derive(Debug, Clone, Copy, Default)]
pub struct LinePlacement {
    pub indent: usize,
    /// First physical line of its logical group.
    pub first: bool,
    /// The group's first line ended with `&`.
    pub previous_cont: bool,
    /// Active parenthesis alignment column, when `--align-paren` is on.
    pub alignment: Option<usize>,
}

/// Compatibility wrapper used by small callers and tests.  The formatter's
/// production path uses `emit_line_to`, which writes directly to its sink.
pub fn emit_line<B: AsRef<[u8]>>(
    buf: &SourceBuffer<B>,
    index: usize,
    place: LinePlacement,
    style: &EmitStyle,
    replacement: Option<&[u8]>,
) -> Vec<u8> {
    let mut out = Vec::new();
    emit_line_to(buf, index, place, style, replacement, &mut out)
        .expect("writing to a Vec cannot fail");
    out
}

/// Emit one physical line without allocating an intermediate line buffer.
/// The default policy replaces leading horizontal whitespace and trims
/// trailing horizontal whitespace.  Other bytes in the line body remain
/// source-owned unless an explicit transformation is enabled.
pub fn emit_line_to<B: AsRef<[u8]>, W: Write>(
    buf: &SourceBuffer<B>,
    index: usize,
    place: LinePlacement,
    style: &EmitStyle,
    replacement: Option<&[u8]>,
    out: &mut W,
) -> Result<(), FormatError> {
    let mut quote = 0u8;
    emit_line_to_with_quote(buf, index, place, style, replacement, &mut quote, out)
}

/// Emit one physical line while carrying the redundant-whitespace transform's
/// quote state across a logical continuation group.
pub fn emit_line_to_with_quote<B: AsRef<[u8]>, W: Write>(
    buf: &SourceBuffer<B>,
    index: usize,
    place: LinePlacement,
    style: &EmitStyle,
    replacement: Option<&[u8]>,
    quote: &mut u8,
    out: &mut W,
) -> Result<(), FormatError> {
    let LinePlacement {
        indent,
        first,
        previous_cont,
        alignment,
    } = place;
    let config = style.config;
    let line = &buf.lines[index];
    let original = buf.line_bytes(line);

    // Preprocessor spelling is preserved, but its source indentation is always
    // structural noise and trailing horizontal whitespace is normalized.  This
    // must run before the apply-indent fast path because the engine deliberately disables normal
    // code indentation while emitting directive groups.
    if line.kind == PhysicalLineKind::Preprocessor {
        out.write_all(trim_end_horizontal(trim_start(original)))
            .map_err(FormatError::Write)?;
        write_newline(buf, index, out)?;
        return Ok(());
    }

    if !style.apply_indent {
        let leading = leading_len(original);
        if let Some(replacement) = replacement {
            out.write_all(&original[..leading])
                .map_err(FormatError::Write)?;
            out.write_all(trim_end_horizontal(replacement))
                .map_err(FormatError::Write)?;
        } else {
            out.write_all(trim_end_horizontal(original))
                .map_err(FormatError::Write)?;
        }
        write_newline(buf, index, out)?;
        return Ok(());
    }

    if line.kind == PhysicalLineKind::Blank {
        write_newline(buf, index, out)?;
        return Ok(());
    }

    // The directive is a source comment which also feeds a synthetic
    // statement to the classifier.  Its spelling is retained, apart from
    // trailing horizontal whitespace normalization.
    if line.kind == PhysicalLineKind::FindentFix {
        out.write_all(trim_end_horizontal(original))
            .map_err(FormatError::Write)?;
        write_newline(buf, index, out)?;
        return Ok(());
    }

    if line.kind == PhysicalLineKind::Comment {
        let comment = trim_end_horizontal(original);
        let near_omp = near_openmp_comment(trim_start(comment));
        if has_leading_horizontal_space(comment) {
            // findent keeps a single separating blank before an indented
            // comment even when the surrounding construct is at column zero.
            // This also prevents a previously over-indented comment from
            // retaining stale source indentation after `--indent_contains=restart`.
            write_spaces(out, clamp_indent(indent, config.max_indent).max(1))?;
            if let Some(rest) = near_omp {
                write_near_openmp_comment(rest, out)?;
            } else {
                out.write_all(trim_start(comment))
                    .map_err(FormatError::Write)?;
            }
        } else if let Some(rest) = near_omp {
            write_near_openmp_comment(rest, out)?;
        } else {
            out.write_all(comment).map_err(FormatError::Write)?;
        }
        write_newline(buf, index, out)?;
        return Ok(());
    }

    // With OpenMP disabled, an exact sentinel is an ordinary comment.  Keep
    // unindented comments at column zero, while bringing a source-indented
    // comment to the current structural level just like other comments.
    if line.omp && !config.openmp {
        if has_leading_horizontal_space(original) {
            write_spaces(out, clamp_indent(indent, config.max_indent).max(1))?;
            out.write_all(trim_end_horizontal(trim_start(original)))
                .map_err(FormatError::Write)?;
        } else {
            out.write_all(trim_end_horizontal(original))
                .map_err(FormatError::Write)?;
        }
        write_newline(buf, index, out)?;
        return Ok(());
    }

    let mut source = trim_start(original);
    let conditional = line.omp && config.openmp;
    if conditional {
        // The input prefix is not always three bytes: a valid continuation can
        // use the compact `!$&` spelling. Parse the source-owned prefix and
        // start at its Fortran body, then emit one canonical three-column
        // `!$ ` prefix below.
        let prefix = conditional_compilation_prefix(original)
            .expect("conditional SourceBuffer line has a parsed sentinel prefix");
        source = trim_start(&original[prefix.body_start..]);
    }

    let mut target = indent;
    if conditional {
        target = target.saturating_sub(3);
    }
    if !first {
        if leading_ampersand(source) {
            if config.indent_ampersand && previous_cont {
                target = target.saturating_add(config.continuation_indent);
            }
        } else if let Some(alignment) = alignment {
            target = alignment;
        } else if config.indent_continuation && previous_cont {
            target = target.saturating_add(config.continuation_indent);
        } else if !config.indent_continuation && previous_cont {
            target = if conditional {
                0
            } else {
                leading_len(original)
            };
        }
    }
    if conditional && config.max_indent != 0 {
        target = target.min(config.max_indent.saturating_sub(3));
    }
    if (!first || previous_cont) && is_label_fragment(source) {
        target = 0;
    }

    if let Some(replacement) = replacement {
        source = replacement;
    }

    if conditional {
        out.write_all(b"!$").map_err(FormatError::Write)?;
        if trim_end_horizontal(source).is_empty() {
            return write_newline(buf, index, out);
        }
        out.write_all(b" ").map_err(FormatError::Write)?;
    }

    if first && !(previous_cont && is_label_fragment(source)) {
        if let Some((label, rest)) = split_label(source) {
            if config.label_left {
                out.write_all(label).map_err(FormatError::Write)?;
                let padding =
                    clamp_indent(target.saturating_sub(label.len()), config.max_indent).max(1);
                write_spaces(out, padding)?;
            } else {
                write_spaces(out, clamp_indent(target, config.max_indent))?;
                out.write_all(label).map_err(FormatError::Write)?;
                out.write_all(b" ").map_err(FormatError::Write)?;
            }
            write_body(rest, style, quote, out)?;
        } else {
            write_spaces(out, clamp_indent(target, config.max_indent))?;
            write_body(source, style, quote, out)?;
        }
    } else {
        write_spaces(out, clamp_indent(target, config.max_indent))?;
        write_body(source, style, quote, out)?;
    }
    write_newline(buf, index, out)
}

fn write_body<W: Write>(
    body: &[u8],
    style: &EmitStyle,
    quote: &mut u8,
    out: &mut W,
) -> Result<(), FormatError> {
    let body = trim_end_horizontal(body);
    if style.remred {
        // The post-layout alignment passes that would own a protected gap
        // (`declaration_separator_alignment`, `trailing_comment_alignment`)
        // only run in full mode: indent-only reaches this same emitter
        // through `engine::format` without ever running them. Protecting the
        // gap there would leave it un-owned and un-collapsed, breaking the
        // byte-exact indent-only contract.
        let alignment_runs_after = style.config.mode == FormatMode::Full;
        crate::transform::whitespace::reduce_to_with_quote_protected(
            body,
            quote,
            alignment_runs_after && style.config.align_declarations,
            alignment_runs_after && style.config.align_comments,
            out,
        )
        .map_err(FormatError::Write)
    } else {
        out.write_all(body).map_err(FormatError::Write)
    }
}

fn trim_end_horizontal(mut s: &[u8]) -> &[u8] {
    while s.last().is_some_and(|byte| *byte == b' ' || *byte == b'\t') {
        s = &s[..s.len() - 1];
    }
    s
}

fn write_near_openmp_comment<W: Write>(rest: &[u8], out: &mut W) -> Result<(), FormatError> {
    out.write_all(b"!$").map_err(FormatError::Write)?;
    if !rest.is_empty() {
        out.write_all(b" ").map_err(FormatError::Write)?;
        out.write_all(rest).map_err(FormatError::Write)?;
    }
    Ok(())
}

fn write_newline<B: AsRef<[u8]>, W: Write>(
    buf: &SourceBuffer<B>,
    index: usize,
    out: &mut W,
) -> Result<(), FormatError> {
    out.write_all(newline_bytes(buf.newline(index)))
        .map_err(FormatError::Write)
}

fn write_spaces<W: Write>(out: &mut W, mut count: usize) -> Result<(), FormatError> {
    while count >= SPACES.len() {
        out.write_all(&SPACES).map_err(FormatError::Write)?;
        count -= SPACES.len();
    }
    out.write_all(&SPACES[..count]).map_err(FormatError::Write)
}

fn leading_len(s: &[u8]) -> usize {
    s.iter().take_while(|c| is_horizontal(**c)).count()
}

fn has_leading_horizontal_space(s: &[u8]) -> bool {
    leading_len(s) != 0
}

fn trim_start(s: &[u8]) -> &[u8] {
    &s[leading_len(s)..]
}

fn is_horizontal(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

fn clamp_indent(indent: usize, max: usize) -> usize {
    if max == 0 {
        indent
    } else {
        indent.min(max)
    }
}

/// The column at which this emitter will start a labelled statement's body,
/// given the statement's structural indent.
///
/// The arithmetic is the one in [`emit_line`]'s labelled branch and has to
/// stay that way: under `--label-left=1` the label occupies the left margin
/// and the padding after it is chosen so the *body* still lands on `indent`,
/// so a label costs the line nothing until it is wider than the indent it sits
/// in. Under `--label-left=0` the label sits inside the indent and does push
/// the body along. Callers that only want to know how wide a labelled line
/// will be must ask this rather than adding the label's own length to
/// `indent`, which pays for the same columns twice.
pub(crate) fn labelled_body_column(
    indent: usize,
    label_len: usize,
    config: &FormatConfig,
) -> usize {
    if config.label_left {
        label_len + clamp_indent(indent.saturating_sub(label_len), config.max_indent).max(1)
    } else {
        clamp_indent(indent, config.max_indent) + label_len + 1
    }
}

fn near_openmp_comment(s: &[u8]) -> Option<&[u8]> {
    if !s.starts_with(b"!$") {
        return None;
    }
    match s.get(2) {
        None => Some(&s[2..]),
        Some(byte) if is_horizontal(*byte) => Some(trim_start(&s[2..])),
        _ => None,
    }
}

/// Split a leading statement label off the first line of a statement,
/// returning the digits and the statement text that follows them.
///
/// The author's gap between the two is discarded, because this emitter does
/// not preserve it: it writes the label and then pads out to the column the
/// statement is owed. Anything that needs to measure a labelled statement
/// before it is emitted has to split it the same way, or it charges the
/// statement for bytes that are about to disappear — hence `pub(crate)`.
pub(crate) fn split_label(s: &[u8]) -> Option<(&[u8], &[u8])> {
    let mut i = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i < s.len() && (is_horizontal(s[i]) || s[i] == b'&') {
        let mut j = i;
        while j < s.len() && is_horizontal(s[j]) {
            j += 1;
        }
        Some((&s[..i], &s[j..]))
    } else {
        None
    }
}

fn is_label_fragment(s: &[u8]) -> bool {
    let Some(amp) = s.iter().position(|byte| *byte == b'&') else {
        return false;
    };
    amp > 0 && s[..amp].iter().all(u8::is_ascii_digit) && s[amp + 1..].is_empty()
}

#[cfg(test)]
mod tests {
    use super::{emit_line, labelled_body_column, EmitStyle, LinePlacement};
    use crate::{config::FormatConfig, source::SourceBuffer};

    fn first(indent: usize) -> LinePlacement {
        LinePlacement {
            indent,
            first: true,
            previous_cont: false,
            alignment: None,
        }
    }

    fn continued(indent: usize, alignment: Option<usize>) -> LinePlacement {
        LinePlacement {
            indent,
            first: false,
            previous_cont: true,
            alignment,
        }
    }

    #[test]
    fn direct_emitter_handles_labels_and_continuations() {
        let labeled = SourceBuffer::new(b"  10 x=1\n").unwrap();
        let config = FormatConfig::default();
        let style = EmitStyle::new(&config);
        assert_eq!(emit_line(&labeled, 0, first(3), &style, None), b"10 x=1\n");

        let source = SourceBuffer::new(b"x = &\n  & y\n").unwrap();
        assert_eq!(
            emit_line(&source, 1, continued(3, None), &style, None),
            b"   & y\n"
        );
    }

    #[test]
    fn the_labelled_body_column_is_the_one_the_emitter_writes() {
        // `labelled_body_column` exists so the wrapper can size a labelled line
        // without re-deriving this branch's arithmetic.  It is only worth
        // having while the two agree, so the check is against the bytes.
        let statement = b"call f(x)";
        for label in ["1", "21", "1005", "99999"] {
            for indent in [0usize, 1, 2, 3, 8, 32, 40] {
                for label_left in [true, false] {
                    for max_indent in [0usize, 32] {
                        let config = FormatConfig {
                            label_left,
                            max_indent,
                            ..FormatConfig::default()
                        };
                        let mut source = label.as_bytes().to_vec();
                        source.extend_from_slice(b" ");
                        source.extend_from_slice(statement);
                        source.push(b'\n');
                        let buffer = SourceBuffer::new(&source).unwrap();
                        let emitted =
                            emit_line(&buffer, 0, first(indent), &EmitStyle::new(&config), None);
                        let column = emitted.len() - 1 - statement.len();
                        assert_eq!(
                            labelled_body_column(indent, label.len(), &config),
                            column,
                            "label {label}, indent {indent}, label_left {label_left}, \
                             max_indent {max_indent}: {}",
                            String::from_utf8_lossy(&emitted)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn direct_emitter_preserves_comment_and_openmp_boundaries() {
        let comments = SourceBuffer::new(b"  ! comment\n!$    call x\n").unwrap();
        let config = FormatConfig::default();
        let style = EmitStyle::new(&config);
        assert_eq!(
            emit_line(&comments, 0, first(0), &style, None),
            b" ! comment\n"
        );
        assert_eq!(
            emit_line(&comments, 1, first(6), &style, None),
            b"!$    call x\n"
        );

        let compact = SourceBuffer::new(b"!$ call f( &\n!$& arg = 1)\n").unwrap();
        assert_eq!(
            emit_line(&compact, 1, first(0), &style, None),
            b"!$ & arg = 1)\n"
        );

        let empty_sentinels = SourceBuffer::new(b"!$\n!$ \n").unwrap();
        assert_eq!(
            emit_line(&empty_sentinels, 0, first(0), &style, None),
            b"!$\n"
        );
        assert_eq!(
            emit_line(&empty_sentinels, 1, first(0), &style, None),
            b"!$\n"
        );
    }

    #[test]
    fn direct_emitter_covers_label_alignment_replacement_and_whitespace_modes() {
        let labeled = SourceBuffer::new(b"10 x=1\n").unwrap();
        let label_right = FormatConfig {
            label_left: false,
            ..FormatConfig::default()
        };
        assert_eq!(
            emit_line(&labeled, 0, first(3), &EmitStyle::new(&label_right), None),
            b"   10 x=1\n"
        );
        let source = SourceBuffer::new(b"x = f(a, &\n  b)\n").unwrap();
        let config = FormatConfig::default();
        let style = EmitStyle::new(&config);
        assert_eq!(
            emit_line(&source, 1, continued(3, Some(9)), &style, None),
            b"         b)\n"
        );

        assert_eq!(
            emit_line(&labeled, 0, first(0), &style, Some(b"10 y=2")),
            b"10 y=2\n"
        );

        let whitespace = SourceBuffer::new(b"x = \"a  b\"  \n").unwrap();
        assert_eq!(
            emit_line(&whitespace, 0, first(0), &style, None),
            b"x = \"a  b\"\n"
        );
        let mut reduced = config.clone();
        reduced.ws_remred = true;
        assert_eq!(
            emit_line(&whitespace, 0, first(0), &EmitStyle::new(&reduced), None),
            b"x = \"a  b\"\n"
        );
    }

    #[test]
    fn direct_emitter_protects_declaration_and_comment_gaps_from_remred() {
        // `--ws-remred` still applies everywhere else on the line; only the
        // gap the corresponding alignment pass owns is left for it to decide.
        let declaration = SourceBuffer::new(b"real(dl), intent(in)  ::   x\n").unwrap();
        let mut reduced = FormatConfig {
            ws_remred: true,
            ..FormatConfig::default()
        };
        assert_eq!(
            emit_line(&declaration, 0, first(0), &EmitStyle::new(&reduced), None),
            b"real(dl), intent(in)  :: x\n"
        );

        let mut reduced_no_align = reduced.clone();
        reduced_no_align.align_declarations = false;
        assert_eq!(
            emit_line(
                &declaration,
                0,
                first(0),
                &EmitStyle::new(&reduced_no_align),
                None
            ),
            b"real(dl), intent(in) :: x\n"
        );

        let comment = SourceBuffer::new(b"x  =  1   ! note\n").unwrap();
        assert_eq!(
            emit_line(&comment, 0, first(0), &EmitStyle::new(&reduced), None),
            b"x = 1 ! note\n"
        );
        reduced.align_comments = true;
        assert_eq!(
            emit_line(&comment, 0, first(0), &EmitStyle::new(&reduced), None),
            b"x = 1   ! note\n"
        );
    }

    #[test]
    fn indent_only_remred_still_collapses_the_declaration_gap() {
        // Indent-only reaches this emitter directly (`format::engine`) and
        // never runs the post-layout alignment passes that would otherwise
        // own the `::` gap. Gap protection must not apply there, or
        // `--ws-remred` in indent-only would stop matching the byte-exact
        // findent contract it is required to reproduce.
        use crate::config::FormatMode;
        let declaration = SourceBuffer::new(b"real(dl), intent(in)  :: x\n").unwrap();
        let reduced = FormatConfig {
            mode: FormatMode::IndentOnly,
            ws_remred: true,
            ..FormatConfig::default()
        };
        assert!(
            reduced.align_declarations,
            "protection is gated on mode, not this default"
        );
        assert_eq!(
            emit_line(&declaration, 0, first(0), &EmitStyle::new(&reduced), None),
            b"real(dl), intent(in) :: x\n"
        );
    }

    #[test]
    fn direct_emitter_preserves_mixed_terminators_and_ampersand_policy() {
        let source = SourceBuffer::new(b"x = a &\r\n  & b\ny = c\n").unwrap();
        let config = FormatConfig::default();
        let style = EmitStyle::new(&config);
        assert_eq!(
            emit_line(&source, 0, first(0), &style, None),
            b"x = a &\r\n"
        );
        assert_eq!(
            emit_line(&source, 1, continued(3, None), &style, None),
            b"   & b\n"
        );

        let mut indented_ampersand = config.clone();
        indented_ampersand.indent_ampersand = true;
        assert_eq!(
            emit_line(
                &source,
                1,
                continued(3, None),
                &EmitStyle::new(&indented_ampersand),
                None
            ),
            b"      & b\n"
        );
    }
}
