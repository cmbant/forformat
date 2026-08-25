#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Construct {
    Associate,
    Block,
    Case,
    Changeteam,
    Critical,
    Do,
    Entry,
    Enum,
    Forall,
    If,
    Interface,
    Module,
    Procedure,
    Select,
    Type,
    Where,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OptionId {
    Config,
    NoConfig,
    Indent,
    StartIndent,
    IndentContains,
    IndentConstruct(Construct),
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
    CanonicalizeAndIndent,
    Wrap,
    NoWrap,
    Rewrap,
    LineLength,
    TargetStandard,
    UppercaseSingleL,
    Define,
    KeywordCase,
    OpenmpCase,
    RelationalSymbols,
    ArrayBrackets,
    CompactMultiplicative,
    JoinGoto,
    SplitCompoundKeywords,
    StripEmptyArgs,
    RemoveRedundantParens,
    NormalizeSemicolons,
    RemoveTerminalReturn,
    ProgramUnitSpacing,
    MaxBlankLines,
    DelimiterSpacing,
    CommentSpacing,
    ContinuationMarkers,
    RefactorEnd,
    InputFormat,
    OutputFormat,
    Help,
    Version,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CliArity {
    None,
    Required,
    Optional,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ValueKind {
    Flag,
    Boolean,
    OptionalBoolean,
    NonNegative,
    OptionalNonNegative,
    Indent,
    StartIndent,
    ContainsIndent,
    ContinuationIndent,
    WhitespaceReduction,
    KeywordCase,
    FortranStandard,
    MaxBlankLines,
    RefactorEnd,
    Text,
    Path,
    InputFormat,
    OutputFormat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Repeatability {
    LastWins,
    Once,
    Append,
    ReplaceLayer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfigMapping {
    None,
    Same,
    Keys(&'static [&'static str]),
    Mode(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfigPhase {
    Baseline,
    Specific,
}

#[derive(Clone, Copy)]
pub(crate) struct HelpLine {
    pub(crate) syntax: &'static str,
    pub(crate) description: &'static str,
}

#[derive(Clone, Copy)]
pub(crate) struct OptionSpec {
    pub(crate) id: OptionId,
    pub(crate) long: &'static str,
    pub(crate) aliases: &'static [&'static str],
    pub(crate) value_kind: ValueKind,
    pub(crate) cli_arity: CliArity,
    pub(crate) config: ConfigMapping,
    pub(crate) default: Option<&'static str>,
    pub(crate) repeatability: Repeatability,
    pub(crate) config_phase: ConfigPhase,
    pub(crate) help: Option<HelpLine>,
    pub(crate) suggest_single_dash: bool,
}

impl OptionSpec {
    const fn new(
        id: OptionId,
        long: &'static str,
        value_kind: ValueKind,
        cli_arity: CliArity,
    ) -> Self {
        Self {
            id,
            long,
            aliases: &[],
            value_kind,
            cli_arity,
            config: ConfigMapping::None,
            default: None,
            repeatability: Repeatability::LastWins,
            config_phase: ConfigPhase::Specific,
            help: None,
            suggest_single_dash: true,
        }
    }

    const fn aliases(mut self, aliases: &'static [&'static str]) -> Self {
        self.aliases = aliases;
        self
    }

    const fn config(mut self, config: ConfigMapping) -> Self {
        self.config = config;
        self
    }

    const fn default(mut self, default: &'static str) -> Self {
        self.default = Some(default);
        self
    }

    const fn repeatability(mut self, repeatability: Repeatability) -> Self {
        self.repeatability = repeatability;
        self
    }

    const fn config_phase(mut self, config_phase: ConfigPhase) -> Self {
        self.config_phase = config_phase;
        self
    }

    const fn help(mut self, syntax: &'static str, description: &'static str) -> Self {
        self.help = Some(HelpLine {
            syntax,
            description,
        });
        self
    }

    const fn no_single_dash_suggestion(mut self) -> Self {
        self.suggest_single_dash = false;
        self
    }
}

/// Shared option identity, grammar, configuration mapping, declared defaults,
/// repeatability, and user-facing terminal help.
///
/// CLI and TOML parsing both resolve through this table before creating typed
/// [`FormatSetting`] values. Runtime defaults remain embodied by
/// [`FormatConfig::default`], and ordered layer application implements the
/// declared repeat/merge behavior. The invariance tests below keep those runtime
/// semantics in lockstep with this metadata instead of duplicating defaults in a
/// second executable configuration object. TOML settings use [`ConfigPhase`] so
/// semantic baselines such as `indent` are applied before their more-specific
/// overrides without depending on map order.
pub(crate) static OPTIONS: &[OptionSpec] = &[
    OptionSpec::new(
        OptionId::Indent,
        "indent",
        ValueKind::Indent,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3")
    .config_phase(ConfigPhase::Baseline)
    .help(
        "-i<n>, -i-, --indent=<n|none>",
        "global indentation (default 3)",
    ),
    OptionSpec::new(
        OptionId::StartIndent,
        "start-indent",
        ValueKind::StartIndent,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("0")
    .help("-I<n|a>, --start-indent=<n|a>", "starting indentation"),
    OptionSpec::new(
        OptionId::IndentContains,
        "indent-contains",
        ValueKind::ContainsIndent,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Associate),
        "indent-associate",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Block),
        "indent-block",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Case),
        "indent-case",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("2"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Changeteam),
        "indent-changeteam",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Critical),
        "indent-critical",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Do),
        "indent-do",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Entry),
        "indent-entry",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("2"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Enum),
        "indent-enum",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Forall),
        "indent-forall",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::If),
        "indent-if",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Interface),
        "indent-interface",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Module),
        "indent-module",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Procedure),
        "indent-procedure",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Select),
        "indent-select",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Type),
        "indent-type",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Where),
        "indent-where",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3"),
    OptionSpec::new(
        OptionId::IncludeLeft,
        "include-left",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("false")
    .help(
        "--include-left=<BOOL>",
        "put INCLUDE at the starting indent",
    ),
    OptionSpec::new(
        OptionId::LabelLeft,
        "label-left",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true"),
    OptionSpec::new(
        OptionId::MaxIndent,
        "max-indent",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("100")
    .help(
        "-M<n>, --max-indent=<n>",
        "maximum indentation (0 = unlimited)",
    ),
    OptionSpec::new(
        OptionId::Openmp,
        "openmp",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true"),
    OptionSpec::new(
        OptionId::IndentAmpersand,
        "indent-ampersand",
        ValueKind::OptionalBoolean,
        CliArity::Optional,
    )
    .config(ConfigMapping::Same)
    .default("false")
    .help(
        "-K, --indent-ampersand[=<BOOL>]",
        "indent leading continuation ampersands",
    ),
    OptionSpec::new(
        OptionId::IndentContinuation,
        "indent-continuation",
        ValueKind::ContinuationIndent,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("3")
    .help(
        "-k<n>, --indent-continuation=<n>",
        "continuation indentation",
    ),
    OptionSpec::new(
        OptionId::AlignParen,
        "align-paren",
        ValueKind::OptionalNonNegative,
        CliArity::Optional,
    )
    .config(ConfigMapping::Same)
    .default("0")
    .help(
        "--align-paren[=<n|BOOL>]",
        "align continuation lines at parentheses",
    ),
    OptionSpec::new(
        OptionId::WsRemred,
        "reduce-whitespace",
        ValueKind::WhitespaceReduction,
        CliArity::Optional,
    )
    .aliases(&["ws-remred"])
    .config(ConfigMapping::Same)
    .default("0")
    .help("--reduce-whitespace[=<n>]", "reduce redundant whitespace"),
    OptionSpec::new(
        OptionId::AlignDeclarations,
        "align-declarations",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true")
    .help(
        "--align-declarations=<BOOL>",
        "shrink space to align `::` blocks (default 1)",
    ),
    OptionSpec::new(
        OptionId::AlignComments,
        "align-comments",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("false")
    .help(
        "--align-comments=<BOOL>",
        "shrink space to align trailing comment blocks (default 0)",
    ),
    OptionSpec::new(
        OptionId::LastIndent,
        "last-indent",
        ValueKind::Flag,
        CliArity::None,
    )
    .help(
        "--last-indent, -lastindent",
        "print final indentation instead of source",
    ),
    OptionSpec::new(
        OptionId::LastUsable,
        "last-usable",
        ValueKind::Flag,
        CliArity::None,
    )
    .help(
        "--last-usable, -lastusable",
        "print final usable indentation instead of source",
    ),
    OptionSpec::new(OptionId::All, "all", ValueKind::Flag, CliArity::None).help(
        "<paths>, --all [directory]",
        "format explicit files or all tracked sources recursively",
    ),
    OptionSpec::new(
        OptionId::AllFiles,
        "all-files",
        ValueKind::Flag,
        CliArity::None,
    )
    .help(
        "--all-files [directory]",
        "format this checkout's tracked sources; submodules are context only",
    ),
    OptionSpec::new(
        OptionId::NoSubmodules,
        "no-submodules",
        ValueKind::OptionalBoolean,
        CliArity::Optional,
    )
    .config(ConfigMapping::Same)
    .default("false")
    .help(
        "--no-submodules[=<BOOL>]",
        "omit submodule sources from targets and project context",
    ),
    OptionSpec::new(
        OptionId::ContextPath,
        "context-path",
        ValueKind::Path,
        CliArity::Required,
    )
    .config(ConfigMapping::Keys(&["context-paths"]))
    .repeatability(Repeatability::ReplaceLayer)
    .help(
        "--context-path=<directory>",
        "limit project context to sources beneath DIRECTORY; repeatable",
    ),
    OptionSpec::new(
        OptionId::ProjectContext,
        "project-context",
        ValueKind::Path,
        CliArity::Required,
    )
    .repeatability(Repeatability::Once)
    .help(
        "--project-context=<path>",
        "treat stdin as belonging to the Git project containing PATH",
    ),
    OptionSpec::new(OptionId::Stdin, "stdin", ValueKind::Flag, CliArity::None)
        .help("--stdin", "read source from stdin (default without paths)"),
    OptionSpec::new(OptionId::Stdout, "stdout", ValueKind::Flag, CliArity::None)
        .help("--stdout", "write one file's result to stdout"),
    OptionSpec::new(
        OptionId::Isolated,
        "isolated",
        ValueKind::Flag,
        CliArity::None,
    )
    .help(
        "--isolated",
        "do not scan repository sources for case resolution",
    ),
    OptionSpec::new(OptionId::Check, "check", ValueKind::Flag, CliArity::None)
        .help("--check", "exit 1 if selected files would change"),
    OptionSpec::new(OptionId::Diff, "diff", ValueKind::Flag, CliArity::None)
        .help("--diff", "print unified diffs and exit 1 if changed"),
    OptionSpec::new(
        OptionId::ShowFiles,
        "show-files",
        ValueKind::Flag,
        CliArity::None,
    )
    .help("--show-files", "print selected target paths without reading or formatting them"),
    OptionSpec::new(
        OptionId::QueryFormat,
        "query-format",
        ValueKind::Flag,
        CliArity::None,
    )
    .help("--query-format", "print free/fixed for each input and exit"),
    OptionSpec::new(
        OptionId::Exclude,
        "exclude",
        ValueKind::Text,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .repeatability(Repeatability::ReplaceLayer)
    .help(
        "--exclude=<glob>",
        "exclude tracked sources from selection and project scanning (repeatable)",
    ),
    OptionSpec::new(
        OptionId::ExtendExclude,
        "extend-exclude",
        ValueKind::Text,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .repeatability(Repeatability::Append)
    .help(
        "--extend-exclude=<glob>",
        "add to the exclusions instead of replacing them (repeatable)",
    ),
    OptionSpec::new(
        OptionId::IndentOnly,
        "indent-only",
        ValueKind::Flag,
        CliArity::None,
    )
    .config(ConfigMapping::Mode("indent-only"))
    .help("--indent-only", "findent-compatible indentation only"),
    OptionSpec::new(OptionId::Full, "full", ValueKind::Flag, CliArity::None)
        .config(ConfigMapping::Mode("full"))
        .help(
            "--full",
            "full formatting: normalization and wrapping (default)",
        ),
    OptionSpec::new(
        OptionId::NormalizeOnly,
        "normalize-only",
        ValueKind::Flag,
        CliArity::None,
    )
    .config(ConfigMapping::Mode("normalize-only"))
    .help(
        "--normalize-only",
        "normalization without structural layout",
    ),
    OptionSpec::new(
        OptionId::CanonicalizeOnly,
        "canonicalize-only",
        ValueKind::Flag,
        CliArity::None,
    )
    .config(ConfigMapping::Mode("canonicalize-only"))
    .help(
        "--canonicalize-only",
        "canonical spelling without whitespace or structural layout",
    ),
    OptionSpec::new(
        OptionId::CanonicalizeAndIndent,
        "canonicalize-and-indent",
        ValueKind::Flag,
        CliArity::None,
    )
    .config(ConfigMapping::Mode("canonicalize-and-indent"))
    .help(
        "--canonicalize-and-indent",
        "canonical spelling followed by findent-compatible indentation",
    ),
    OptionSpec::new(
        OptionId::Wrap,
        "wrap",
        ValueKind::OptionalBoolean,
        CliArity::Optional,
    )
    .config(ConfigMapping::Same)
    .default("true")
    .help(
        "--wrap[=<BOOL>], --no-wrap[=<BOOL>]",
        "reflow over-long statements (full mode)",
    ),
    OptionSpec::new(
        OptionId::NoWrap,
        "no-wrap",
        ValueKind::OptionalBoolean,
        CliArity::Optional,
    )
    .config(ConfigMapping::Same)
    .default("false"),
    OptionSpec::new(
        OptionId::Rewrap,
        "rewrap",
        ValueKind::OptionalBoolean,
        CliArity::Optional,
    )
    .config(ConfigMapping::Same)
    .default("false")
    .help(
        "--rewrap[=<BOOL>]",
        "repack eligible authored continuations (full mode)",
    ),
    OptionSpec::new(
        OptionId::LineLength,
        "line-length",
        ValueKind::NonNegative,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("120")
    .help("--line-length=<n>", "wrapping budget (default 120)"),
    OptionSpec::new(
        OptionId::TargetStandard,
        "target-standard",
        ValueKind::FortranStandard,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("f2003")
    .help(
        "--target-standard=<f95|f2003|f2008|f2018|f2023>",
        "cap syntax introduced by formatting (default f2003)",
    ),
    OptionSpec::new(
        OptionId::UppercaseSingleL,
        "uppercase-single-l",
        ValueKind::OptionalBoolean,
        CliArity::Optional,
    )
    .config(ConfigMapping::Same)
    .default("false")
    .help(
        "--uppercase-single-l[=<BOOL>]",
        "uppercase a lone `l` used as a name",
    ),
    OptionSpec::new(
        OptionId::Define,
        "define",
        ValueKind::Text,
        CliArity::Required,
    )
    .config(ConfigMapping::Keys(&["define", "defines"]))
    .repeatability(Repeatability::Append)
    .help(
        "-D NAME[=VALUE], --define=...",
        "define a macro name (repeatable)",
    ),
    OptionSpec::new(
        OptionId::KeywordCase,
        "keyword-case",
        ValueKind::KeywordCase,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("lower")
    .help(
        "--keyword-case=<lower|upper|preserve>",
        "recognized keyword case (default lower)",
    ),
    OptionSpec::new(
        OptionId::OpenmpCase,
        "openmp-case",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true")
    .help(
        "--openmp-case=<BOOL>",
        "uppercase reserved OpenMP directives (default true)",
    ),
    OptionSpec::new(
        OptionId::RelationalSymbols,
        "relational-symbols",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true")
    .help(
        "--relational-symbols=<BOOL>",
        "rewrite `.eq.` and friends as `==` (default true)",
    ),
    OptionSpec::new(
        OptionId::ArrayBrackets,
        "array-brackets",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true")
    .help(
        "--array-brackets=<BOOL>",
        "rewrite `(/ ... /)` as `[ ... ]` (default true)",
    ),
    OptionSpec::new(
        OptionId::CompactMultiplicative,
        "compact-multiplicative",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true")
    .help(
        "--compact-multiplicative=<BOOL>",
        "no spaces around binary `*`, `/`, `**` (default true)",
    ),
    OptionSpec::new(
        OptionId::JoinGoto,
        "join-goto",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true")
    .help(
        "--join-goto=<BOOL>",
        "write `go to` as `goto` (default true)",
    ),
    OptionSpec::new(
        OptionId::SplitCompoundKeywords,
        "split-compound-keywords",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true")
    .help(
        "--split-compound-keywords=<BOOL>",
        "write `endif` as `end if` (default true)",
    ),
    OptionSpec::new(
        OptionId::StripEmptyArgs,
        "strip-empty-args",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true")
    .help(
        "--strip-empty-args=<BOOL>",
        "strip empty SUBROUTINE definition arg lists (default true)",
    ),
    OptionSpec::new(
        OptionId::RemoveRedundantParens,
        "remove-redundant-parens",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true")
    .help(
        "--remove-redundant-parens=<BOOL>",
        "remove redundant parentheses (default true)",
    ),
    OptionSpec::new(
        OptionId::NormalizeSemicolons,
        "normalize-semicolons",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true")
    .help(
        "--normalize-semicolons=<BOOL>",
        "drop redundant statement separators (default true)",
    ),
    OptionSpec::new(
        OptionId::RemoveTerminalReturn,
        "remove-terminal-return",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true")
    .help(
        "--remove-terminal-return=<BOOL>",
        "remove terminal procedure RETURN (default true)",
    ),
    OptionSpec::new(
        OptionId::ProgramUnitSpacing,
        "program-unit-spacing",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true")
    .help(
        "--program-unit-spacing=<BOOL>",
        "canonical blank lines around program units (default true)",
    ),
    OptionSpec::new(
        OptionId::MaxBlankLines,
        "max-blank-lines",
        ValueKind::MaxBlankLines,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("2")
    .help(
        "--max-blank-lines=<n|preserve>",
        "blank-line cap (default 2)",
    ),
    OptionSpec::new(
        OptionId::DelimiterSpacing,
        "delimiter-spacing",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true")
    .help(
        "--delimiter-spacing=<BOOL>",
        "normalize spaces after delimiters (default true)",
    ),
    OptionSpec::new(
        OptionId::CommentSpacing,
        "comment-spacing",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true")
    .help(
        "--comment-spacing=<BOOL>",
        "normalize the gap before a trailing `!` (default true)",
    ),
    OptionSpec::new(
        OptionId::ContinuationMarkers,
        "continuation-markers",
        ValueKind::Boolean,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("true")
    .help(
        "--continuation-markers=<BOOL>",
        "normalize continuation markers and OpenMP sentinels (default true)",
    ),
    OptionSpec::new(
        OptionId::RefactorEnd,
        "refactor-end",
        ValueKind::RefactorEnd,
        CliArity::Optional,
    )
    .aliases(&["refactor-procedures"])
    .config(ConfigMapping::Same)
    .help(
        "-Rr, -RR, --refactor-end[=<BOOL>|upcase]",
        "complete END definition statements",
    ),
    OptionSpec::new(
        OptionId::InputFormat,
        "input-format",
        ValueKind::InputFormat,
        CliArity::Required,
    )
    .config(ConfigMapping::Same)
    .default("auto"),
    OptionSpec::new(
        OptionId::OutputFormat,
        "output-format",
        ValueKind::OutputFormat,
        CliArity::Required,
    )
    .config(ConfigMapping::Same),
    OptionSpec::new(
        OptionId::Config,
        "config",
        ValueKind::Path,
        CliArity::Required,
    )
    .repeatability(Repeatability::Once)
    .help(
        "--config=<path>",
        "use a project TOML configuration explicitly",
    ),
    OptionSpec::new(
        OptionId::NoConfig,
        "no-config",
        ValueKind::Flag,
        CliArity::None,
    )
    .help("--no-config", "ignore project TOML configuration"),
    OptionSpec::new(OptionId::Help, "help", ValueKind::Flag, CliArity::None)
        .help("-h, --help", "show this help")
        .no_single_dash_suggestion(),
    OptionSpec::new(
        OptionId::Version,
        "version",
        ValueKind::Flag,
        CliArity::None,
    )
    .help("-v, --version", "show version")
    .no_single_dash_suggestion(),
];

pub(crate) fn normalize_long(name: &str) -> String {
    name.replace('_', "-").to_ascii_lowercase()
}

pub(crate) fn lookup_long(name: &str) -> Option<&'static OptionSpec> {
    OPTIONS.iter().find(|spec| {
        !matches!(spec.id, OptionId::Help | OptionId::Version)
            && (spec.long == name || spec.aliases.contains(&name))
    })
}

pub(crate) fn lookup_any_long(name: &str) -> Option<&'static OptionSpec> {
    OPTIONS
        .iter()
        .find(|spec| spec.long == name || spec.aliases.contains(&name))
}

pub(crate) fn lookup_config(name: &str, mode_value: Option<&str>) -> Option<&'static OptionSpec> {
    OPTIONS.iter().find(|spec| match spec.config {
        ConfigMapping::None => false,
        ConfigMapping::Same => spec.long == name || spec.aliases.contains(&name),
        ConfigMapping::Keys(keys) => keys.contains(&name),
        ConfigMapping::Mode(expected) => {
            name == "mode" && mode_value.is_some_and(|value| value == expected)
        }
    })
}

#[cfg(test)]
pub(crate) fn primary_config_key(spec: &OptionSpec) -> Option<&'static str> {
    match spec.config {
        ConfigMapping::None => None,
        ConfigMapping::Same => Some(spec.long),
        ConfigMapping::Keys(keys) => keys.first().copied(),
        ConfigMapping::Mode(_) => Some("mode"),
    }
}

pub(crate) fn spec_for_id(id: OptionId) -> &'static OptionSpec {
    OPTIONS
        .iter()
        .find(|spec| spec.id == id)
        .expect("every parsed option id has a schema entry")
}

/// Point out the common `-all` typo without interfering with findent-style
/// short options such as `-i4` and `-ifree`.
pub(crate) fn single_dash_long_option_suggestion(arg: &str) -> Option<String> {
    let spelling = arg.strip_prefix('-')?;
    if spelling.is_empty() || spelling.starts_with('-') {
        return None;
    }
    let name = spelling.split_once('=').map_or(spelling, |(name, _)| name);
    let normalized = normalize_long(name);
    let known = lookup_any_long(&normalized).is_some_and(|spec| spec.suggest_single_dash)
        || normalized.starts_with("indent-");
    known.then(|| format!("--{spelling}"))
}

#[cfg(test)]
mod tests {
    use super::{
        lookup_config, normalize_long, primary_config_key, CliArity, ConfigMapping, OptionId,
        Repeatability, OPTIONS,
    };
    use crate::{
        cli::{parse, Command},
        config::FormatConfig,
    };
    use std::collections::{BTreeSet, HashSet};

    #[test]
    fn option_schema_names_and_aliases_are_unique() {
        let mut seen = HashSet::new();
        for spec in OPTIONS {
            assert!(seen.insert(spec.long), "duplicate option --{}", spec.long);
            for alias in spec.aliases {
                assert!(seen.insert(*alias), "duplicate option alias --{alias}");
            }
        }
    }

    #[test]
    fn configuration_mappings_round_trip_through_the_schema() {
        for spec in OPTIONS {
            match spec.config {
                ConfigMapping::None => {}
                ConfigMapping::Mode(value) => {
                    assert_eq!(
                        lookup_config("mode", Some(value)).map(|item| item.id),
                        Some(spec.id)
                    );
                }
                ConfigMapping::Same | ConfigMapping::Keys(_) => {
                    let key = primary_config_key(spec).expect("configurable option has a key");
                    assert_eq!(lookup_config(key, None).map(|item| item.id), Some(spec.id));
                }
            }
        }
    }

    fn contains_name(text: &str, name: &str) -> bool {
        text.match_indices(name).any(|(start, _)| {
            let before = text[..start].chars().next_back();
            let after = text[start + name.len()..].chars().next();
            let is_name_char = |ch: char| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_');
            before.is_none_or(|ch| !is_name_char(ch)) && after.is_none_or(|ch| !is_name_char(ch))
        })
    }

    #[test]
    fn schema_defaults_match_runtime_defaults() {
        let expected = FormatConfig::default();

        for spec in OPTIONS {
            let Some(default) = spec.default else {
                continue;
            };
            let option = match spec.cli_arity {
                CliArity::None => format!("--{}", spec.long),
                CliArity::Required | CliArity::Optional => {
                    format!("--{}={default}", spec.long)
                }
            };
            let Command::Run(invocation) = parse([
                "forformat".to_string(),
                "--no-config".to_string(),
                option,
            ])
            .unwrap_or_else(|error| panic!("schema default for --{} is invalid: {error}", spec.long))
            else {
                panic!("schema default for --{} did not produce a run", spec.long)
            };

            assert_eq!(
                invocation.config, expected,
                "schema default `{default}` for --{} differs from FormatConfig::default()",
                spec.long
            );
            match spec.id {
                OptionId::NoSubmodules => assert!(!invocation.no_submodules),
                OptionId::InputFormat => assert!(!invocation.force_free_input),
                _ => {}
            }
        }
    }

    #[test]
    fn non_default_repeatability_categories_are_exhaustive() {
        let names = |repeatability| {
            OPTIONS
                .iter()
                .filter(|spec| spec.repeatability == repeatability)
                .map(|spec| spec.long)
                .collect::<BTreeSet<_>>()
        };
        let expected = |values: &[&'static str]| values.iter().copied().collect::<BTreeSet<_>>();

        assert_eq!(
            names(Repeatability::Once),
            expected(&["config", "project-context"])
        );
        assert_eq!(
            names(Repeatability::Append),
            expected(&["define", "extend-exclude"])
        );
        assert_eq!(
            names(Repeatability::ReplaceLayer),
            expected(&["context-path", "exclude"])
        );
    }

    #[test]
    fn docs_cover_schema_names_config_keys_and_defaults() {
        let docs = include_str!("../../docs/options.md");
        let normalized_docs = docs.replace('`', "").replace('_', "-");

        for spec in OPTIONS {
            if matches!(spec.id, OptionId::Help | OptionId::Version) {
                continue;
            }
            let option = format!("--{}", spec.long);
            assert!(
                contains_name(&normalized_docs, &option),
                "docs/options.md does not mention {option}"
            );

            if let Some(key) = primary_config_key(spec) {
                let key = normalize_long(key);
                assert!(
                    contains_name(&normalized_docs, &key),
                    "docs/options.md does not mention config key `{key}` for --{}",
                    spec.long
                );
            }

            let Some(default) = spec.default else {
                continue;
            };
            if let Some(line) = normalized_docs
                .lines()
                .find(|line| line.trim_start().starts_with('|') && contains_name(line, &option))
            {
                assert!(
                    line.contains(default),
                    "docs/options.md does not show default `{default}` on the --{} row",
                    spec.long
                );
            }
        }
    }
}
