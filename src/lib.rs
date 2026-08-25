//! A byte-oriented, free-form Fortran formatter.
//!
//! The formatter intentionally does not build a Fortran AST. It uses byte-oriented
//! lexical and statement analysis instead. The default [`FormatMode::Full`] pipeline
//! performs normalization, statement wrapping, findent-compatible structural layout,
//! and post-layout alignment; [`FormatMode::IndentOnly`] is the spelling-preserving
//! indentation path.
//!
//! Formatter-generated syntax targets Fortran 2003 by default. Set
//! [`FormatConfig::target_standard`] to [`FortranStandard::F95`] to prevent syntax
//! upgrades such as square-bracket array constructors.
//!
//! # Rust API status
//!
//! `forformat` is a command-line tool whose binary and integration tests use
//! this crate interface. The Rust surface is not covered by semantic-versioning
//! guarantees; applications that need a supported integration boundary should
//! use the command or the importable Python API.
//!
//! [`FormatConfig`] is a plain struct with public fields and a [`Default`]
//! impl, so it is configured with struct-update syntax rather than a builder:
//!
//! ```
//! use forformat::{FormatConfig, FormatMode};
//!
//! let config = FormatConfig {
//!     mode: FormatMode::IndentOnly,
//!     ..FormatConfig::default()
//! };
//! # let _ = config;
//! ```
//!
//! The modules below are `pub` because the binary, integration tests and
//! `cargo-fuzz` targets are separate crates and need to reach them. Depend on
//! them only if you are willing to track this crate commit by commit.

pub mod analysis;
pub mod classify;
pub mod cli;
pub mod config;
pub mod error;
pub mod format;
pub mod io;
pub mod source;
pub mod transform;

pub use analysis::{analyze_project, ProjectContext};
pub use config::{
    ConstructIndents, FormatConfig, FormatMode, FortranStandard, KeywordCase, MacroDefine,
    StyleConfig, WrapConfig,
};
pub use error::FormatError;

/// The result of formatting a source buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatResult {
    pub bytes: Vec<u8>,
    pub meta: FormatMeta,
}

/// Query information produced while formatting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormatMeta {
    pub last_indent: usize,
    pub last_usable: usize,
    /// Full-mode statements the wrapper deliberately left long, with the
    /// physical starting line and the reason for declining.
    pub declines: Vec<(usize, format::wrapping::Decline)>,
}

/// Format an in-memory source buffer with an empty, command-line-defined
/// project context.
///
/// A lone buffer has no file/project declarations, but `-D` definitions still
/// apply to it. This is also the context used by the isolated and stdin CLI
/// routes, so all single-buffer entry points share the same macro behavior.
pub fn format_source(source: &[u8], config: &FormatConfig) -> Result<FormatResult, FormatError> {
    if !config.mode.normalizes() {
        return format::engine::format(source, config);
    }
    let mut context = analysis::ProjectContext::empty();
    context.define(&config.defines);
    format_source_with_context(source, &context, config)
}

/// Format one source with the declarations of the whole project available.
///
/// A single `&[u8]` cannot express project context, so it is passed separately
/// rather than smuggled into `FormatConfig`. Build the context once per
/// invocation with [`analysis::analyze_project`] and reuse it for every target.
pub fn format_source_with_context(
    source: &[u8],
    context: &analysis::ProjectContext,
    config: &FormatConfig,
) -> Result<FormatResult, FormatError> {
    format::full::format_with_context(source, context, config)
}

/// Format into a caller-provided writer.
///
/// Full and normalize-only modes serialize their final `Document` through a
/// staging buffer capped at 64 KiB: small outputs are coalesced, while large
/// outputs avoid a second output-sized allocation.
pub fn format_to<W: std::io::Write>(
    source: &[u8],
    config: &FormatConfig,
    out: &mut W,
) -> Result<FormatMeta, FormatError> {
    if !config.mode.normalizes() {
        return format::engine::format_to(source, config, out);
    }
    let mut context = analysis::ProjectContext::empty();
    context.define(&config.defines);
    format::full::format_to_with_context(source, &context, config, out)
}

/// Format an owned source buffer into a caller-provided writer.
///
/// Indent-only mode reuses the caller's input allocation directly. Full and
/// normalize-only modes still split the input into the formatter's mutable
/// line-oriented `Document`, so they do not currently reuse that allocation;
/// final serialization uses the same bounded staging buffer as [`format_to`].
pub fn format_to_owned<W: std::io::Write>(
    source: Vec<u8>,
    config: &FormatConfig,
    out: &mut W,
) -> Result<FormatMeta, FormatError> {
    if !config.mode.normalizes() {
        return format::engine::format_to_owned(source, config, out);
    }
    let mut context = analysis::ProjectContext::empty();
    context.define(&config.defines);
    format::full::format_to_with_context(&source, &context, config, out)
}

