//! Project-wide context: the declarations of every source, read once.
//!
//! A single `&[u8]` cannot express project context, so it is built separately
//! and handed to the formatter.  Gate G of the port plan is about this object:
//! the project sources are read and analyzed **once per invocation**, not once
//! per formatted file.

use super::{
    declarations::{extract, FileFacts},
    names::{CaseMap, CaseResolver, CaseTables},
    scope::ScopeTree,
};
use crate::{config::MacroDefine, error::FormatError, transform::document::Document};
use std::path::{Path, PathBuf};

/// The union of every project source's declarations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectContext {
    /// Merged spellings.  A name spelled differently in two files is ambiguous
    /// project-wide and is left alone (I4).
    pub cases: CaseTables,
    /// Macro names from every `#define` in the project, plus any `-D` names.
    pub macros: CaseMap,
    /// Merged type maps for `%` chain resolution.
    pub types: super::declarations::TypeMaps,
    /// The files that contributed, in the order analyzed.
    pub sources: Vec<PathBuf>,
}

impl ProjectContext {
    /// The context used when a caller formats a lone buffer with no project.
    /// Every existing single-buffer entry point behaves as if it passed this.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Analyze one source and fold it in.
    pub fn add_source(&mut self, path: &Path, source: &[u8]) -> Result<(), FormatError> {
        let facts = analyze_file(source)?;
        self.absorb(path, &facts);
        Ok(())
    }

    /// Fold already-extracted facts in, so a caller that analyzes files in
    /// parallel can merge the results deterministically by sorting first.
    pub fn absorb(&mut self, path: &Path, facts: &FileFacts) {
        self.cases.merge(&facts.cases);
        self.macros.merge(&facts.macros);
        self.types.merge(&facts.types);
        self.sources.push(path.to_path_buf());
    }

    /// Add command-line macro definitions.  These are recorded exactly as
    /// spelled and outrank every declaration.
    pub fn define(&mut self, defines: &[MacroDefine]) {
        for define in defines {
            self.macros.insert(define.name.as_bytes());
        }
    }

    /// Bind this context to one file's own declarations, producing the object
    /// that answers "how should this identifier be spelled here?".
    pub fn resolver<'a>(&'a self, local: &'a FileFacts) -> CaseResolver<'a> {
        CaseResolver {
            local: &local.cases,
            project: &self.cases,
            macros: &self.macros,
        }
    }
}

/// Extract the declaration facts of one source buffer.
pub fn analyze_file(source: &[u8]) -> Result<FileFacts, FormatError> {
    let document = Document::from_bytes(source);
    let analysis = document.analyze()?;
    let scopes = ScopeTree::build(&analysis);
    Ok(extract(&analysis, &scopes))
}

/// Build a project context from every source in the project.
///
/// Sources are consumed in the order given; the result is order independent
/// except for `sources`, because merging is commutative on agreement and
/// collapses to "ambiguous" on disagreement.
pub fn analyze_project<'a, I>(sources: I) -> Result<ProjectContext, FormatError>
where
    I: IntoIterator<Item = (&'a Path, &'a [u8])>,
{
    let mut context = ProjectContext::empty();
    for (path, source) in sources {
        context.add_source(path, source)?;
    }
    Ok(context)
}

#[cfg(test)]
mod tests {
    use super::{analyze_file, analyze_project, ProjectContext};
    use crate::{analysis::names::NameSpace, config::MacroDefine};
    use std::path::Path;

    const MODULE: &[u8] = b"module Precision\ninteger, parameter :: dl = 8\nend module Precision\n";
    const USER: &[u8] = b"program p\nuse Precision\nend program p\n";
    const SHOUTER: &[u8] = b"program q\nuse PRECISION\nend program q\n";

    #[test]
    fn a_project_wide_agreement_applies_to_a_file_that_does_not_declare_the_name() {
        let project = analyze_project([
            (Path::new("precision.f90"), MODULE),
            (Path::new("user.f90"), USER),
        ])
        .unwrap();
        let local = analyze_file(b"program r\nend program r\n").unwrap();
        let resolver = project.resolver(&local);
        assert_eq!(
            resolver.spelling(NameSpace::Module, b"precision"),
            Some(b"Precision".as_slice())
        );
        assert_eq!(project.sources.len(), 2);
    }

    #[test]
    fn project_wide_disagreement_leaves_the_name_alone() {
        let project =
            analyze_project([(Path::new("a.f90"), MODULE), (Path::new("b.f90"), SHOUTER)]).unwrap();
        let local = analyze_file(b"program r\nend program r\n").unwrap();
        assert_eq!(
            project
                .resolver(&local)
                .spelling(NameSpace::Module, b"precision"),
            None
        );
    }

    #[test]
    fn a_local_spelling_still_wins_over_the_project() {
        let project = analyze_project([(Path::new("a.f90"), MODULE)]).unwrap();
        let local = analyze_file(b"program r\nuse PRECISION\nend program r\n").unwrap();
        assert_eq!(
            project
                .resolver(&local)
                .spelling(NameSpace::Module, b"precision"),
            Some(b"PRECISION".as_slice())
        );
    }

    #[test]
    fn merging_is_order_independent_for_the_case_tables() {
        let forward = analyze_project([(Path::new("a"), MODULE), (Path::new("b"), USER)]).unwrap();
        let backward = analyze_project([(Path::new("b"), USER), (Path::new("a"), MODULE)]).unwrap();
        assert_eq!(forward.cases, backward.cases);
    }

    #[test]
    fn command_line_defines_join_the_macro_table() {
        let mut project = ProjectContext::empty();
        project.define(&[MacroDefine {
            name: "MPI_Enabled".to_string(),
            value: None,
        }]);
        let local = analyze_file(b"program p\nend\n").unwrap();
        assert_eq!(
            project
                .resolver(&local)
                .spelling(NameSpace::Symbol, b"mpi_enabled"),
            Some(b"MPI_Enabled".as_slice())
        );
    }
}
