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

impl ValueKind {
    pub(crate) const fn cli_arity(self) -> CliArity {
        match self {
            Self::Flag => CliArity::None,
            Self::OptionalBoolean
            | Self::OptionalNonNegative
            | Self::WhitespaceReduction
            | Self::RefactorEnd => CliArity::Optional,
            _ => CliArity::Required,
        }
    }
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
    pub(crate) config: ConfigMapping,
    pub(crate) default_text: Option<&'static str>,
    pub(crate) config_phase: ConfigPhase,
    pub(crate) help: Option<HelpLine>,
    pub(crate) suggest_single_dash: bool,
}

impl OptionSpec {
    const fn new(id: OptionId, long: &'static str, value_kind: ValueKind) -> Self {
        Self {
            id,
            long,
            aliases: &[],
            value_kind,
            config: ConfigMapping::None,
            default_text: None,
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

    const fn default_text(mut self, default_text: &'static str) -> Self {
        self.default_text = Some(default_text);
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

/// Shared option identity, value grammar, configuration mapping, documented
/// defaults, config ordering, and user-facing terminal help.
///
/// CLI and TOML parsing both resolve through this table before creating typed
/// [`FormatSetting`] values. CLI arity is derived from [`ValueKind`]. Runtime
/// defaults remain embodied by `FormatConfig::default()`, while typed layer
/// fields and their merge code own repeat/replace semantics. The invariance test
/// below keeps documented default text aligned with the runtime defaults. TOML
/// settings use [`ConfigPhase`] so semantic baselines such as `indent` are
/// applied before their more-specific overrides without depending on map order.
pub(crate) static OPTIONS: &[OptionSpec] = &[
    OptionSpec::new(OptionId::Indent, "indent", ValueKind::Indent)
        .config(ConfigMapping::Same)
        .default_text("3")
        .config_phase(ConfigPhase::Baseline)
        .help(
            "-i<n>, -i-, --indent=<n|none>",
            "global indentation (default 3)",
        ),
    OptionSpec::new(
        OptionId::StartIndent,
        "start-indent",
        ValueKind::StartIndent,
    )
    .config(ConfigMapping::Same)
    .default_text("0")
    .help("-I<n|a>, --start-indent=<n|a>", "starting indentation"),
    OptionSpec::new(
        OptionId::IndentContains,
        "indent-contains",
        ValueKind::ContainsIndent,
    )
    .config(ConfigMapping::Same)
    .default_text("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Associate),
        "indent-associate",
        ValueKind::NonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Block),
        "indent-block",
        ValueKind::NonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Case),
        "indent-case",
        ValueKind::NonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("2"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Changeteam),
        "indent-changeteam",
        ValueKind::NonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Critical),
        "indent-critical",
        ValueKind::NonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Do),
        "indent-do",
        ValueKind::NonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Entry),
        "indent-entry",
        ValueKind::NonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("2"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Enum),
        "indent-enum",
        ValueKind::NonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Forall),
        "indent-forall",
        ValueKind::NonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::If),
        "indent-if",
        ValueKind::NonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Interface),
        "indent-interface",
        ValueKind::NonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Module),
        "indent-module",
        ValueKind::NonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Procedure),
        "indent-procedure",
        ValueKind::NonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Select),
        "indent-select",
        ValueKind::NonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Type),
        "indent-type",
        ValueKind::NonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("3"),
    OptionSpec::new(
        OptionId::IndentConstruct(Construct::Where),
        "indent-where",
        ValueKind::NonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("3"),
    OptionSpec::new(OptionId::IncludeLeft, "include-left", ValueKind::Boolean)
        .config(ConfigMapping::Same)
        .default_text("false")
        .help(
            "--include-left=<BOOL>",
            "put INCLUDE at the starting indent",
        ),
    OptionSpec::new(OptionId::LabelLeft, "label-left", ValueKind::Boolean)
        .config(ConfigMapping::Same)
        .default_text("true"),
    OptionSpec::new(OptionId::MaxIndent, "max-indent", ValueKind::NonNegative)
        .config(ConfigMapping::Same)
        .default_text("100")
        .help(
            "-M<n>, --max-indent=<n>",
            "maximum indentation (0 = unlimited)",
        ),
    OptionSpec::new(OptionId::Openmp, "openmp", ValueKind::Boolean)
        .config(ConfigMapping::Same)
        .default_text("true"),
    OptionSpec::new(
        OptionId::IndentAmpersand,
        "indent-ampersand",
        ValueKind::OptionalBoolean,
    )
    .config(ConfigMapping::Same)
    .default_text("false")
    .help(
        "-K, --indent-ampersand[=<BOOL>]",
        "indent leading continuation ampersands",
    ),
    OptionSpec::new(
        OptionId::IndentContinuation,
        "indent-continuation",
        ValueKind::ContinuationIndent,
    )
    .config(ConfigMapping::Same)
    .default_text("3")
    .help(
        "-k<n>, --indent-continuation=<n>",
        "continuation indentation",
    ),
    OptionSpec::new(
        OptionId::AlignParen,
        "align-paren",
        ValueKind::OptionalNonNegative,
    )
    .config(ConfigMapping::Same)
    .default_text("0")
    .help(
        "--align-paren[=<n|BOOL>]",
        "align continuation lines at parentheses",
    ),
    OptionSpec::new(
        OptionId::WsRemred,
        "reduce-whitespace",
        ValueKind::WhitespaceReduction,
    )
    .aliases(&["ws-remred"])
    .config(ConfigMapping::Same)
    .default_text("0")
    .help(
        "--reduce-whitespace[=<n|BOOL>]",
        "reduce redundant whitespace",
    ),
    OptionSpec::new(
        OptionId::AlignDeclarations,
        "align-declarations",
        ValueKind::Boolean,
    )
    .config(ConfigMapping::Same)
    .default_text("true")
    .help(
        "--align-declarations=<BOOL>",
        "shrink space to align `::` blocks (default 1)",
    ),
    OptionSpec::new(
        OptionId::AlignComments,
        "align-comments",
        ValueKind::Boolean,
    )
    .config(ConfigMapping::Same)
    .default_text("false")
    .help(
        "--align-comments=<BOOL>",
        "shrink space to align trailing comment blocks (default 0)",
    ),
    OptionSpec::new(OptionId::LastIndent, "last-indent", ValueKind::Flag).help(
        "--last-indent, -lastindent",
        "print final indentation instead of source",
    ),
    OptionSpec::new(OptionId::LastUsable, "last-usable", ValueKind::Flag).help(
        "--last-usable, -lastusable",
        "print final usable indentation instead of source",
    ),
    OptionSpec::new(OptionId::All, "all", ValueKind::Flag).help(
        "<paths>, --all [directory]",
        "format explicit files or all tracked sources recursively",
    ),
    OptionSpec::new(OptionId::AllFiles, "all-files", ValueKind::Flag).help(
        "--all-files [directory]",
        "format this checkout's tracked sources; submodules are context only",
    ),
    OptionSpec::new(
        OptionId::NoSubmodules,
        "no-submodules",
        ValueKind::OptionalBoolean,
    )
    .config(ConfigMapping::Same)
    .default_text("false")
    .help(
        "--no-submodules[=<BOOL>]",
        "omit submodule sources from targets and project context",
    ),
    OptionSpec::new(OptionId::ContextPath, "context-path", ValueKind::Path)
        .config(ConfigMapping::Keys(&["context-paths"]))
        .help(
            "--context-path=<directory>",
            "limit project context to sources beneath DIRECTORY; repeatable",
        ),
    OptionSpec::new(OptionId::ProjectContext, "project-context", ValueKind::Path).help(
        "--project-context=<path>",
        "treat stdin as belonging to the Git project containing PATH",
    ),
    OptionSpec::new(OptionId::Stdin, "stdin", ValueKind::Flag)
        .help("--stdin", "read source from stdin (default without paths)"),
    OptionSpec::new(OptionId::Stdout, "stdout", ValueKind::Flag)
        .help("--stdout", "write one file's result to stdout"),
    OptionSpec::new(OptionId::Isolated, "isolated", ValueKind::Flag).help(
        "--isolated",
        "do not scan repository sources for case resolution",
    ),
    OptionSpec::new(OptionId::Check, "check", ValueKind::Flag)
        .help("--check", "exit 1 if selected files would change"),
    OptionSpec::new(OptionId::Diff, "diff", ValueKind::Flag)
        .help("--diff", "print unified diffs and exit 1 if changed"),
    OptionSpec::new(OptionId::ShowFiles, "show-files", ValueKind::Flag)
        .help("--show-files", "print selected files without formatting"),
    OptionSpec::new(OptionId::QueryFormat, "query-format", ValueKind::Flag)
        .help("--query-format", "print free/fixed for each input and exit"),
    OptionSpec::new(OptionId::Exclude, "exclude", ValueKind::Text)
        .config(ConfigMapping::Same)
        .help(
            "--exclude=<glob>",
            "exclude tracked sources from selection and project scanning (repeatable)",
        ),
    OptionSpec::new(OptionId::ExtendExclude, "extend-exclude", ValueKind::Text)
        .config(ConfigMapping::Same)
        .help(
            "--extend-exclude=<glob>",
            "add to the exclusions instead of replacing them (repeatable)",
        ),
    OptionSpec::new(OptionId::IndentOnly, "indent-only", ValueKind::Flag)
        .config(ConfigMapping::Mode("indent-only"))
        .help("--indent-only", "findent-compatible indentation only"),
    OptionSpec::new(OptionId::Full, "full", ValueKind::Flag)
        .config(ConfigMapping::Mode("full"))
        .help(
            "--full",
            "full formatting: normalization and wrapping (default)",
        ),
    OptionSpec::new(OptionId::NormalizeOnly, "normalize-only", ValueKind::Flag)
        .config(ConfigMapping::Mode("normalize-only"))
        .help(
            "--normalize-only",
            "normalization without structural layout",
        ),
    OptionSpec::new(
        OptionId::CanonicalizeOnly,
        "canonicalize-only",
        ValueKind::Flag,
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
    )
    .config(ConfigMapping::Mode("canonicalize-and-indent"))
    .help(
        "--canonicalize-and-indent",
        "canonical spelling followed by findent-compatible indentation",
    ),
    OptionSpec::new(OptionId::Wrap, "wrap", ValueKind::OptionalBoolean)
        .config(ConfigMapping::Same)
        .default_text("true")
        .help(
            "--wrap[=<BOOL>], --no-wrap[=<BOOL>]",
            "reflow over-long statements (full mode)",
        ),
    OptionSpec::new(OptionId::NoWrap, "no-wrap", ValueKind::OptionalBoolean)
        .config(ConfigMapping::Same)
        .default_text("false"),
    OptionSpec::new(OptionId::Rewrap, "rewrap", ValueKind::OptionalBoolean)
        .config(ConfigMapping::Same)
        .default_text("false")
        .help(
            "--rewrap[=<BOOL>]",
            "repack eligible authored continuations (full mode)",
        ),
    OptionSpec::new(OptionId::LineLength, "line-length", ValueKind::NonNegative)
        .config(ConfigMapping::Same)
        .default_text("120")
        .help("--line-length=<n>", "wrapping budget (default 120)"),
    OptionSpec::new(
        OptionId::TargetStandard,
        "target-standard",
        ValueKind::FortranStandard,
    )
    .config(ConfigMapping::Same)
    .default_text("f2003")
    .help(
        "--target-standard=<f95|f2003|f2008|f2018|f2023>",
        "cap syntax introduced by formatting (default f2003)",
    ),
    OptionSpec::new(
        OptionId::UppercaseSingleL,
        "uppercase-single-l",
        ValueKind::OptionalBoolean,
    )
    .config(ConfigMapping::Same)
    .default_text("false")
    .help(
        "--uppercase-single-l[=<BOOL>]",
        "uppercase a lone `l` used as a name",
    ),
    OptionSpec::new(OptionId::Define, "define", ValueKind::Text)
        .config(ConfigMapping::Keys(&["define", "defines"]))
        .help(
            "-D NAME[=VALUE], --define=...",
            "define a macro name (repeatable)",
        ),
    OptionSpec::new(
        OptionId::KeywordCase,
        "keyword-case",
        ValueKind::KeywordCase,
    )
    .config(ConfigMapping::Same)
    .default_text("lower")
    .help(
        "--keyword-case=<lower|upper|preserve>",
        "recognized keyword case (default lower)",
    ),
    OptionSpec::new(OptionId::OpenmpCase, "openmp-case", ValueKind::Boolean)
        .config(ConfigMapping::Same)
        .default_text("true")
        .help(
            "--openmp-case=<BOOL>",
            "uppercase reserved OpenMP directives (default true)",
        ),
    OptionSpec::new(
        OptionId::RelationalSymbols,
        "relational-symbols",
        ValueKind::Boolean,
    )
    .config(ConfigMapping::Same)
    .default_text("true")
    .help(
        "--relational-symbols=<BOOL>",
        "rewrite `.eq.` and friends as `==` (default true)",
    ),
    OptionSpec::new(
        OptionId::ArrayBrackets,
        "array-brackets",
        ValueKind::Boolean,
    )
    .config(ConfigMapping::Same)
    .default_text("true")
    .help(
        "--array-brackets=<BOOL>",
        "rewrite `(/ ... /)` as `[ ... ]` (default true)",
    ),
    OptionSpec::new(
        OptionId::CompactMultiplicative,
        "compact-multiplicative",
        ValueKind::Boolean,
    )
    .config(ConfigMapping::Same)
    .default_text("true")
    .help(
        "--compact-multiplicative=<BOOL>",
        "no spaces around binary `*`, `/`, `**` (default true)",
    ),
    OptionSpec::new(OptionId::JoinGoto, "join-goto", ValueKind::Boolean)
        .config(ConfigMapping::Same)
        .default_text("true")
        .help(
            "--join-goto=<BOOL>",
            "write `go to` as `goto` (default true)",
        ),
    OptionSpec::new(
        OptionId::SplitCompoundKeywords,
        "split-compound-keywords",
        ValueKind::Boolean,
    )
    .config(ConfigMapping::Same)
    .default_text("true")
    .help(
        "--split-compound-keywords=<BOOL>",
        "write `endif` as `end if` (default true)",
    ),
    OptionSpec::new(
        OptionId::StripEmptyArgs,
        "strip-empty-args",
        ValueKind::Boolean,
    )
    .config(ConfigMapping::Same)
    .default_text("true")
    .help(
        "--strip-empty-args=<BOOL>",
        "strip empty SUBROUTINE definition arg lists (default true)",
    ),
    OptionSpec::new(
        OptionId::RemoveRedundantParens,
        "remove-redundant-parens",
        ValueKind::Boolean,
    )
    .config(ConfigMapping::Same)
    .default_text("true")
    .help(
        "--remove-redundant-parens=<BOOL>",
        "remove redundant parentheses (default true)",
    ),
    OptionSpec::new(
        OptionId::NormalizeSemicolons,
        "normalize-semicolons",
        ValueKind::Boolean,
    )
    .config(ConfigMapping::Same)
    .default_text("true")
    .help(
        "--normalize-semicolons=<BOOL>",
        "drop redundant statement separators (default true)",
    ),
    OptionSpec::new(
        OptionId::RemoveTerminalReturn,
        "remove-terminal-return",
        ValueKind::Boolean,
    )
    .config(ConfigMapping::Same)
    .default_text("true")
    .help(
        "--remove-terminal-return=<BOOL>",
        "remove terminal procedure RETURN (default true)",
    ),
    OptionSpec::new(
        OptionId::ProgramUnitSpacing,
        "program-unit-spacing",
        ValueKind::Boolean,
    )
    .config(ConfigMapping::Same)
    .default_text("true")
    .help(
        "--program-unit-spacing=<BOOL>",
        "canonical blank lines around program units (default true)",
    ),
    OptionSpec::new(
        OptionId::MaxBlankLines,
        "max-blank-lines",
        ValueKind::MaxBlankLines,
    )
    .config(ConfigMapping::Same)
    .default_text("2")
    .help(
        "--max-blank-lines=<n|preserve>",
        "blank-line cap (default 2)",
    ),
    OptionSpec::new(
        OptionId::DelimiterSpacing,
        "delimiter-spacing",
        ValueKind::Boolean,
    )
    .config(ConfigMapping::Same)
    .default_text("true")
    .help(
        "--delimiter-spacing=<BOOL>",
        "normalize spaces after delimiters (default true)",
    ),
    OptionSpec::new(
        OptionId::CommentSpacing,
        "comment-spacing",
        ValueKind::Boolean,
    )
    .config(ConfigMapping::Same)
    .default_text("true")
    .help(
        "--comment-spacing=<BOOL>",
        "normalize the gap before a trailing `!` (default true)",
    ),
    OptionSpec::new(
        OptionId::ContinuationMarkers,
        "continuation-markers",
        ValueKind::Boolean,
    )
    .config(ConfigMapping::Same)
    .default_text("true")
    .help(
        "--continuation-markers=<BOOL>",
        "normalize continuation markers and OpenMP sentinels (default true)",
    ),
    OptionSpec::new(
        OptionId::RefactorEnd,
        "refactor-end",
        ValueKind::RefactorEnd,
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
    )
    .config(ConfigMapping::Same)
    .default_text("auto"),
    OptionSpec::new(
        OptionId::OutputFormat,
        "output-format",
        ValueKind::OutputFormat,
    )
    .config(ConfigMapping::Same),
    OptionSpec::new(OptionId::Config, "config", ValueKind::Path).help(
        "--config=<path>",
        "use a project TOML configuration explicitly",
    ),
    OptionSpec::new(OptionId::NoConfig, "no-config", ValueKind::Flag)
        .help("--no-config", "ignore project TOML configuration"),
    OptionSpec::new(OptionId::Help, "help", ValueKind::Flag)
        .help("-h, --help", "show this help")
        .no_single_dash_suggestion(),
    OptionSpec::new(OptionId::Version, "version", ValueKind::Flag)
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
        OPTIONS,
    };
    use crate::{
        cli::{parse, Command},
        config::FormatConfig,
    };
    use std::collections::HashSet;

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
    fn schema_default_text_matches_runtime_defaults() {
        let expected = FormatConfig::default();

        for spec in OPTIONS {
            let Some(default_text) = spec.default_text else {
                continue;
            };
            let option = match spec.value_kind.cli_arity() {
                CliArity::None => format!("--{}", spec.long),
                CliArity::Required | CliArity::Optional => {
                    format!("--{}={default_text}", spec.long)
                }
            };
            let Command::Run(invocation) =
                parse(["forformat".to_string(), "--no-config".to_string(), option]).unwrap_or_else(
                    |error| {
                        panic!(
                            "schema default text for --{} is invalid: {error}",
                            spec.long
                        )
                    },
                )
            else {
                panic!(
                    "schema default text for --{} did not produce a run",
                    spec.long
                )
            };

            assert_eq!(
                invocation.config, expected,
                "schema default text `{default_text}` for --{} differs from FormatConfig::default()",
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
    fn docs_cover_schema_names_and_config_keys() {
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
        }
    }
}
