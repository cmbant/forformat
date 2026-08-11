//! Case resolution.  This module owns invariant I4: **project casing never
//! guesses.**
//!
//! Three rules, in order:
//!
//! 1. a spelling the file itself declares wins, but only if the file declares
//!    exactly one spelling for that name;
//! 2. otherwise a spelling the whole project agrees on wins;
//! 3. otherwise nothing happens — an ambiguous name keeps whatever the author
//!    wrote.
//!
//! Intrinsics and standard specifiers sit *below* declared identifiers, because
//! a local variable named `size` or `precision` is a real thing and renaming its
//! case would be a lie about the code.  CPP macro names sit *above* everything,
//! because their spelling is fixed by the preprocessor.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    /// Exactly one spelling has been seen.
    Unique(Vec<u8>),
    /// Two or more spellings differ: the name is off limits.
    Ambiguous,
}

/// A case-insensitive name index that remembers whether one spelling is
/// unambiguous.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaseMap {
    entries: HashMap<Vec<u8>, Entry>,
}

impl CaseMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a declared spelling.  A second, byte-different spelling of the
    /// same name makes it ambiguous permanently.
    pub fn insert(&mut self, spelling: &[u8]) {
        if spelling.is_empty() {
            return;
        }
        let key = spelling.to_ascii_lowercase();
        match self.entries.get(&key) {
            None => {
                self.entries.insert(key, Entry::Unique(spelling.to_vec()));
            }
            Some(Entry::Unique(existing)) if existing.as_slice() != spelling => {
                self.entries.insert(key, Entry::Ambiguous);
            }
            _ => {}
        }
    }

    /// The single agreed spelling of `name`, if there is one.
    pub fn get(&self, name: &[u8]) -> Option<&[u8]> {
        match self.entries.get(&name.to_ascii_lowercase())? {
            Entry::Unique(spelling) => Some(spelling),
            Entry::Ambiguous => None,
        }
    }

    /// True when the name is known but spelled inconsistently.
    pub fn is_ambiguous(&self, name: &[u8]) -> bool {
        matches!(
            self.entries.get(&name.to_ascii_lowercase()),
            Some(Entry::Ambiguous)
        )
    }

    /// True when the name has been declared at all, however spelled.
    pub fn contains(&self, name: &[u8]) -> bool {
        self.entries.contains_key(&name.to_ascii_lowercase())
    }

    /// Fold another map in.  Disagreement between the two makes the name
    /// ambiguous, which is how a project-wide map is built from per-file maps.
    pub fn merge(&mut self, other: &CaseMap) {
        for (key, entry) in &other.entries {
            match (self.entries.get(key), entry) {
                (None, entry) => {
                    self.entries.insert(key.clone(), entry.clone());
                }
                (Some(Entry::Ambiguous), _) => {}
                (Some(Entry::Unique(_)), Entry::Ambiguous) => {
                    self.entries.insert(key.clone(), Entry::Ambiguous);
                }
                (Some(Entry::Unique(mine)), Entry::Unique(theirs)) => {
                    if mine != theirs {
                        self.entries.insert(key.clone(), Entry::Ambiguous);
                    }
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The five independent name spaces the reference formatter tracks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaseTables {
    /// `MODULE` names and the names in `USE` statements.
    pub modules: CaseMap,
    /// Procedure names, module variables, arguments, locals.
    pub symbols: CaseMap,
    /// Derived-type names.
    pub types: CaseMap,
    /// Derived-type component names.
    pub components: CaseMap,
    /// Type-bound procedure names.
    pub type_procedures: CaseMap,
}

impl CaseTables {
    pub fn merge(&mut self, other: &CaseTables) {
        self.modules.merge(&other.modules);
        self.symbols.merge(&other.symbols);
        self.types.merge(&other.types);
        self.components.merge(&other.components);
        self.type_procedures.merge(&other.type_procedures);
    }
}

/// Which name space a use site belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameSpace {
    Module,
    Symbol,
    Type,
    Component,
    TypeProcedure,
}

impl NameSpace {
    fn select(self, tables: &CaseTables) -> &CaseMap {
        match self {
            NameSpace::Module => &tables.modules,
            NameSpace::Symbol => &tables.symbols,
            NameSpace::Type => &tables.types,
            NameSpace::Component => &tables.components,
            NameSpace::TypeProcedure => &tables.type_procedures,
        }
    }
}

/// The I4 decision for one name space.
///
/// `local` is the file under formatting; `project` is every source in the
/// project, including that file.
pub fn resolve<'a>(local: &'a CaseMap, project: &'a CaseMap, name: &[u8]) -> Option<&'a [u8]> {
    if local.contains(name) {
        // The file speaks for itself, whether or not it agrees with the rest of
        // the project.  If it contradicts itself, nothing is safe to do.
        return local.get(name);
    }
    project.get(name)
}

/// Everything needed to decide the spelling of one identifier occurrence.
#[derive(Debug, Clone, Copy)]
pub struct CaseResolver<'a> {
    pub local: &'a CaseTables,
    pub project: &'a CaseTables,
    /// Names defined by `-D` or by a `#define` anywhere in the project.  These
    /// outrank declarations and intrinsics.
    pub macros: &'a CaseMap,
}

impl<'a> CaseResolver<'a> {
    /// The spelling to emit for `name`, or `None` to leave the source alone.
    pub fn spelling(&self, space: NameSpace, name: &[u8]) -> Option<&'a [u8]> {
        if let Some(spelling) = self.macros.get(name) {
            return Some(spelling);
        }
        if self.macros.is_ambiguous(name) {
            return None;
        }
        resolve(space.select(self.local), space.select(self.project), name)
    }

    /// The spelling for an ordinary identifier occurrence: a declared name if
    /// the project knows one, otherwise the canonical lowercase intrinsic or
    /// standard-word spelling, otherwise nothing.
    ///
    /// The order is the point.  A file that declares `Size` as a variable keeps
    /// `Size`; a file that does not gets the intrinsic's `size`.
    pub fn identifier(&self, name: &[u8]) -> Option<&'a [u8]> {
        if let Some(spelling) = self.spelling(NameSpace::Symbol, name) {
            return Some(spelling);
        }
        if self.declared_filewide(name) {
            return None;
        }
        crate::transform::vocab::lookup(crate::transform::vocab::INTRINSIC_NAMES, name)
            .or_else(|| {
                crate::transform::vocab::lookup(crate::transform::vocab::FORTRAN_SPECIFIERS, name)
            })
            .map(str::as_bytes)
    }

    /// True when a name space that can shadow a keyword declares this name,
    /// however spelled.  An intrinsic must not override such a name (I4).
    ///
    /// Type-bound procedures are deliberately excluded.  They are only ever
    /// reached through `%` or as the target of a binding, both of which have
    /// their own guards, and the reference keeps them out of the name set that
    /// suppresses keyword lowering — `test_type_bound_procedures_only_supply_
    /// component_case` asserts exactly that.  Including them meant a single
    /// `procedure :: Close => …` stopped `close(unit)` being lowercased
    /// anywhere in the file.
    ///
    /// File-wide declaration predicate retained for the identifier case pass.
    ///
    /// The keyword normalizer does not use this method: its reference path is
    /// `declared_names_at(line)`, with a separate active-procedure local set.
    /// Keeping this predicate here preserves the existing Chunk C contract
    /// until case application gets its own scope-aware lookup.
    pub fn declared_filewide(&self, name: &[u8]) -> bool {
        [
            (&self.local.symbols, &self.project.symbols),
            (&self.local.types, &self.project.types),
            (&self.local.components, &self.project.components),
        ]
        .into_iter()
        .any(|(local, project)| local.contains(name) || project.contains(name))
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve, CaseMap, CaseResolver, CaseTables, NameSpace};

    #[test]
    fn one_spelling_is_unique_and_two_are_ambiguous() {
        let mut map = CaseMap::new();
        map.insert(b"CAMBdata");
        assert_eq!(map.get(b"cambdata"), Some(b"CAMBdata".as_slice()));
        map.insert(b"CAMBdata");
        assert_eq!(map.get(b"CAMBDATA"), Some(b"CAMBdata".as_slice()));
        map.insert(b"CAMBData");
        assert_eq!(map.get(b"cambdata"), None);
        assert!(map.is_ambiguous(b"cambdata"));
        assert!(map.contains(b"cambdata"));
    }

    #[test]
    fn a_local_declaration_outranks_a_project_agreement() {
        let mut local = CaseMap::new();
        local.insert(b"myVar");
        let mut project = CaseMap::new();
        project.insert(b"MyVar");
        assert_eq!(
            resolve(&local, &project, b"myvar"),
            Some(b"myVar".as_slice())
        );
    }

    #[test]
    fn local_ambiguity_blocks_the_project_spelling() {
        let mut local = CaseMap::new();
        local.insert(b"myVar");
        local.insert(b"MYVAR");
        let mut project = CaseMap::new();
        project.insert(b"MyVar");
        assert_eq!(resolve(&local, &project, b"myvar"), None);
    }

    #[test]
    fn a_name_the_file_does_not_declare_takes_the_project_spelling() {
        let local = CaseMap::new();
        let mut project = CaseMap::new();
        project.insert(b"MatterPower");
        assert_eq!(
            resolve(&local, &project, b"matterpower"),
            Some(b"MatterPower".as_slice())
        );
        let mut split = CaseMap::new();
        split.insert(b"MatterPower");
        split.insert(b"Matterpower");
        assert_eq!(resolve(&local, &split, b"matterpower"), None);
    }

    #[test]
    fn merging_per_file_maps_detects_project_wide_disagreement() {
        let mut a = CaseMap::new();
        a.insert(b"Foo");
        let mut b = CaseMap::new();
        b.insert(b"Foo");
        let mut c = CaseMap::new();
        c.insert(b"FOO");
        let mut project = CaseMap::new();
        project.merge(&a);
        project.merge(&b);
        assert_eq!(project.get(b"foo"), Some(b"Foo".as_slice()));
        project.merge(&c);
        assert_eq!(project.get(b"foo"), None);
    }

    #[test]
    fn macros_outrank_declarations_and_declarations_outrank_intrinsics() {
        let mut local = CaseTables::default();
        local.symbols.insert(b"Size");
        let project = CaseTables::default();
        let mut macros = CaseMap::new();
        macros.insert(b"CAMB_DEBUG");

        let resolver = CaseResolver {
            local: &local,
            project: &project,
            macros: &macros,
        };
        assert_eq!(
            resolver.spelling(NameSpace::Symbol, b"camb_debug"),
            Some(b"CAMB_DEBUG".as_slice())
        );
        // Declared locally, so the intrinsic spelling must not win.
        assert_eq!(resolver.identifier(b"SIZE"), Some(b"Size".as_slice()));

        let empty = CaseTables::default();
        let plain = CaseResolver {
            local: &empty,
            project: &empty,
            macros: &CaseMap::new(),
        };
        assert_eq!(plain.identifier(b"SIZE"), Some(b"size".as_slice()));
        assert_eq!(plain.identifier(b"not_a_known_name"), None);
    }
}
