//! What the invocation says on stderr: declined wraps, and skipped fixed-form
//! sources.
//!
//! The formatter keeps paths out of [`crate::FormatMeta`] so the library API
//! stays about a source buffer. The CLI is the layer that has the path, so it
//! is also the layer that names the file in a diagnostic.

use super::sources::display_path;
use crate::{cli::Invocation, source::SourceForm};
use std::{collections::HashSet, path::Path};

/// Bounded declined-wrap diagnostics for one CLI invocation.
///
/// The formatter deliberately keeps paths out of [`crate::FormatMeta`] so the
/// library API remains about a source buffer.  The CLI has the path, and adds
/// it here while combining diagnostics from all formatted targets.
#[derive(Default)]
pub(super) struct DeclineReporter {
    reported: usize,
    suppressed: usize,
    suppressed_inputs: HashSet<String>,
    suppressed_stdin: bool,
}

/// Keep routine formatting invocations useful when a generated source has
/// hundreds of equally-unwrappable statements.  Five concrete locations are
/// enough to identify the condition; the remainder are counted below.
const DECLINE_DIAGNOSTIC_LIMIT: usize = 5;

impl DeclineReporter {
    pub(super) fn report_fixed(&mut self, path: &Path, root: Option<&Path>) {
        let input = display_path(path, root).display().to_string();
        eprintln!("{}", fixed_message(&input));
    }

    pub(super) fn report(
        &mut self,
        meta: &crate::FormatMeta,
        path: Option<&Path>,
        root: Option<&Path>,
    ) {
        let input = input_name(path, root);
        for (line, reason) in &meta.declines {
            if self.reported < DECLINE_DIAGNOSTIC_LIMIT {
                eprintln!("{}", decline_message(&input, *line, *reason));
                self.reported += 1;
            } else {
                self.suppressed += 1;
                self.suppressed_stdin |= path.is_none();
                self.suppressed_inputs.insert(input.clone());
            }
        }
    }

    pub(super) fn finish(&self) {
        if let Some(message) = self.summary() {
            eprintln!("{message}");
        }
    }

    pub(super) fn summary(&self) -> Option<String> {
        (self.suppressed > 0).then(|| {
            let inputs = self.suppressed_inputs.len();
            let input_word = if self.suppressed_stdin {
                if inputs == 1 {
                    "input"
                } else {
                    "inputs"
                }
            } else if inputs == 1 {
                "file"
            } else {
                "files"
            };
            format!(
                "forformat: + {} additional declined-wrap diagnostics in {inputs} {input_word}",
                self.suppressed
            )
        })
    }
}

/// How a diagnostic names one formatted input.
///
/// Every route that can report against an input uses this, so a failure names
/// the file exactly the way a declined wrap on the same file would.
pub(super) fn input_name(path: Option<&Path>, root: Option<&Path>) -> String {
    path.map(|path| display_path(path, root).display().to_string())
        .unwrap_or_else(|| "<stdin>".to_owned())
}

pub(super) fn decline_message(
    input: &str,
    line: usize,
    reason: crate::format::wrapping::Decline,
) -> String {
    format!("forformat: {input}:{}: declined wrap: {reason:?}", line + 1)
}

pub(super) fn fixed_message(input: &str) -> String {
    format!("forformat: {input}: fixed-form source, skipped")
}

/// Should this unnamed buffer be declined as fixed form?
///
/// Two carve-outs beyond the `-ifree` override. A buffer with no non-blank byte
/// has nothing to protect, and findent's detector answers FIXED at EOF, so
/// without this every content-free invocation — `forformat </dev/null` among
/// them — would report a skip. And `-lastindent`/`-lastusable` only report on
/// the source rather than rewriting it, so there is nothing to decline.
pub(super) fn skips_fixed_form(
    invocation: &Invocation,
    input_path: Option<&Path>,
    source: &[u8],
) -> bool {
    !invocation.force_free_input
        && !invocation.config.last_indent
        && !invocation.config.last_usable
        && source.iter().any(|byte| !byte.is_ascii_whitespace())
        && input_path.map_or_else(
            || crate::source::detect(source),
            |path| crate::source::detect_path(path, source),
        ) == SourceForm::Fixed
}

#[cfg(test)]
mod tests {
    use super::{decline_message, DeclineReporter};
    use crate::format::wrapping::Decline;

    #[test]
    fn declined_wrap_diagnostics_include_the_input_and_bound_the_summary() {
        assert_eq!(
            decline_message("src/example.f90", 41, Decline::NoSafeBreak),
            "forformat: src/example.f90:42: declined wrap: NoSafeBreak"
        );

        let mut reporter = DeclineReporter {
            suppressed: 7,
            ..Default::default()
        };
        reporter
            .suppressed_inputs
            .insert("src/example.f90".to_owned());
        reporter
            .suppressed_inputs
            .insert("src/another.f90".to_owned());
        assert_eq!(
            reporter.summary().as_deref(),
            Some("forformat: + 7 additional declined-wrap diagnostics in 2 files")
        );

        let mut stdin_reporter = DeclineReporter {
            suppressed: 1,
            suppressed_stdin: true,
            ..Default::default()
        };
        stdin_reporter
            .suppressed_inputs
            .insert("<stdin>".to_owned());
        assert_eq!(
            stdin_reporter.summary().as_deref(),
            Some("forformat: + 1 additional declined-wrap diagnostics in 1 input")
        );
    }
}
