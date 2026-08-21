//! Unified diff for `--diff` and `--check`.
//!
//! A Myers diff over whole physical lines, with the trace bounded so a pair of
//! large and wholly dissimilar files cannot make the CLI allocate without
//! limit. Nothing here knows about Fortran.

use super::sources::display_path;
use std::path::Path;

pub(super) fn split_lines(bytes: &[u8]) -> Vec<&[u8]> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            lines.push(&bytes[start..=index]);
            start = index + 1;
        }
    }
    if start < bytes.len() {
        lines.push(&bytes[start..]);
    }
    lines
}

const DIFF_CONTEXT_LINES: usize = 3;
const MAX_DIFF_TRACE_CELLS: usize = 4_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffKind {
    Context,
    Delete,
    Insert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DiffLine {
    kind: DiffKind,
    old_index: usize,
    new_index: usize,
}

fn diagonal_index(diagonal: isize, distance: usize) -> usize {
    ((diagonal + distance as isize) / 2) as usize
}

fn coarse_diff(old: &[&[u8]], new: &[&[u8]]) -> Vec<DiffLine> {
    let common_limit = old.len().min(new.len());
    let mut prefix = 0usize;
    while prefix < common_limit && old[prefix] == new[prefix] {
        prefix += 1;
    }

    let mut suffix = 0usize;
    while suffix < common_limit - prefix
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_change_end = old.len() - suffix;
    let new_change_end = new.len() - suffix;
    let mut lines = Vec::with_capacity(old.len() + new.len());

    for index in 0..prefix {
        lines.push(DiffLine {
            kind: DiffKind::Context,
            old_index: index,
            new_index: index,
        });
    }
    for old_index in prefix..old_change_end {
        lines.push(DiffLine {
            kind: DiffKind::Delete,
            old_index,
            new_index: prefix,
        });
    }
    for new_index in prefix..new_change_end {
        lines.push(DiffLine {
            kind: DiffKind::Insert,
            old_index: old_change_end,
            new_index,
        });
    }
    for offset in 0..suffix {
        lines.push(DiffLine {
            kind: DiffKind::Context,
            old_index: old_change_end + offset,
            new_index: new_change_end + offset,
        });
    }
    lines
}

fn backtrack_diff(trace: &[Vec<isize>], old_len: usize, new_len: usize) -> Vec<DiffLine> {
    let mut old_cursor = old_len as isize;
    let mut new_cursor = new_len as isize;
    let mut kinds = Vec::with_capacity(old_len + new_len);

    for distance in (1..trace.len()).rev() {
        let diagonal = old_cursor - new_cursor;
        let previous = &trace[distance - 1];
        let previous_diagonal = if diagonal == -(distance as isize) {
            diagonal + 1
        } else if diagonal == distance as isize {
            diagonal - 1
        } else {
            let down = previous[diagonal_index(diagonal + 1, distance - 1)];
            let right = previous[diagonal_index(diagonal - 1, distance - 1)] + 1;
            if right > down {
                diagonal - 1
            } else {
                diagonal + 1
            }
        };
        let previous_old = previous[diagonal_index(previous_diagonal, distance - 1)];
        let previous_new = previous_old - previous_diagonal;

        while old_cursor > previous_old && new_cursor > previous_new {
            old_cursor -= 1;
            new_cursor -= 1;
            kinds.push(DiffKind::Context);
        }
        if old_cursor == previous_old {
            new_cursor -= 1;
            kinds.push(DiffKind::Insert);
        } else {
            old_cursor -= 1;
            kinds.push(DiffKind::Delete);
        }
    }

    while old_cursor > 0 && new_cursor > 0 {
        old_cursor -= 1;
        new_cursor -= 1;
        kinds.push(DiffKind::Context);
    }
    while old_cursor > 0 {
        old_cursor -= 1;
        kinds.push(DiffKind::Delete);
    }
    while new_cursor > 0 {
        new_cursor -= 1;
        kinds.push(DiffKind::Insert);
    }
    kinds.reverse();

    let mut old_index = 0usize;
    let mut new_index = 0usize;
    kinds
        .into_iter()
        .map(|kind| {
            let line = DiffLine {
                kind,
                old_index,
                new_index,
            };
            if kind != DiffKind::Insert {
                old_index += 1;
            }
            if kind != DiffKind::Delete {
                new_index += 1;
            }
            line
        })
        .collect()
}

/// Return a shortest line edit script with stable indices into both inputs.
///
/// Myers' frontier is compact for formatter output, where most lines are equal.
/// The retained trace is explicitly bounded so a near-total rewrite falls back
/// to one coarse changed region instead of consuming memory proportional to the
/// square of the edit distance.
fn diff_lines(old: &[&[u8]], new: &[&[u8]]) -> Vec<DiffLine> {
    let old_len = old.len() as isize;
    let new_len = new.len() as isize;
    let max_distance = old.len() + new.len();
    let mut trace = Vec::<Vec<isize>>::new();
    let mut trace_cells = 0usize;

    for distance in 0..=max_distance {
        trace_cells += distance + 1;
        if trace_cells > MAX_DIFF_TRACE_CELLS {
            return coarse_diff(old, new);
        }

        let mut frontier = vec![0isize; distance + 1];
        for diagonal in (-(distance as isize)..=distance as isize).step_by(2) {
            let mut old_cursor = if distance == 0 {
                0
            } else {
                let previous = &trace[distance - 1];
                if diagonal == -(distance as isize) {
                    previous[diagonal_index(diagonal + 1, distance - 1)]
                } else if diagonal == distance as isize {
                    previous[diagonal_index(diagonal - 1, distance - 1)] + 1
                } else {
                    let down = previous[diagonal_index(diagonal + 1, distance - 1)];
                    let right = previous[diagonal_index(diagonal - 1, distance - 1)] + 1;
                    if right > down {
                        right
                    } else {
                        down
                    }
                }
            };
            let mut new_cursor = old_cursor - diagonal;
            while old_cursor < old_len
                && new_cursor < new_len
                && old[old_cursor as usize] == new[new_cursor as usize]
            {
                old_cursor += 1;
                new_cursor += 1;
            }
            frontier[diagonal_index(diagonal, distance)] = old_cursor;
            if old_cursor == old_len && new_cursor == new_len {
                trace.push(frontier);
                return backtrack_diff(&trace, old.len(), new.len());
            }
        }
        trace.push(frontier);
    }
    unreachable!("edit distance cannot exceed the combined input length")
}

fn diff_hunks(lines: &[DiffLine]) -> Vec<std::ops::Range<usize>> {
    let changes: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (line.kind != DiffKind::Context).then_some(index))
        .collect();
    let Some((&first, rest)) = changes.split_first() else {
        return Vec::new();
    };

    let mut hunks = Vec::new();
    let mut start = first.saturating_sub(DIFF_CONTEXT_LINES);
    let mut last = first;
    for &change in rest {
        if change - last - 1 > 2 * DIFF_CONTEXT_LINES {
            hunks.push(start..(last + DIFF_CONTEXT_LINES + 1).min(lines.len()));
            start = change.saturating_sub(DIFF_CONTEXT_LINES);
        }
        last = change;
    }
    hunks.push(start..(last + DIFF_CONTEXT_LINES + 1).min(lines.len()));
    hunks
}

