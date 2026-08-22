use super::ProjectContext;
use crate::{
    analysis::{
        declarations::{extract, FileFacts},
        scope::ScopeTree,
    },
    error::FormatError,
    transform::document::Document,
};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
};

pub(super) fn expand_includes_with(
    path: &Path,
    facts: &FileFacts,
    loader: &mut impl FnMut(&Path) -> Option<FileFacts>,
) -> FileFacts {
    fn visit(
        path: &Path,
        facts: &FileFacts,
        loader: &mut impl FnMut(&Path) -> Option<FileFacts>,
        stack: &mut HashSet<PathBuf>,
    ) -> FileFacts {
        let mut expanded = facts.clone();
        for include in &facts.includes {
            let include_path = path_from_bytes(&include.path);
            let candidate = if include_path.is_absolute() {
                include_path
            } else {
                path.parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(include_path)
            };
            let candidate = normalize_path(&candidate);
            if !stack.insert(candidate.clone()) {
                continue;
            }
            if let Some(included) = loader(&candidate) {
                let included = visit(&candidate, &included, loader, stack);
                expanded.merge_include_at(include.line, &included);
            }
            stack.remove(&candidate);
        }
        expanded
    }

    let mut stack = HashSet::new();
    stack.insert(normalize_path(path));
    visit(path, facts, loader, &mut stack)
}

fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        PathBuf::from(OsString::from_vec(bytes.to_vec()))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

