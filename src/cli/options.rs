#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OptionId {
    Config,
    NoConfig,
    Indent,
    StartIndent,
    IndentContains,
    IncludeLeft,
    LabelLeft,
    MaxIndent,
    Openmp,
    IndentAmpersand,
    IndentContinuation,
    AlignParen,
    WsRemred,
    AlignDeclarations,
    AlignComments,
    LastIndent,
    LastUsable,
    All,
    AllFiles,
    NoSubmodules,
    ContextPath,
    ProjectContext,
    Stdin,
    Stdout,
    Isolated,
    Check,
    Diff,
    ShowFiles,
    QueryFormat,
    Exclude,
    ExtendExclude,
    IndentOnly,
    Full,
    NormalizeOnly,
    CanonicalizeOnly,
    Wrap,
    NoWrap,
    Rewrap,
    LineLength,
    UppercaseSingleL,
    Define,
    KeywordCase,
    RelationalSymbols,
    ArrayBrackets,
    CompactMultiplicative,
    JoinGoto,
    SplitCompoundKeywords,
    StripEmptyArgs,
    RemoveRedundantParens,
    RemoveTerminalReturn,
    ProgramUnitSpacing,
    MaxBlankLines,
    DelimiterSpacing,
    CommentSpacing,
    ContinuationMarkers,
    IndentChangeteam,
    RefactorEnd,
    InputFormat,
    OutputFormat,
    Help,
    Version,
}

#[derive(Clone, Copy)]
pub(super) struct HelpLine {
    pub(super) syntax: &'static str,
    pub(super) description: &'static str,
}

#[derive(Clone, Copy)]
pub(super) struct OptionSpec {
    pub(super) id: OptionId,
    pub(super) long: &'static str,
    pub(super) aliases: &'static [&'static str],
    pub(super) help: Option<HelpLine>,
    pub(super) suggest_single_dash: bool,
}

macro_rules! spec {
    ($id:ident, $long:literal) => {
        OptionSpec {
            id: OptionId::$id,
            long: $long,
            aliases: &[],
            help: None,
            suggest_single_dash: true,
        }
    };
    ($id:ident, $long:literal, $syntax:literal, $description:literal) => {
        OptionSpec {
            id: OptionId::$id,
            long: $long,
            aliases: &[],
            help: Some(HelpLine {
                syntax: $syntax,
                description: $description,
            }),
            suggest_single_dash: true,
        }
    };
    ($id:ident, $long:literal, aliases = $aliases:expr) => {
        OptionSpec {
            id: OptionId::$id,
            long: $long,
            aliases: $aliases,
            help: None,
            suggest_single_dash: true,
        }
    };
    ($id:ident, $long:literal, aliases = $aliases:expr, $syntax:literal, $description:literal) => {
        OptionSpec {
            id: OptionId::$id,
            long: $long,
            aliases: $aliases,
            help: Some(HelpLine {
                syntax: $syntax,
                description: $description,
            }),
            suggest_single_dash: true,
        }
    };
}

