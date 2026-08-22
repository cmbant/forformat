use super::{
    includes::{analyze_file_at, expand_includes_with, normalize_path},
    ProjectContext,
};
use crate::{
    analysis::{
        declarations::{FileFacts, HostUnit, UnitFacts},
        names::CaseResolver,
    },
    config::MacroDefine,
    error::FormatError,
};
use std::{fs, path::Path};

impl ProjectContext {
    /// The context used when a caller formats a lone buffer with no project.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Analyze one source and fold it in. Relative Fortran INCLUDE statements
    /// are resolved from `path` when the files exist on disk.
    pub fn add_source(&mut self, path: &Path, source: &[u8]) -> Result<(), FormatError> {
        let facts = analyze_file_at(path, source)?;
        self.absorb(path, &facts);
        Ok(())
    }

    /// Fold already-extracted facts in. INCLUDE fragments are analyzed on
    /// demand from the filesystem, without making their declarations globally
    /// visible: they are merged into the scope containing the INCLUDE statement.
    ///
    /// This reads files. Every relative INCLUDE in `facts` is resolved against
    /// `path`'s directory and every absolute one as written, following the same
    /// text-substitution rule a compiler applies, so a caller that filters which
    /// sources reach the project does not thereby bound which files are opened.
    /// Unreadable and unparsable fragments are skipped rather than reported.
    /// Use [`ProjectContext::empty`] alone when no filesystem access is wanted.
    pub fn absorb(&mut self, path: &Path, facts: &FileFacts) {
        let expanded = expand_includes_with(path, facts, &mut |candidate| {
            fs::read(candidate)
                .ok()
                .and_then(|source| analyze_file_at(candidate, &source).ok())
        });
        self.absorb_expanded(path, facts, expanded);
    }

    /// `expanded` is consumed: it is retained verbatim as this path's scope
    /// facts, and every caller has already finished with it.
    pub(super) fn absorb_expanded(&mut self, path: &Path, facts: &FileFacts, expanded: FileFacts) {
        self.cases.merge(&facts.cases);
        // Included owner-qualified members are safe to add to the project type
        // graph; included root declarations remain in expanded scope facts.
        self.cases.components.merge(&expanded.cases.components);
        self.cases
            .bound_type_procedures
            .merge(&expanded.cases.bound_type_procedures);
        self.file_symbols.merge(&facts.file_symbols);
        self.external_symbols.merge(&facts.external_symbols);
        self.generic_type_procedures
            .merge(&facts.generic_type_procedures);
        self.generic_bound_type_procedures
            .merge(&expanded.generic_bound_type_procedures);
        self.declared_types.merge(&facts.declared_types);
        self.macros.merge(&expanded.macros);
        self.types.merge(&facts.types);
        self.types.merge_non_roots(&expanded.types);
        self.register_modules(&expanded);
        let path = normalize_path(path);
        if let Some((previous_id, _)) = self.expanded_facts.get(&path) {
            if previous_id != &facts.source_id {
                self.expanded_sources.insert(*previous_id, None);
            }
        }
        match self.expanded_sources.get_mut(&facts.source_id) {
            None => {
                self.expanded_sources
                    .insert(facts.source_id, Some(path.clone()));
            }
            Some(Some(existing)) if existing == &path => {}
            Some(slot) => *slot = None,
        }
        self.expanded_facts
            .insert(path.clone(), (facts.source_id, expanded));
        self.sources.push(path);
    }

    /// Expand `facts`'s own INCLUDE directives when this context holds no
    /// expansion that describes them.
    ///
    /// [`ProjectContext::expanded`] matches a stored expansion by path *and*
    /// source fingerprint, so a buffer edited since the context was built — an
    /// editor's unsaved change — matches nothing and falls back to the local
    /// facts. Without this it would fall back to *unexpanded* local facts and
    /// silently lose every declaration its fragments contribute, so a file
    /// would resolve differently for the sole reason that it was dirty.
    ///
    /// This reads files, on the same terms as [`ProjectContext::absorb`].
    pub(crate) fn expand_uncached(&self, path: &Path, facts: FileFacts) -> FileFacts {
        if facts.includes.is_empty() || self.holds_expansion_of(&facts) {
            return facts;
        }
        expand_includes_with(path, &facts, &mut |candidate| {
            fs::read(candidate)
                .ok()
                .and_then(|source| analyze_file_at(candidate, &source).ok())
        })
    }