#[cfg(test)]
mod tests {
    use super::{classify::classify, format_source, FormatConfig};
    use crate::{classify::StatementKind, FormatMode};

    fn indent_only_config() -> FormatConfig {
        FormatConfig {
            mode: FormatMode::IndentOnly,
            ..FormatConfig::default()
        }
    }

    #[test]
    fn formats_nested_constructs() {
        let input = b"program p\nif (a) then\nx=1\nelse\ny=2\nend if\nend program\n";
        let expected =
            b"program p\n   if (a) then\n      x=1\n   else\n      y=2\n   end if\nend program\n";
        assert_eq!(
            format_source(input, &indent_only_config()).unwrap().bytes,
            expected
        );
    }

    #[test]
    fn preserves_non_utf8_and_newlines() {
        let input = b"program p\r\n! caf\xe9\r\nx=1";
        let output = format_source(input, &indent_only_config()).unwrap().bytes;
        assert_eq!(output, b"program p\r\n! caf\xe9\r\n   x=1\r\n");
    }

    #[test]
    fn missing_final_terminator_matches_previous_line() {
        let lf = format_source(b"program p\nx=1", &indent_only_config())
            .unwrap()
            .bytes;
        assert!(lf.ends_with(b"\n"));
        assert!(!lf.ends_with(b"\r\n"));

        let crlf = format_source(b"program p\r\nx=1", &indent_only_config())
            .unwrap()
            .bytes;
        assert!(crlf.ends_with(b"\r\n"));

        let mixed = format_source(b"program p\r\nx=1\ny=2", &indent_only_config())
            .unwrap()
            .bytes;
        assert!(mixed.ends_with(b"\n"));

        let mixed_crlf = format_source(b"program p\nx=1\r\ny=2", &indent_only_config())
            .unwrap()
            .bytes;
        assert!(mixed_crlf.ends_with(b"\r\n"));
    }

    #[test]
    fn keyword_identifiers_do_not_open_constructs() {
        let input = b"program p\ninteger :: if, do, type\ntype(C) :: pointer_value\nif = 1\ndo = 2\nif (if > 0) then\ndo = do + 1\nend if\nend program\n";
        let expected = b"program p\n   integer :: if, do, type\n   type(C) :: pointer_value\n   if = 1\n   do = 2\n   if (if > 0) then\n      do = do + 1\n   end if\nend program\n";
        assert_eq!(
            format_source(input, &indent_only_config()).unwrap().bytes,
            expected
        );
    }

    #[test]
    fn malformed_digit_prefixes_do_not_mutate_label_or_construct_state() {
        let input =
            b"program p\n10abc continue\nif (x) then\n10def continue\nend if\nend program\n";
        let expected = b"program p\n   10abc continue\n   if (x) then\n      10def continue\n   end if\nend program\n";
        assert_eq!(
            format_source(input, &indent_only_config()).unwrap().bytes,
            expected
        );
    }

    #[test]
    fn semicolons_update_state_once_per_physical_line() {
        let input = b"program p\nif(a)then;x=1;y=2;end if\nend program\n";
        let expected = b"program p\n   if(a)then;x=1;y=2;end if\nend program\n";
        assert_eq!(
            format_source(input, &indent_only_config()).unwrap().bytes,
            expected
        );
    }

    #[test]
    fn malformed_bytes_are_total() {
        let input: Vec<u8> = (0..=255).chain(*b"\n)(\n").collect();
        assert!(format_source(&input, &indent_only_config()).is_ok());
    }

    #[test]
    fn named_do_with_comma_is_structural() {
        assert_eq!(classify(b"loop1 : do , i=1,2").kind, StatementKind::Do);
        assert_eq!(classify(b"endcritical").kind, StatementKind::EndCritical);
        assert_eq!(classify(b"elseif (x) then").kind, StatementKind::ElseIf);
        let input = b"program p\nloop1 : do , i=1,2\ncontinue\nenddo loop1\nend program\n";
        let expected =
            b"program p\n   loop1 : do , i=1,2\n      continue\n   enddo loop1\nend program\n";
        assert_eq!(
            format_source(input, &indent_only_config()).unwrap().bytes,
            expected
        );
    }

    #[test]
    fn compact_end_and_findentfix_directive_update_state() {
        let input = b"program p\nif (x) then\nx=1\nendif\ny=2\nendprogram\n";
        let expected = b"program p\n   if (x) then\n      x=1\n   endif\n   y=2\nendprogram\n";
        assert_eq!(
            format_source(input, &indent_only_config()).unwrap().bytes,
            expected
        );

        let input = b"program p\n!  findentfix: do\ny=2\nenddo\nend\n";
        let expected = b"program p\n!  findentfix: do\n      y=2\n   enddo\nend\n";
        assert_eq!(
            format_source(input, &indent_only_config()).unwrap().bytes,
            expected
        );
    }

