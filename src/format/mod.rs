mod context;
pub mod continuation;
pub mod emitter;
pub mod engine;

// Keep the established full-mode driver intact and put cross-mode composition
// in the public facade. This lets the combined mode be literally the output of
// canonicalize-only fed to the existing indent-only engine, while every
// pre-existing mode continues through the same implementation as before.
#[path = "full.rs"]
mod full_impl;

pub mod full {
    use crate::{
        analysis::{analyze_file, FileFacts, ProjectContext},
        config::{FormatConfig, FormatMode},
        error::FormatError,
        FormatMeta, FormatResult,
    };

    pub use super::full_impl::{reflow, reflow_with_context};

    pub fn format_with_context(
        source: &[u8],
        project: &ProjectContext,
        config: &FormatConfig,
    ) -> Result<FormatResult, FormatError> {
        if config.mode != FormatMode::CanonicalizeAndIndent {
            return super::full_impl::format_with_context(source, project, config);
        }
        let local = analyze_file(source)?;
        canonicalize_and_indent_with_local(source, project, &local, config)
    }

    pub(crate) fn format_with_context_and_local(
        source: &[u8],
        project: &ProjectContext,
        local: &FileFacts,
        config: &FormatConfig,
    ) -> Result<FormatResult, FormatError> {
        if config.mode != FormatMode::CanonicalizeAndIndent {
            return super::full_impl::format_with_context_and_local(source, project, local, config);
        }
        canonicalize_and_indent_with_local(source, project, local, config)
    }

    pub(crate) fn format_to_with_context<W: std::io::Write>(
        source: &[u8],
        project: &ProjectContext,
        config: &FormatConfig,
        out: &mut W,
    ) -> Result<FormatMeta, FormatError> {
        if config.mode != FormatMode::CanonicalizeAndIndent {
            return super::full_impl::format_to_with_context(source, project, config, out);
        }
        let local = analyze_file(source)?;
        let result = canonicalize_and_indent_with_local(source, project, &local, config)?;
        std::io::Write::write_all(out, &result.bytes).map_err(FormatError::Write)?;
        Ok(result.meta)
    }

    fn canonicalize_and_indent_with_local(
        source: &[u8],
        project: &ProjectContext,
        local: &FileFacts,
        config: &FormatConfig,
    ) -> Result<FormatResult, FormatError> {
        let mut canonical = config.clone();
        canonical.mode = FormatMode::CanonicalizeOnly;
        let canonicalized =
            super::full_impl::format_with_context_and_local(source, project, local, &canonical)?;

        let mut indent = config.clone();
        indent.mode = FormatMode::IndentOnly;
        super::engine::format(&canonicalized.bytes, &indent)
    }
}

pub mod planner;
pub mod preprocessor;
pub mod stack;
pub mod wrapping;
