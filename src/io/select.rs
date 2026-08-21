//! Which files this invocation reads, and where it is rooted.
//!
//! Three questions in order, each one a type: [`Scope`] settles the repository
//! root and the directories `--project-context` and `--all` bound the run to,
//! [`Selection`] turns that plus the command line into the two path lists —
//! what to format and what to read for context — and [`Loaded`] reads each of
//! those paths exactly once, so the bytes a target is formatted from are the
//! same bytes the project tables were built from.

use super::{
    exclude::ExcludeMatcher,
    sources::{
        context_sources, deduplicate, read_source, repository_root, resolve_context_paths,
        resolve_input, tracked_sources, tracked_sources_without_submodules, validate_extension,
        Source,
    },
    WorkflowError,
};
use crate::{cli::Invocation, source::SourceForm};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

/// Where the invocation is rooted, and the directories that bound what it may
/// read.
pub(super) struct Scope {
    pub(super) root: Option<PathBuf>,
    /// The directory a `--project-context` names and, when it named a file,
    /// the tracked path whose on-disk bytes stdin replaces.
    pub(super) project: Option<(PathBuf, Option<PathBuf>)>,
    /// The directory `--all`/`--all-files` was pointed at, if any.
    all: Option<PathBuf>,
    pub(super) context_paths: Vec<PathBuf>,
}

impl Scope {
    pub(super) fn resolve(
        invocation: &Invocation,
        cwd: &Path,
        all_selection: bool,
    ) -> Result<Self, WorkflowError> {
        // A directory-valued project context describes an anonymous stdin
        // buffer.  A file-valued context additionally identifies the tracked
        // file whose in-memory contents stdin replaces, so its stale on-disk
        // bytes must not contribute to project analysis.
        let project = invocation
            .project_context
            .as_deref()
            .map(|path| {
                let candidate = resolve_input(path, None);
                let canonical = fs::canonicalize(&candidate).map_err(|error| {
                    WorkflowError::Usage(format!(
                        "--project-context path does not exist: {} ({error})",
                        candidate.display()
                    ))
                })?;
                let metadata = fs::metadata(&canonical)?;
                if metadata.is_dir() {
                    return Ok((canonical, None));
                }
                if !metadata.is_file() {
                    return Err(WorkflowError::Usage(format!(
                        "--project-context requires a directory or regular source file: {}",
                        candidate.display()
                    )));
                }
                validate_extension(&candidate).map_err(WorkflowError::Usage)?;
                let parent = candidate
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                let directory = fs::canonicalize(parent)?;
                let stdin_path = directory.join(
                    candidate
                        .file_name()
                        .expect("a regular file must have a file name"),
                );
                Ok((directory, Some(stdin_path)))
            })
            .transpose()?;
        let all = if all_selection {
            invocation
                .paths
                .first()
                .map(|path| {
                    let candidate = resolve_input(path, None);
                    let canonical = fs::canonicalize(&candidate).map_err(|error| {
                        WorkflowError::Usage(format!(
                            "--all/--all-files directory does not exist: {} ({error})",
                            candidate.display()
                        ))
                    })?;
                    if !fs::metadata(&canonical)?.is_dir() {
                        return Err(WorkflowError::Usage(format!(
                            "--all/--all-files requires a directory: {}",
                            candidate.display()
                        )));
                    }
                    Ok(canonical)
                })
                .transpose()?
        } else {
            None
        };
        let root = if let Some(scope) = all.as_deref() {
            repository_root(scope)?
        } else if let Some((scope, _)) = project.as_ref() {
            repository_root(scope)?
        } else {
            repository_root(cwd)?
        };
        if invocation.project_context.is_some() && root.is_none() {
            return Err(WorkflowError::Usage(
                "--project-context requires a valid Git checkout".into(),
            ));
        }
        let context_paths = if invocation.context_paths.is_empty() {
            Vec::new()
        } else {
            resolve_context_paths(&invocation.context_paths, root.as_deref(), cwd)?
        };
        Ok(Self {
            root,
            project,
            all,
            context_paths,
        })
    }

    /// The tracked path a file-valued `--project-context` named, if it named
    /// one.
    pub(super) fn stdin_path(&self) -> Option<&Path> {
        self.project.as_ref().and_then(|(_, path)| path.as_deref())
    }

    pub(super) fn exclusion_root<'a>(&'a self, cwd: &'a Path) -> &'a Path {
        self.root.as_deref().unwrap_or(cwd)
    }
}

/// The paths this invocation will format, and the paths whose declarations it
/// reads in order to do so.
pub(super) struct Selection {
    pub(super) targets: Vec<PathBuf>,
    pub(super) project: Vec<PathBuf>,
}

impl Selection {
    /// Every path that has to be read: the targets and the project sources,
    /// each once.
    pub(super) fn all(&self) -> Vec<PathBuf> {
        deduplicate(
            self.targets
                .iter()
                .cloned()
                .chain(self.project.iter().cloned())
                .collect::<Vec<_>>(),
        )
    }

    /// Drop what turned out not to exist, and keep only free-form sources in
    /// the project tables — a fixed-form file is skipped, not analyzed.
    pub(super) fn retain_loaded(&mut self, loaded: &Loaded) {
        self.targets.retain(|path| loaded.index.contains_key(path));
        self.project.retain(|path| {
            loaded
                .get(path)
                .is_some_and(|source| source.form == SourceForm::Free)
        });
    }

