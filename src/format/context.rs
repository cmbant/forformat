use crate::{
    analysis::{analyze_file_at, ProjectContext},
    config::FormatConfig,
    error::FormatError,
    FormatResult,
};
use std::path::Path;

impl ProjectContext {
    /// Format one project member with its path identity preserved for relative
    /// Fortran `INCLUDE` resolution.
    ///
    /// This is the path-aware counterpart to [`crate::format_source_with_context`].
    /// It is needed when identical source buffers live in different directories
    /// and therefore resolve the same relative include spelling differently.
    pub fn format_source_at(
        &self,
        path: &Path,
        source: &[u8],
        config: &FormatConfig,
    ) -> Result<FormatResult, FormatError> {
        if !config.mode.normalizes() {
            return super::engine::format(source, config);
        }
        // Expanding here rather than trusting the context's cache is what keeps
        // an edited buffer equivalent to the saved one: the cache is keyed by
        // source fingerprint, and an unsaved edit matches no entry.
        let local = self.expand_uncached(path, analyze_file_at(path, source)?);
        super::full::format_with_context_and_local(source, self, &local, config)
    }
}