fn append_diff_line(output: &mut Vec<u8>, marker: u8, line: &[u8]) {
    output.push(marker);
    output.extend_from_slice(line);
    if !line.ends_with(b"\n") {
        output.extend_from_slice(b"\n\\ No newline at end of file\n");
    }
}

fn hunk_line_number(start: usize, count: usize) -> usize {
    if count == 0 {
        start
    } else {
        start + 1
    }
}

pub(super) fn unified_diff(path: &Path, old: &[u8], new: &[u8], root: Option<&Path>) -> Vec<u8> {
    if old == new {
        return Vec::new();
    }

    let relative = display_path(path, root).display().to_string();
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let diff = diff_lines(&old_lines, &new_lines);
    let mut output = Vec::new();
    output.extend_from_slice(format!("--- a/{relative}\n+++ b/{relative}\n").as_bytes());

    for hunk in diff_hunks(&diff) {
        let lines = &diff[hunk];
        let old_start = lines[0].old_index;
        let new_start = lines[0].new_index;
        let old_count = lines
            .iter()
            .filter(|line| line.kind != DiffKind::Insert)
            .count();
        let new_count = lines
            .iter()
            .filter(|line| line.kind != DiffKind::Delete)
            .count();
        output.extend_from_slice(
            format!(
                "@@ -{},{} +{},{} @@\n",
                hunk_line_number(old_start, old_count),
                old_count,
                hunk_line_number(new_start, new_count),
                new_count,
            )
            .as_bytes(),
        );

        for line in lines {
            match line.kind {
                DiffKind::Context => append_diff_line(&mut output, b' ', old_lines[line.old_index]),
                DiffKind::Delete => append_diff_line(&mut output, b'-', old_lines[line.old_index]),
                DiffKind::Insert => append_diff_line(&mut output, b'+', new_lines[line.new_index]),
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::unified_diff;
    use std::path::Path;

    #[test]
    fn unified_diff_marks_missing_final_newlines() {
        let diff = unified_diff(Path::new("source.f90"), b"a\nold", b"a\nnew", None);
        assert_eq!(
            diff,
            b"--- a/source.f90\n+++ b/source.f90\n@@ -1,2 +1,2 @@\n a\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n"
        );
    }

    #[test]
    fn unified_diff_reports_a_newline_only_change() {
        let diff = unified_diff(Path::new("source.f90"), b"same", b"same\n", None);
        assert_eq!(
            diff,
            b"--- a/source.f90\n+++ b/source.f90\n@@ -1,1 +1,1 @@\n-same\n\\ No newline at end of file\n+same\n"
        );
    }

    #[test]
    fn unified_diff_trims_unchanged_file_ends() {
        let old = b"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n";
        let new = b"1\n2\n3\n4\n5\n6\n7\nchanged\n9\n10\n11\n12\n13\n14\n15\n";
        let diff = unified_diff(Path::new("source.f90"), old, new, None);
        assert_eq!(
            diff,
            b"--- a/source.f90\n+++ b/source.f90\n@@ -5,7 +5,7 @@\n 5\n 6\n 7\n-8\n+changed\n 9\n 10\n 11\n"
        );
    }

    #[test]
    fn unified_diff_splits_distant_changes_into_hunks() {
        let old = b"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n";
        let new = b"1\nTWO\n3\n4\n5\n6\n7\n8\n9\n10\n11\nTWELVE\n13\n14\n15\n";
        let diff = unified_diff(Path::new("source.f90"), old, new, None);
        assert_eq!(
            diff,
            b"--- a/source.f90\n+++ b/source.f90\n@@ -1,5 +1,5 @@\n 1\n-2\n+TWO\n 3\n 4\n 5\n@@ -9,7 +9,7 @@\n 9\n 10\n 11\n-12\n+TWELVE\n 13\n 14\n 15\n"
        );
    }

    #[test]
    fn unified_diff_tracks_later_hunk_lines_after_an_insertion() {
        let old = b"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n";
        let new = b"1\n2\ninserted\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\nFOURTEEN\n15\n";
        let diff = unified_diff(Path::new("source.f90"), old, new, None);
        assert_eq!(
            diff,
            b"--- a/source.f90\n+++ b/source.f90\n@@ -1,5 +1,6 @@\n 1\n 2\n+inserted\n 3\n 4\n 5\n@@ -11,5 +12,5 @@\n 11\n 12\n 13\n-14\n+FOURTEEN\n 15\n"
        );
    }
}