pub(super) fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    let rooted = path.has_root();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match normalized.components().next_back() {
                Some(Component::Normal(_)) => {
                    normalized.pop();
                }
                Some(Component::ParentDir) | Some(Component::Prefix(_)) | None => {
                    if !rooted {
                        normalized.push("..");
                    }
                }
                Some(Component::RootDir) => {}
                Some(Component::CurDir) => {
                    unreachable!("normalized paths do not retain current-directory components")
                }
            },
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn fingerprint(source: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in source {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Extract the declaration facts of one source buffer.
pub fn analyze_file(source: &[u8]) -> Result<FileFacts, FormatError> {
    analyze_file_with_path(None, source)
}

/// Extract declaration facts for a source whose project path is known.
/// Relative INCLUDE expansion is path-sensitive, so callers formatting a
/// concrete project member should preserve this identity.
pub fn analyze_file_at(path: &Path, source: &[u8]) -> Result<FileFacts, FormatError> {
    analyze_file_with_path(Some(path), source)
}

fn analyze_file_with_path(path: Option<&Path>, source: &[u8]) -> Result<FileFacts, FormatError> {
    let document = Document::from_bytes(source);
    let analysis = document.analyze()?;
    let scopes = ScopeTree::build(&analysis);
    let mut facts = extract(&analysis, &scopes);
    facts.source_id = fingerprint(source);
    facts.source_path = path.map(normalize_path);
    Ok(facts)
}

/// Fold a batch of already-analyzed project sources into one context while
/// sharing INCLUDE fragment analysis across the whole batch.
///
/// Supplied project sources can satisfy INCLUDE lookups for each other, but
/// every supplied source is still absorbed independently as a project root.
/// Other candidates are loaded at most once, including nested INCLUDEs and
/// failed lookups, and their cached facts are reused by every including source.
pub(super) fn absorb_analyzed<'a, I>(context: &mut ProjectContext, sources: I)
where
    I: IntoIterator<Item = (&'a Path, &'a FileFacts)>,
{
    absorb_analyzed_with(context, sources, &mut |candidate| {
        fs::read(candidate)
            .ok()
            .and_then(|source| analyze_file_at(candidate, &source).ok())
    });
}

fn absorb_analyzed_with<'a, I>(
    context: &mut ProjectContext,
    sources: I,
    loader: &mut impl FnMut(&Path) -> Option<FileFacts>,
) where
    I: IntoIterator<Item = (&'a Path, &'a FileFacts)>,
{
    absorb_analyzed_with_resources(
        context,
        sources,
        std::iter::empty::<(&Path, &FileFacts)>(),
        loader,
    );
}

fn absorb_analyzed_with_resources<'a, 'b, I, J>(
    context: &mut ProjectContext,
    sources: I,
    include_resources: J,
    loader: &mut impl FnMut(&Path) -> Option<FileFacts>,
) where
    I: IntoIterator<Item = (&'a Path, &'a FileFacts)>,
    J: IntoIterator<Item = (&'b Path, &'b FileFacts)>,
{
    let analyzed = sources.into_iter().collect::<Vec<_>>();
    let resources = include_resources.into_iter().collect::<Vec<_>>();
    let mut lookup = HashMap::with_capacity(analyzed.len() + resources.len());

    // Include resources are lookup-only. Project roots are inserted second so
    // an explicitly supplied root wins if the caller provides the same path in
    // both collections.
    for &(path, facts) in &resources {
        lookup.insert(normalize_path(path), facts.clone());
    }
    for &(path, facts) in &analyzed {
        lookup.insert(normalize_path(path), facts.clone());
    }

    // One fragment is typically included by many sources, and a nested include
    // tree multiplies that again. Analyze each fragment once and hand out
    // copies. `expand_includes_with` normalizes before it asks, so the candidate
    // path is already the cache key.
    let mut fragments: HashMap<PathBuf, Option<FileFacts>> = HashMap::new();
    for &(path, facts) in &analyzed {
        let expanded = expand_includes_with(path, facts, &mut |candidate| {
            fragments
                .entry(candidate.to_path_buf())
                .or_insert_with(|| lookup.get(candidate).cloned().or_else(|| loader(candidate)))
                .clone()
        });
        context.absorb_expanded(path, facts, expanded);
    }
}

fn analyze_inputs<'a, I>(inputs: I) -> Result<Vec<(PathBuf, FileFacts)>, FormatError>
where
    I: IntoIterator<Item = (&'a Path, &'a [u8])>,
{
    inputs
        .into_iter()
        .map(|(path, source)| Ok((path.to_path_buf(), analyze_file_at(path, source)?)))
        .collect()
}

/// Build a project context from every supplied project source.
///
/// Every item passed here is a project root and is absorbed independently.
/// Relative INCLUDEs may resolve another supplied root or fall back to the
/// filesystem, but include-only in-memory buffers should instead be passed via
/// [`analyze_project_with_includes`].
pub fn analyze_project<'a, I>(sources: I) -> Result<ProjectContext, FormatError>
where
    I: IntoIterator<Item = (&'a Path, &'a [u8])>,
{
    let analyzed = analyze_inputs(sources)?;
    let mut context = ProjectContext::empty();
    absorb_analyzed(
        &mut context,
        analyzed.iter().map(|(path, facts)| (path.as_path(), facts)),
    );
    Ok(context)
}

/// Build a project context from project roots plus in-memory INCLUDE resources.
///
/// `sources` are absorbed independently as project sources. `include_resources`
/// participate only in path-based INCLUDE lookup (including nested INCLUDEs)
/// and never become project-wide sources in their own right. Missing resources
/// still fall back to the filesystem.
pub fn analyze_project_with_includes<'a, 'b, I, J>(
    sources: I,
    include_resources: J,
) -> Result<ProjectContext, FormatError>
where
    I: IntoIterator<Item = (&'a Path, &'a [u8])>,
    J: IntoIterator<Item = (&'b Path, &'b [u8])>,
{
    let analyzed = analyze_inputs(sources)?;
    let resources = analyze_inputs(include_resources)?;
    let mut context = ProjectContext::empty();
    absorb_analyzed_with_resources(
        &mut context,
        analyzed.iter().map(|(path, facts)| (path.as_path(), facts)),
        resources
            .iter()
            .map(|(path, facts)| (path.as_path(), facts)),
        &mut |candidate| {
            fs::read(candidate)
                .ok()
                .and_then(|source| analyze_file_at(candidate, &source).ok())
        },
    );
    Ok(context)
}

#[cfg(test)]
mod tests {
    use super::{
        absorb_analyzed_with, analyze_file_at, analyze_project_with_includes, ProjectContext,
    };
    use crate::analysis::names::NameSpace;
    use std::path::Path;

    #[test]
    fn bulk_absorption_caches_shared_include_analysis() {
        let first = analyze_file_at(
            Path::new("first.f90"),
            b"program First\ninclude 'shared.inc'\nend program First\n",
        )
        .unwrap();
        let second = analyze_file_at(
            Path::new("second.f90"),
            b"program Second\ninclude 'shared.inc'\nend program Second\n",
        )
        .unwrap();
        let shared = analyze_file_at(Path::new("shared.inc"), b"integer :: SharedCase\n").unwrap();
        let mut loads = 0usize;
        let mut context = ProjectContext::empty();

        absorb_analyzed_with(
            &mut context,
            [
                (Path::new("first.f90"), &first),
                (Path::new("second.f90"), &second),
            ],
            &mut |candidate| {
                loads += 1;
                assert_eq!(candidate, Path::new("shared.inc"));
                Some(shared.clone())
            },
        );

        assert_eq!(loads, 1);
        assert_eq!(context.sources.len(), 2);
    }

    #[test]
    fn in_memory_include_resources_are_not_project_sources() {
        let root = b"program p\nend program p\n";
        let unused = b"module LeakedResource\nend module LeakedResource\n";
        let project = analyze_project_with_includes(
            [(Path::new("p.f90"), root.as_slice())],
            [(Path::new("unused.inc"), unused.as_slice())],
        )
        .unwrap();

        assert_eq!(project.sources, vec![Path::new("p.f90")]);
        let local = analyze_file_at(Path::new("p.f90"), root).unwrap();
        assert_eq!(
            project
                .resolver(&local)
                .spelling(NameSpace::Module, b"leakedresource"),
            None
        );
    }

    #[test]
    fn in_memory_include_resources_still_expand_textually() {
        let host = b"module host\ninclude 'defs.inc'\nend module host\n";
        let target = b"program p\nuse host\nprint *, includedcase\nend program p\n";
        let defs = b"integer :: IncludedCase\n";
        let project = analyze_project_with_includes(
            [
                (Path::new("host.f90"), host.as_slice()),
                (Path::new("target.f90"), target.as_slice()),
            ],
            [(Path::new("defs.inc"), defs.as_slice())],
        )
        .unwrap();

        assert_eq!(
            project.sources,
            vec![Path::new("host.f90"), Path::new("target.f90")]
        );
        let local = analyze_file_at(Path::new("target.f90"), target).unwrap();
        assert_eq!(
            project.visible_symbol_spelling(&local, 2, b"includedcase"),
            Some(b"IncludedCase".to_vec())
        );
    }
}