/// Canonical long-option identity and user-facing documentation.
///
/// This is intentionally metadata, not a parser table. The findent-compatible
/// value grammar remains explicit in `parse::long`; this list only removes the
/// three independent inventories that used to live in the long-option match,
/// the single-dash typo helper, and `usage()`.
pub(super) static OPTIONS: &[OptionSpec] = &[
    spec!(
        Indent,
        "indent",
        "-i<n>, -i-, --indent=<n|none>",
        "global indentation (default 3)"
    ),
    spec!(
        StartIndent,
        "start-indent",
        "-I<n|a>, --start-indent=<n|a>",
        "starting indentation"
    ),
    spec!(IndentContains, "indent-contains"),
    spec!(
        IncludeLeft,
        "include-left",
        "--include-left=<BOOL>",
        "put INCLUDE at the starting indent"
    ),
    spec!(LabelLeft, "label-left"),
    spec!(
        MaxIndent,
        "max-indent",
        "-M<n>, --max-indent=<n>",
        "maximum indentation (0 = unlimited)"
    ),
    spec!(Openmp, "openmp"),
    spec!(
        IndentAmpersand,
        "indent-ampersand",
        "-K, --indent-ampersand[=<BOOL>]",
        "indent leading continuation ampersands"
    ),
    spec!(
        IndentContinuation,
        "indent-continuation",
        "-k<n>, --indent-continuation=<n>",
        "continuation indentation"
    ),
    spec!(
        AlignParen,
        "align-paren",
        "--align-paren[=<n>]",
        "align continuation lines at parentheses"
    ),
    spec!(
        WsRemred,
        "reduce-whitespace",
        aliases = &["ws-remred"],
        "--reduce-whitespace[=<n>]",
        "reduce redundant whitespace"
    ),
    spec!(
        AlignDeclarations,
        "align-declarations",
        "--align-declarations=<BOOL>",
        "shrink space to align `::` blocks (default 1)"
    ),
    spec!(
        AlignComments,
        "align-comments",
        "--align-comments=<BOOL>",
        "shrink space to align trailing comment blocks (default 0)"
    ),
    spec!(
        LastIndent,
        "last-indent",
        "-lastindent, -lastusable",
        "print query result instead of source"
    ),
    spec!(LastUsable, "last-usable"),
    spec!(
        All,
        "all",
        "<paths>, --all [directory]",
        "format explicit files or all tracked sources recursively"
    ),
    spec!(
        AllFiles,
        "all-files",
        "--all-files [directory]",
        "format this checkout's tracked sources; submodules are context only"
    ),
    spec!(
        NoSubmodules,
        "no-submodules",
        "--no-submodules[=<BOOL>]",
        "omit submodule sources from targets and project context"
    ),
    spec!(
        ContextPath,
        "context-path",
        "--context-path=<directory>",
        "limit project context to sources beneath DIRECTORY; repeatable"
    ),
    spec!(
        ProjectContext,
        "project-context",
        "--project-context=<path>",
        "treat stdin as belonging to the Git project containing PATH"
    ),
    spec!(
        Stdin,
        "stdin",
        "--stdin",
        "read source from stdin (default without paths)"
    ),
    spec!(
        Stdout,
        "stdout",
        "--stdout",
        "write one file's result to stdout"
    ),
    spec!(
        Isolated,
        "isolated",
        "--isolated",
        "do not scan repository sources for case resolution"
    ),
    spec!(
        Check,
        "check",
        "--check",
        "exit 1 if selected files would change"
    ),
    spec!(
        Diff,
        "diff",
        "--diff",
        "print unified diffs and exit 1 if changed"
    ),
    spec!(
        ShowFiles,
        "show-files",
        "--show-files",
        "print selected files without formatting"
    ),
    spec!(
        QueryFormat,
        "query-format",
        "--query-format",
        "print free/fixed for each input and exit"
    ),
    spec!(
        Exclude,
        "exclude",
        "--exclude=<glob>",
        "exclude tracked sources from selection and project scanning (repeatable)"
    ),
    spec!(
        ExtendExclude,
        "extend-exclude",
        "--extend-exclude=<glob>",
        "add to the exclusions instead of replacing them (repeatable)"
    ),
    spec!(
        IndentOnly,
        "indent-only",
        "--indent-only",
        "findent-compatible indentation only"
    ),
    spec!(
        Full,
        "full",
        "--full",
        "full formatting: normalization and wrapping (default)"
    ),
    spec!(
        NormalizeOnly,
        "normalize-only",
        "--normalize-only",
        "normalization without structural layout"
    ),
    spec!(
        CanonicalizeOnly,
        "canonicalize-only",
        "--canonicalize-only",
        "canonical spelling without whitespace or structural layout"
    ),
    spec!(
        Wrap,
        "wrap",
        "--wrap[=<BOOL>], --no-wrap[=<BOOL>]",
        "reflow over-long statements (full mode)"
    ),
    spec!(NoWrap, "no-wrap"),
    spec!(
        Rewrap,
        "rewrap",
        "--rewrap[=<BOOL>]",
        "repack eligible authored continuations (full mode)"
    ),
    spec!(
        LineLength,
        "line-length",
        "--line-length=<n>",
        "wrapping budget (default 120)"
    ),
    spec!(
        UppercaseSingleL,
        "uppercase-single-l",
        "--uppercase-single-l[=<BOOL>]",
        "uppercase a lone `l` used as a name"
    ),
    spec!(
        Define,
        "define",
        "-D NAME[=VALUE], --define=...",
        "define a macro name (repeatable)"
    ),
    spec!(
        KeywordCase,
        "keyword-case",
        "--keyword-case=<lower|upper|preserve>",
        "recognized keyword case (default lower)"
    ),
    spec!(
        RelationalSymbols,
        "relational-symbols",
        "--relational-symbols=<BOOL>",
        "rewrite `.eq.` and friends as `==` (default true)"
    ),
    spec!(
        ArrayBrackets,
        "array-brackets",
        "--array-brackets=<BOOL>",
        "rewrite `(/ ... /)` as `[ ... ]` (default true)"
    ),
    spec!(
        CompactMultiplicative,
        "compact-multiplicative",
        "--compact-multiplicative=<BOOL>",
        "no spaces around binary `*`, `/`, `**` (default true)"
    ),
    spec!(
        JoinGoto,
        "join-goto",
        "--join-goto=<BOOL>",
        "write `go to` as `goto` (default true)"
    ),
    spec!(
        SplitCompoundKeywords,
        "split-compound-keywords",
        "--split-compound-keywords=<BOOL>",
        "write `endif` as `end if` (default true)"
    ),
    spec!(
        StripEmptyArgs,
        "strip-empty-args",
        "--strip-empty-args=<BOOL>",
        "strip empty SUBROUTINE definition arg lists (default true)"
    ),
    spec!(
        RemoveRedundantParens,
        "remove-redundant-parens",
        "--remove-redundant-parens=<BOOL>",
        "remove redundant parentheses (default true)"
    ),
    spec!(
        RemoveTerminalReturn,
        "remove-terminal-return",
        "--remove-terminal-return=<BOOL>",
        "remove terminal procedure RETURN (default true)"
    ),
    spec!(
        ProgramUnitSpacing,
        "program-unit-spacing",
        "--program-unit-spacing=<BOOL>",
        "canonical blank lines around program units (default true)"
    ),
    spec!(
        MaxBlankLines,
        "max-blank-lines",
        "--max-blank-lines=<n|preserve>",
        "blank-line cap (default 2)"
    ),
    spec!(
        DelimiterSpacing,
        "delimiter-spacing",
        "--delimiter-spacing=<BOOL>",
        "normalize spaces after delimiters (default true)"
    ),
    spec!(
        CommentSpacing,
        "comment-spacing",
        "--comment-spacing=<BOOL>",
        "normalize the gap before a trailing `!` (default true)"
    ),
    spec!(
        ContinuationMarkers,
        "continuation-markers",
        "--continuation-markers=<BOOL>",
        "normalize continuation markers and OpenMP sentinels (default true)"
    ),
    spec!(IndentChangeteam, "indent-changeteam"),
    spec!(
        RefactorEnd,
        "refactor-end",
        aliases = &["refactor-procedures"],
        "-Rr, -RR, --refactor-end[=<BOOL>|upcase]",
        "complete END definition statements"
    ),
    spec!(InputFormat, "input-format"),
    spec!(OutputFormat, "output-format"),
    spec!(
        Config,
        "config",
        "--config=<path>",
        "use a project TOML configuration explicitly"
    ),
    spec!(
        NoConfig,
        "no-config",
        "--no-config",
        "ignore project TOML configuration"
    ),
    OptionSpec {
        id: OptionId::Help,
        long: "help",
        aliases: &[],
        help: Some(HelpLine {
            syntax: "-h, --help",
            description: "show this help",
        }),
        suggest_single_dash: false,
    },
    OptionSpec {
        id: OptionId::Version,
        long: "version",
        aliases: &[],
        help: Some(HelpLine {
            syntax: "-v, --version",
            description: "show version",
        }),
        suggest_single_dash: false,
    },
];

pub(super) fn normalize_long(name: &str) -> String {
    name.replace('_', "-").to_ascii_lowercase()
}

pub(super) fn lookup_long(name: &str) -> Option<OptionId> {
    OPTIONS
        .iter()
        .find(|spec| {
            !matches!(spec.id, OptionId::Help | OptionId::Version)
                && (spec.long == name || spec.aliases.contains(&name))
        })
        .map(|spec| spec.id)
}

fn spec_for_name(name: &str) -> Option<&'static OptionSpec> {
    OPTIONS
        .iter()
        .find(|spec| spec.long == name || spec.aliases.contains(&name))
}

/// Point out the common `-all` typo without interfering with findent-style
/// short options such as `-i4` and `-ifree`.
pub(super) fn single_dash_long_option_suggestion(arg: &str) -> Option<String> {
    let spelling = arg.strip_prefix('-')?;
    if spelling.is_empty() || spelling.starts_with('-') {
        return None;
    }
    let name = spelling.split_once('=').map_or(spelling, |(name, _)| name);
    let normalized = normalize_long(name);
    let known = spec_for_name(&normalized).is_some_and(|spec| spec.suggest_single_dash)
        || normalized.starts_with("indent-");
    known.then(|| format!("--{spelling}"))
}
