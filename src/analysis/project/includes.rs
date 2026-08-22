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
/// A candidate that is itself one of the supplied sources is cloned from the
/// precomputed facts. Other candidates are loaded at most once, including
/// nested INCLUDEs and failed lookups, and their cached facts are then reused
/// by every including source.
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
    let analyzed = sources.into_iter().collect::<Vec<_>>();
    let mut lookup = HashMap::with_capacity(analyzed.len());
    for (index, &(path, _)) in analyzed.iter().enumerate() {
        lookup.insert(normalize_path(path), index);
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
                .or_insert_with(|| {
                    lookup
                        .get(candidate)
                        .map(|index| (*analyzed[*index].1).clone())
                        .or_else(|| loader(candidate))
                })
                .clone()
        });
        context.absorb_expanded(path, facts, expanded);
    }
}

/// Build a project context from every source in the project.
///
/// The source list is analyzed once up front. INCLUDE resolution first uses
/// that in-memory set (so `.inc` fragments can be supplied without touching
/// the filesystem), then falls back to the filesystem for ordinary CLI use.
pub fn analyze_project<'a, I>(sources: I) -> Result<ProjectContext, FormatError>
where
    I: IntoIterator<Item = (&'a Path, &'a [u8])>,
{
    let inputs = sources
        .into_iter()
        .map(|(path, source)| (path.to_path_buf(), source))
        .collect::<Vec<_>>();
    let mut analyzed = Vec::with_capacity(inputs.len());
    for (path, source) in &inputs {
        analyzed.push((path.clone(), analyze_file_at(path, source)?));
    }

    let mut context = ProjectContext::empty();
    absorb_analyzed(
        &mut context,
        analyzed.iter().map(|(path, facts)| (path.as_path(), facts)),
    );
    Ok(context)
}

#[cfg(test)]
mod tests {
    use super::{absorb_analyzed_with, analyze_file_at, ProjectContext};
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
}