    #[test]
    fn preprocessor_branches_restore_the_entry_state() {
        let input = b"program p\n#if defined(X)\nif (a) then\n#else\nif (b) then\n#endif\nx=1\nend if\nend program\n";
        let expected = b"program p\n#if defined(X)\n   if (a) then\n#else\n   if (b) then\n#endif\n      x=1\n   end if\nend program\n";
        assert_eq!(
            format_source(input, &indent_only_config()).unwrap().bytes,
            expected
        );
    }

    #[test]
    fn empty_queries_have_stable_results() {
        let indent = FormatConfig {
            last_indent: true,
            ..indent_only_config()
        };
        assert_eq!(format_source(b"", &indent).unwrap().bytes, b"0\n");
        let usable = FormatConfig {
            last_usable: true,
            ..indent_only_config()
        };
        assert_eq!(format_source(b"", &usable).unwrap().bytes, b"1\n");
    }

    #[test]
    fn queries_emit_only_the_requested_metadata() {
        let input = b"program p\nx=1\n";
        let indent = FormatConfig {
            last_indent: true,
            ..indent_only_config()
        };
        assert_eq!(format_source(input, &indent).unwrap().bytes, b"3\n");

        let usable = FormatConfig {
            last_usable: true,
            ..indent_only_config()
        };
        assert_eq!(format_source(input, &usable).unwrap().bytes, b"2\n");

        let both = FormatConfig {
            last_indent: true,
            last_usable: true,
            ..indent_only_config()
        };
        assert_eq!(format_source(input, &both).unwrap().bytes, b"2\n");
    }

    #[test]
    fn preprocessor_continuations_keep_their_generation_and_spelling() {
        let input = b"program p\n#if defined(X) \\\n  && defined(Y)\nif (x) then\n#else\nif (y) then\n#endif\nx=1\nend if\nend program\n";
        let expected = b"program p\n#if defined(X) \\\n  && defined(Y)\n   if (x) then\n#else\n   if (y) then\n#endif\n      x=1\n   end if\nend program\n";
        assert_eq!(
            format_source(input, &indent_only_config()).unwrap().bytes,
            expected
        );
    }

    #[test]
    fn openmp_sentinel_replaces_stale_body_indentation() {
        let input = b"program p\nif (x) then\n!$      call work()\nend if\nend program\n";
        let expected = b"program p\n   if (x) then\n!$    call work()\n   end if\nend program\n";
        assert_eq!(
            format_source(input, &indent_only_config()).unwrap().bytes,
            expected
        );
    }

    #[test]
    fn openmp_near_misses_remain_comments_and_trim_empty_sentinels() {
        let input = b"program p\n!$\n!$omp parallel\n!$\tcall x\nend program\n";
        let expected = b"program p\n!$\n!$omp parallel\n!$ call x\nend program\n";
        assert_eq!(
            format_source(input, &indent_only_config()).unwrap().bytes,
            expected
        );
    }

    #[test]
    fn disabling_openmp_treats_the_sentinel_as_ordinary_source() {
        let input = b"program p\nif (x) then\n   !$      call work()\nend if\nend program\n";
        let expected =
            b"program p\n   if (x) then\n      !$      call work()\n   end if\nend program\n";
        let config = FormatConfig {
            openmp: false,
            ..indent_only_config()
        };
        assert_eq!(format_source(input, &config).unwrap().bytes, expected);
    }

    #[test]
    fn findentfix_debug_toggles_are_inert() {
        let input = b"program p\n! findentfix:p-on\n! findentfix:p-off\nx=1\nend program\n";
        let expected = b"program p\n! findentfix:p-on\n! findentfix:p-off\n   x=1\nend program\n";
        assert_eq!(
            format_source(input, &indent_only_config()).unwrap().bytes,
            expected
        );
    }

    #[test]
    fn one_label_closes_multiple_labeled_do_loops() {
        let input = b"program p\ndo 100 i=1,10\ndo 100 j=1,10\n100 continue\nend program\n";
        let expected =
            b"program p\n   do 100 i=1,10\n      do 100 j=1,10\n100 continue\nend program\n";
        assert_eq!(
            format_source(input, &indent_only_config()).unwrap().bytes,
            expected
        );
    }

    #[test]
    fn malformed_end_statements_do_not_consume_an_unrelated_frame() {
        let input = b"program p\nif (x) then\nend do\ny=1\nend if\nend program\n";
        let expected =
            b"program p\n   if (x) then\n      end do\n      y=1\n   end if\nend program\n";
        assert_eq!(
            format_source(input, &indent_only_config()).unwrap().bytes,
            expected
        );
    }

    #[test]
    fn malformed_definition_end_recovers_without_losing_the_active_construct() {
        let input = b"program p\nif (x) then\nend subroutine\ncontinue\nend if\nend program\n";
        let expected =
            b"program p\n   if (x) then\n   end subroutine\n   continue\nend if\nend program\n";
        assert_eq!(
            format_source(input, &indent_only_config()).unwrap().bytes,
            expected
        );
    }
}