    pub(super) fn free_form_indices(&self, paths: &[PathBuf], loaded: &Loaded) -> Vec<usize> {
        paths
            .iter()
            .filter_map(|path| {
                let index = loaded.index[path];
                (loaded.sources[index].form == SourceForm::Free).then_some(index)
            })
            .collect()
    }
}

pub(super) fn select_paths(
    invocation: &Invocation,
    scope: &Scope,
    cwd: &Path,
    stdin_mode: bool,
    all_selection: bool,
) -> Result<Selection, WorkflowError> {
    let exclude_matcher = ExcludeMatcher::new(&invocation.exclude_patterns());
    let tracked_source_reader = if invocation.no_submodules {
        tracked_sources_without_submodules
    } else {
        tracked_sources
    };
    let tracked = if all_selection
        || invocation.project_context.is_some()
        || (!invocation.isolated && scope.root.is_some())
    {
        scope
            .root
            .as_deref()
            .map(tracked_source_reader)
            .transpose()?
    } else {
        None
    };
    let exclusion_root = scope.exclusion_root(cwd);
    let context_tracked = context_sources(
        tracked.as_ref(),
        &scope.context_paths,
        &exclude_matcher,
        exclusion_root,
    )?;
    let tracked = tracked.map(|paths| {
        paths
            .into_iter()
            .filter(|path| !exclude_matcher.is_excluded(exclusion_root, path))
            .collect::<Vec<_>>()
    });
    let targets = if invocation.all {
        let tracked = tracked
            .as_ref()
            .ok_or_else(|| WorkflowError::Usage("--all requires a valid Git checkout".into()))?;
        match scope.all.as_deref() {
            Some(directory) => tracked
                .iter()
                .filter(|path| path.starts_with(directory))
                .cloned()
                .collect(),
            None => tracked.clone(),
        }
    } else if invocation.all_files {
        // `--all-files` deliberately re-reads the tracked list without
        // submodules rather than filtering the one above.
        let root = scope.root.as_deref().ok_or_else(|| {
            WorkflowError::Usage("--all-files requires a valid Git checkout".into())
        })?;
        let tracked = tracked_sources_without_submodules(root)?
            .into_iter()
            .filter(|path| !exclude_matcher.is_excluded(root, path));
        match scope.all.as_deref() {
            Some(directory) => tracked.filter(|path| path.starts_with(directory)).collect(),
            None => tracked.collect(),
        }
    } else if stdin_mode {
        Vec::new()
    } else {
        deduplicate(
            invocation
                .paths
                .iter()
                .map(|path| resolve_input(path, scope.root.as_deref()))
                .collect::<Vec<_>>(),
        )
    };
    let project = if invocation.isolated {
        // Isolated means no project tables at all. The target is still read
        // and formatted, but its declarations remain local to the formatter,
        // exactly as they are for stdin.
        Vec::new()
    } else if scope.project.is_some() {
        // A `--project-context` names the whole context itself, so the targets
        // are not folded in.  When it named a file, stdin *is* that file's
        // current contents and the stale copy on disk must not be read.
        let stdin_path = scope.stdin_path();
        context_tracked
            .as_ref()
            .expect("project-context requires tracked sources")
            .iter()
            .filter(|path| Some(path.as_path()) != stdin_path)
            .cloned()
            .collect()
    } else if let Some(context_tracked) = context_tracked.as_ref() {
        if scope.context_paths.is_empty() {
            deduplicate(
                context_tracked
                    .iter()
                    .cloned()
                    .chain(targets.iter().cloned())
                    .collect::<Vec<_>>(),
            )
        } else {
            context_tracked.clone()
        }
    } else {
        targets.clone()
    };
    Ok(Selection { targets, project })
}

/// Every selected source, read once. The same in-memory bytes serve both the
/// target formatter and the single project-analysis pass.
pub(super) struct Loaded {
    pub(super) sources: Vec<Source>,
    pub(super) index: HashMap<PathBuf, usize>,
}

impl Loaded {
    pub(super) fn read(
        selection: &Selection,
        invocation: &Invocation,
        implicit_targets: bool,
    ) -> Result<Self, WorkflowError> {
        let paths = selection.all();
        // A path the user named must exist; one discovered by `--all` or
        // pulled in as project context may have vanished since `git ls-files`.
        let explicit: HashSet<&Path> = if implicit_targets {
            HashSet::new()
        } else {
            selection.targets.iter().map(PathBuf::as_path).collect()
        };
        let mut sources = Vec::with_capacity(paths.len());
        let mut index = HashMap::with_capacity(paths.len());
        for path in &paths {
            match read_source(path, invocation.force_free_input)? {
                Some(source) => {
                    index.insert(path.clone(), sources.len());
                    sources.push(source);
                }
                None if explicit.contains(path.as_path()) => {
                    return Err(WorkflowError::Usage(format!(
                        "Fortran source file does not exist: {}",
                        path.display()
                    )));
                }
                None => {}
            }
        }
        Ok(Self { sources, index })
    }

    pub(super) fn get(&self, path: &Path) -> Option<&Source> {
        self.index.get(path).map(|&index| &self.sources[index])
    }
}

/// The union of two index lists, in order, without repeats: a path can be both
/// a target and a project source, and must be analyzed once.
pub(super) fn deduplicate_indices(first: &[usize], second: &[usize]) -> Vec<usize> {
    let mut seen = HashSet::new();
    first
        .iter()
        .chain(second)
        .copied()
        .filter(|index| seen.insert(*index))
        .collect()
}