    /// Whether `expanded` would answer from this context rather than fall back
    /// to the caller's own facts.
    fn holds_expansion_of(&self, facts: &FileFacts) -> bool {
        facts.source_path.as_deref().is_some_and(|path| {
            self.expanded_facts
                .get(path)
                .is_some_and(|(source_id, _)| *source_id == facts.source_id)
        })
    }

    fn register_modules(&mut self, facts: &FileFacts) {
        // Scope order rather than map order, so the registered sequence — and
        // with it this context's identity — does not depend on hashing.
        let mut scopes = facts.units.keys().copied().collect::<Vec<_>>();
        scopes.sort_unstable();
        for scope in scopes {
            self.register_unit(&facts.units[&scope]);
        }
        // A module defined inside an INCLUDE fragment is a project entity even
        // though it has no place in the including file's scope tree. Several
        // files including one fragment register the identical unit, which
        // deduplicates below rather than reading as a vendored duplicate.
        for unit in &facts.included_units {
            self.register_unit(unit);
        }
    }

    fn register_unit(&mut self, unit: &UnitFacts) {
        let Some(host) = unit.project_host.as_ref() else {
            return;
        };
        let definitions = match host {
            HostUnit::Module(name) => self.modules.entry(name.clone()).or_default(),
            HostUnit::Submodule { ancestor, name } => self
                .submodules
                .entry((ancestor.clone(), name.clone()))
                .or_default(),
        };
        // Re-absorbing the same source must not register a second copy.
        if !definitions.iter().any(|existing| existing == unit) {
            definitions.push(unit.clone());
        }
    }

    /// Add command-line macro definitions. These are recorded exactly as
    /// spelled and outrank every declaration.
    pub fn define(&mut self, defines: &[MacroDefine]) {
        for define in defines {
            self.macros.insert(define.name.as_bytes());
        }
    }

    pub fn enable_target_local_component_resolution(&mut self) {
        self.target_local_component_resolution = true;
    }

    /// Bind this context to one file's own declarations for namespaces that
    /// remain genuinely file/project-wide.
    pub fn resolver<'a>(&'a self, local: &'a FileFacts) -> CaseResolver<'a> {
        CaseResolver {
            local: &local.cases,
            project: &self.cases,
            macros: &self.macros,
        }
    }

    pub(super) fn expanded<'a>(&'a self, local: &'a FileFacts) -> &'a FileFacts {
        // Every name token of every query lands here, so borrow the stored
        // path rather than rebuilding it: `analyze_file_with_path` normalized
        // `source_path` when it produced these facts, and `absorb_expanded`
        // keyed `expanded_facts` by that same normalized path.
        let path = local.source_path.as_deref().or_else(|| {
            self.expanded_sources
                .get(&local.source_id)
                .and_then(|path| path.as_deref())
        });
        let Some(path) = path else {
            return local;
        };
        self.expanded_facts
            .get(path)
            .filter(|(source_id, _)| *source_id == local.source_id)
            .map(|(_, facts)| facts)
            .unwrap_or(local)
    }

    /// Every definition registered for one module name, in registration order.
    pub(super) fn module_units(&self, module: &[u8]) -> &[UnitFacts] {
        self.modules
            .get(module)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Every definition registered for one host program unit. Empty when the
    /// project never supplied that module or submodule.
    pub(super) fn host_units(&self, host: &HostUnit) -> &[UnitFacts] {
        match host {
            HostUnit::Module(name) => self.module_units(name),
            HostUnit::Submodule { ancestor, name } => self
                .submodules
                .get(&(ancestor.clone(), name.clone()))
                .map(Vec::as_slice)
                .unwrap_or_default(),
        }
    }
}
