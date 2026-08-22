//! Turning selected sources into the analysis every target is formatted
//! against, and running the formatter over them.
//!
//! One project context serves the whole invocation, so the declarations a file
//! borrows from its neighbours are read once rather than once per target.

use super::{sources::Source, WorkflowError};
use crate::{
    analysis::{analyze_file_at, project::absorb_analyzed, FileFacts, ProjectContext},
    format_source, FormatResult,
};
use std::{
    num::NonZeroUsize,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};

pub(super) fn analyze_sources(
    sources: &[Source],
    indices: &[usize],
) -> Result<Vec<Option<FileFacts>>, WorkflowError> {
    let mut facts = (0..sources.len()).map(|_| None).collect::<Vec<_>>();
    for &index in indices {
        facts[index] = Some(analyze_file_at(
            &sources[index].path,
            &sources[index].bytes,
        )?);
    }
    Ok(facts)
}

pub(super) fn project_context(
    sources: &[Source],
    indices: &[usize],
    facts: &[Option<FileFacts>],
    stdin_source: Option<(&Path, &FileFacts)>,
    config: &crate::config::FormatConfig,
) -> ProjectContext {
    // Source facts are extracted once for the invocation, then reused both to
    // build project tables and as target-local precedence data during format.
    // Bulk absorption shares one INCLUDE fragment cache across every source.
    let mut context = ProjectContext::empty();
    absorb_analyzed(
        &mut context,
        indices.iter().map(|&index| {
            (
                sources[index].path.as_path(),
                facts[index]
                    .as_ref()
                    .expect("every project source must have precomputed facts"),
            )
        }),
    );
    // A file-valued --project-context makes stdin the current version of that
    // tracked source. Its already-extracted facts replace the stale disk copy.
    if let Some((path, local)) = stdin_source {
        context.absorb(path, local);
    }
    context.define(&config.defines);
    context.enable_target_local_component_resolution();
    context
}

pub(super) fn isolated_context(config: &crate::config::FormatConfig) -> ProjectContext {
    let mut context = ProjectContext::empty();
    context.define(&config.defines);
    context
}

pub(super) fn format_one(
    source: &Source,
    local: Option<&FileFacts>,
    context: &ProjectContext,
    config: &crate::config::FormatConfig,
) -> Result<FormatResult, WorkflowError> {
    let result = if !config.mode.normalizes() {
        format_source(&source.bytes, config)?
    } else {
        crate::format::full::format_with_context_and_local(
            &source.bytes,
            context,
            local.expect("every full-mode target must have precomputed facts"),
            config,
        )?
    };
    Ok(result)
}

/// One formatted target: its metadata, and its bytes only if they differ from
/// what was read.
type FormattedTarget = (crate::FormatMeta, Option<Vec<u8>>);

/// Format every target, in parallel, and return one entry per target in target
/// order.
///
/// Formatting is pure once the single project-analysis pass has run, so the
/// targets are independent. Two things bound the cost of that independence:
///
/// * the worker count is `available_parallelism()`, not one thread per file —
///   `--all` over a large repository would otherwise ask the OS for thousands
///   of threads at once;
/// * a target whose output equals its input contributes only its metadata, so
///   an already-formatted tree does not hold a second copy of itself in memory.
///
/// Every target is formatted before the caller writes anything, which is what
/// keeps a failure part-way through from leaving a half-rewritten tree.
pub(super) fn format_targets(
    sources: &[Source],
    target_indices: &[usize],
    facts: &[Option<FileFacts>],
    context: &ProjectContext,
    config: &crate::config::FormatConfig,
) -> Result<Vec<FormattedTarget>, WorkflowError> {
    let workers = std::thread::available_parallelism()
        .map(NonZeroUsize::get)
        .unwrap_or(1)
        .min(target_indices.len().max(1));
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();
    let mut slots: Vec<Option<Result<FormattedTarget, WorkflowError>>> =
        (0..target_indices.len()).map(|_| None).collect();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let sender = sender.clone();
            let next = &next;
            scope.spawn(move || loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(&source_index) = target_indices.get(index) else {
                    return;
                };
                let target = &sources[source_index];
                let outcome = format_one(target, facts[source_index].as_ref(), context, config)
                    .map(|result| {
                        let changed = result.bytes != target.bytes;
                        (result.meta, changed.then_some(result.bytes))
                    });
                if sender.send((index, outcome)).is_err() {
                    return;
                }
            });
        }
        // The workers hold the only remaining senders, so the receiver ends
        // exactly when the last one finishes.
        drop(sender);
        for (index, outcome) in receiver {
            slots[index] = Some(outcome);
        }
    });
    slots
        .into_iter()
        .map(|slot| slot.expect("format worker panicked"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::project_context;
    use crate::{analysis::names::NameSpace, config::FormatConfig};
    use std::path::Path;

    #[test]
    fn stdin_replacement_is_present_in_the_project_tables() {
        let replacement = b"module CurrentName\nend module CurrentName\n";
        let replacement_facts = crate::analysis::analyze_file(replacement).unwrap();
        let context = project_context(
            &[],
            &[],
            &[],
            Some((Path::new("target.f90"), &replacement_facts)),
            &FormatConfig::default(),
        );
        let local = crate::analysis::analyze_file(b"program p\nend program p\n").unwrap();
        assert_eq!(
            context
                .resolver(&local)
                .spelling(NameSpace::Module, b"currentname"),
            Some(b"CurrentName".as_slice())
        );
        assert_eq!(context.sources, vec![Path::new("target.f90")]);
    }
}
