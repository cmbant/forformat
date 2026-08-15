//! Matching for repository-relative source exclusions.
//!
//! This deliberately stays smaller than a general glob implementation. The
//! command-line contract is limited to `*`, `**`, `?`, and a trailing `/`
//! directory-prefix marker.

use std::path::Path;

#[derive(Debug, Clone)]
struct Pattern {
    body: Vec<char>,
    anchored: bool,
    directory: bool,
}

impl Pattern {
    fn new(pattern: &str) -> Self {
        let anchored = pattern.starts_with('/');
        let mut body = pattern
            .strip_prefix('/')
            .unwrap_or(pattern)
            .chars()
            .collect::<Vec<_>>();
        let directory = body.last() == Some(&'/');
        if directory {
            body.pop();
        }
        Self {
            body,
            anchored,
            directory,
        }
    }

    fn matches(&self, path: &str) -> bool {
        if self.directory {
            // Only the parts of `path` that are followed by a separator name a
            // directory.  The final segment is the file itself, so testing it
            // here would let `generated-*/` exclude `generated-cache.f90`.
            return path.char_indices().any(|(index, character)| {
                character == '/' && self.matches_candidate(&path[..index])
            });
        }

        if self.anchored {
            return glob_matches(&self.body, path);
        }

        let mut start = 0;
        loop {
            if glob_matches(&self.body, &path[start..]) {
                return true;
            }
            let Some(next) = path[start..].find('/') else {
                return false;
            };
            start += next + 1;
        }
    }

    fn matches_candidate(&self, candidate: &str) -> bool {
        if self.anchored {
            glob_matches(&self.body, candidate)
        } else {
            let mut start = 0;
            loop {
                if glob_matches(&self.body, &candidate[start..]) {
                    return true;
                }
                let Some(next) = candidate[start..].find('/') else {
                    return false;
                };
                start += next + 1;
            }
        }
    }
}

/// The configured repository-relative exclusion patterns.
#[derive(Debug, Clone, Default)]
pub(crate) struct ExcludeMatcher {
    patterns: Vec<Pattern>,
}

impl ExcludeMatcher {
    pub(crate) fn new(patterns: &[String]) -> Self {
        Self {
            patterns: patterns
                .iter()
                .map(|pattern| Pattern::new(pattern))
                .collect(),
        }
    }

    pub(crate) fn is_excluded(&self, root: &Path, path: &Path) -> bool {
        let Some(relative) = path.strip_prefix(root).ok() else {
            return false;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        self.patterns
            .iter()
            .any(|pattern| pattern.matches(&relative))
    }
}

fn glob_matches(pattern: &[char], path: &str) -> bool {
    let path = path.chars().collect::<Vec<_>>();
    let mut memo = vec![vec![None; path.len() + 1]; pattern.len() + 1];

    fn visit(
        pattern: &[char],
        path: &[char],
        pattern_index: usize,
        path_index: usize,
        memo: &mut [Vec<Option<bool>>],
    ) -> bool {
        if let Some(result) = memo[pattern_index][path_index] {
            return result;
        }
        let result = if pattern_index == pattern.len() {
            path_index == path.len()
        } else if pattern[pattern_index] == '*' {
            if pattern.get(pattern_index + 1) == Some(&'*') {
                let after_double_star = if pattern.get(pattern_index + 2) == Some(&'/') {
                    pattern_index + 3
                } else {
                    pattern_index + 2
                };
                visit(pattern, path, after_double_star, path_index, memo)
                    || (path_index < path.len()
                        && visit(pattern, path, pattern_index, path_index + 1, memo))
            } else {
                visit(pattern, path, pattern_index + 1, path_index, memo)
                    || (path_index < path.len()
                        && path[path_index] != '/'
                        && visit(pattern, path, pattern_index, path_index + 1, memo))
            }
        } else {
            path_index < path.len()
                && ((pattern[pattern_index] == '?' && path[path_index] != '/')
                    || pattern[pattern_index] == path[path_index])
                && visit(pattern, path, pattern_index + 1, path_index + 1, memo)
        };
        memo[pattern_index][path_index] = Some(result);
        result
    }

    visit(pattern, &path, 0, 0, &mut memo)
}

#[cfg(test)]
mod tests {
    use super::Pattern;

    fn matches(pattern: &str, path: &str) -> bool {
        Pattern::new(pattern).matches(path)
    }

    #[test]
    fn star_does_not_cross_a_separator() {
        assert!(matches("*.f90", "source.f90"));
        assert!(!matches("/*.f90", "src/source.f90"));
        assert!(matches("src/*.f90", "src/source.f90"));
        assert!(!matches("src/*.f90", "src/nested/source.f90"));
    }

    #[test]
    fn double_star_crosses_separators_and_can_be_empty() {
        assert!(matches("**/*.f90", "source.f90"));
        assert!(matches("**/*.f90", "src/nested/source.f90"));
        assert!(matches("src/**/source.f90", "src/source.f90"));
        assert!(matches("src/**/source.f90", "src/nested/source.f90"));
        assert!(!matches("src/**/source.f90", "other/source.f90"));
    }

    #[test]
    fn question_mark_matches_one_non_separator_character() {
        assert!(matches("file?.f90", "file1.f90"));
        assert!(!matches("file?.f90", "file10.f90"));
        assert!(!matches("file?.f90", "file/a.f90"));
        assert!(!matches("a?b", "a/b"));
    }

    #[test]
    fn trailing_slash_matches_a_directory_prefix() {
        assert!(matches("vendor/", "vendor/generated.f90"));
        assert!(matches("vendor/", "vendor/deep/generated.f90"));
        assert!(!matches("vendor/", "vendorized/generated.f90"));
        assert!(matches("generated-*/", "src/generated-cache/file.f90"));
        // A directory pattern names directories only; the final segment of a
        // path is the file, so it is never a directory-prefix candidate.
        assert!(!matches("generated-*/", "src/generated-cache.f90"));
        assert!(!matches("vendor/", "vendor"));
        assert!(!matches("vendor/", "src/vendor"));
    }

    #[test]
    fn leading_slash_anchors_at_the_repository_root() {
        assert!(matches("/vendor/*.f90", "vendor/source.f90"));
        assert!(!matches("/vendor/*.f90", "src/vendor/source.f90"));
    }

    #[test]
    fn patterns_without_a_leading_slash_are_unanchored() {
        assert!(matches("vendor/*.f90", "src/vendor/source.f90"));
        assert!(matches("*.f90", "src/source.f90"));
        assert!(!matches("vendor/*.f90", "src/other/source.f90"));
        assert!(!matches("source.f90", "src/other.f90"));
    }

    #[test]
    fn non_matches_remain_non_matches() {
        assert!(!matches("*.f90", "source.f95"));
        assert!(!matches("src/**/test?.f90", "src/test10.f90"));
        assert!(!matches("/src/**", "other/source.f90"));
    }

    #[test]
    fn repository_paths_normalize_backslash_separators() {
        use super::ExcludeMatcher;
        use std::path::Path;

        let matcher = ExcludeMatcher::new(&["src/*.f90".to_string()]);
        assert!(matcher.is_excluded(Path::new("repo"), Path::new(r"repo/src\source.f90")));
    }
}
