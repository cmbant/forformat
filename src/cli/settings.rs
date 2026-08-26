use super::{
    options::{spec_for_id, ConfigPhase, Construct, OptionId},
    ContextPath,
};
use crate::{
    config::{FormatConfig, FormatMode, FortranStandard, KeywordCase, MacroDefine},
    error::FormatError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FormatSetting {
    DisableIndent,
    SetIndent(usize),
    StartIndentAuto,
    SetStartIndent(usize),
    RestartContains,
    SetContainsIndent(usize),
    ConstructIndent(Construct, usize),
    IncludeLeft(bool),
    LabelLeft(bool),
    MaxIndent(usize),
    Openmp(bool),
    IndentAmpersand(bool),
    DisableContinuationIndent,
    EnableDefaultContinuationIndent,
    SetContinuationIndent(usize),
    AlignParen(usize),
    WhitespaceReduction(usize),
    AlignDeclarations(bool),
    AlignComments(bool),
    Mode(FormatMode),
    Wrap(bool),
    Rewrap(bool),
    LineLength(usize),
    TargetStandard(FortranStandard),
    UppercaseSingleL(bool),
    Define(MacroDefine),
    KeywordCase(KeywordCase),
    OpenmpCase(bool),
    RelationalSymbols(bool),
    ArrayBrackets(bool),
    CompactMultiplicative(bool),
    JoinGoto(bool),
    SplitCompoundKeywords(bool),
    StripEmptyArgs(bool),
    RemoveRedundantParens(bool),
    NormalizeSemicolons(bool),
    RemoveTerminalReturn(bool),
    ProgramUnitSpacing(bool),
    MaxBlankLines(Option<usize>),
    DelimiterSpacing(bool),
    CommentSpacing(bool),
    ContinuationMarkers(bool),
    RefactorEnd { enabled: bool, uppercase: bool },
}

impl FormatSetting {
    fn apply(&self, config: &mut FormatConfig) {
        match self {
            Self::DisableIndent => config.apply_indent = false,
            Self::SetIndent(value) => {
                config.apply_indent = true;
                config.set_indent(*value);
            }
            Self::StartIndentAuto => config.auto_start_indent = true,
            Self::SetStartIndent(value) => {
                config.start_indent = *value;
                config.auto_start_indent = false;
            }
            Self::RestartContains => config.contains_restart = true,
            Self::SetContainsIndent(value) => {
                config.contains_indent = *value;
                config.contains_restart = false;
            }
            Self::ConstructIndent(construct, value) => apply_construct(*construct, config, *value),
            Self::IncludeLeft(value) => config.include_left = *value,
            Self::LabelLeft(value) => config.label_left = *value,
            Self::MaxIndent(value) => config.max_indent = *value,
            Self::Openmp(value) => config.openmp = *value,
            Self::IndentAmpersand(value) => config.indent_ampersand = *value,
            Self::DisableContinuationIndent => config.indent_continuation = false,
            Self::EnableDefaultContinuationIndent => config.indent_continuation = true,
            Self::SetContinuationIndent(value) => {
                config.indent_continuation = true;
                config.continuation_indent = *value;
            }
            Self::AlignParen(value) => {
                config.align_paren_value = *value;
                config.align_paren = *value != 0;
            }
            Self::WhitespaceReduction(value) => {
                config.ws_remred_value = *value;
                config.ws_remred = *value != 0;
            }
            Self::AlignDeclarations(value) => config.align_declarations = *value,
            Self::AlignComments(value) => config.align_comments = *value,
            Self::Mode(value) => config.mode = *value,
            Self::Wrap(value) => config.wrap.enabled = *value,
            Self::Rewrap(value) => config.rewrap = *value,
            Self::LineLength(value) => config.wrap.line_length = *value,
            Self::TargetStandard(value) => config.target_standard = *value,
            Self::UppercaseSingleL(value) => config.uppercase_single_l = *value,
            Self::Define(value) => config.defines.push(value.clone()),
            Self::KeywordCase(value) => config.style.keyword_case = *value,
            Self::OpenmpCase(value) => config.style.openmp_case = *value,
            Self::RelationalSymbols(value) => config.style.relational_symbols = *value,
            Self::ArrayBrackets(value) => config.style.array_brackets = *value,
            Self::CompactMultiplicative(value) => config.style.compact_multiplicative = *value,
            Self::JoinGoto(value) => config.style.join_goto = *value,
            Self::SplitCompoundKeywords(value) => config.style.split_compound_keywords = *value,
            Self::StripEmptyArgs(value) => config.style.strip_empty_args = *value,
            Self::RemoveRedundantParens(value) => config.style.remove_redundant_parens = *value,
            Self::NormalizeSemicolons(value) => config.style.normalize_semicolons = *value,
            Self::RemoveTerminalReturn(value) => config.style.remove_terminal_return = *value,
            Self::ProgramUnitSpacing(value) => config.style.program_unit_spacing = *value,
            Self::MaxBlankLines(value) => config.style.max_blank_lines = *value,
            Self::DelimiterSpacing(value) => config.style.delimiter_spacing = *value,
            Self::CommentSpacing(value) => config.style.comment_spacing = *value,
            Self::ContinuationMarkers(value) => config.style.continuation_markers = *value,
            Self::RefactorEnd { enabled, uppercase } => {
                config.refactor_end = *enabled;
                config.uppercase_end = *uppercase;
            }
        }
    }
}

fn apply_construct(construct: Construct, config: &mut FormatConfig, value: usize) {
    match construct {
        Construct::Associate => config.construct_indents.associate = value,
        Construct::Block => config.construct_indents.block = value,
        Construct::Case => config.case_indent = value,
        Construct::Changeteam => config.construct_indents.changeteam = value,
        Construct::Critical => config.construct_indents.critical = value,
        Construct::Do => config.construct_indents.do_ = value,
        Construct::Entry => config.entry_indent = value,
        Construct::Enum => config.construct_indents.r#enum = value,
        Construct::Forall => config.construct_indents.forall = value,
        Construct::If => config.construct_indents.if_ = value,
        Construct::Interface => config.construct_indents.interface = value,
        Construct::Module => config.construct_indents.module = value,
        Construct::Procedure => config.construct_indents.procedure = value,
        Construct::Select => config.construct_indents.select = value,
        Construct::Type => config.construct_indents.r#type = value,
        Construct::Where => config.construct_indents.where_ = value,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LayerSetting {
    id: OptionId,
    setting: FormatSetting,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct OptionLayer {
    format: Vec<LayerSetting>,
    pub(crate) no_submodules: Option<bool>,
    pub(crate) context_paths: Option<Vec<ContextPath>>,
    pub(crate) exclude: Option<Vec<String>>,
    pub(crate) extend_exclude: Vec<String>,
    pub(crate) force_free_input: Option<bool>,
}

impl OptionLayer {
    pub(crate) fn push_format(&mut self, id: OptionId, setting: FormatSetting) {
        self.format.push(LayerSetting { id, setting });
    }

    pub(crate) fn push_context_path(&mut self, path: ContextPath) {
        self.context_paths.get_or_insert_with(Vec::new).push(path);
    }

    pub(crate) fn push_exclude(&mut self, pattern: String) {
        self.exclude.get_or_insert_with(Vec::new).push(pattern);
    }

    pub(crate) fn apply_cli(&self, config: &mut FormatConfig) {
        for setting in &self.format {
            setting.setting.apply(config);
        }
    }

    pub(crate) fn apply_config(&self, config: &mut FormatConfig) {
        for phase in [ConfigPhase::Baseline, ConfigPhase::Specific] {
            for setting in &self.format {
                if spec_for_id(setting.id).config_phase == phase {
                    setting.setting.apply(config);
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.format.is_empty()
            && self.no_submodules.is_none()
            && self.context_paths.as_ref().is_none_or(Vec::is_empty)
            && self.exclude.as_ref().is_none_or(Vec::is_empty)
            && self.extend_exclude.is_empty()
            && self.force_free_input.is_none()
    }
}

pub(crate) fn parse_format_setting(
    id: OptionId,
    value: Option<&str>,
) -> Result<Option<FormatSetting>, FormatError> {
    let setting = match id {
        OptionId::Indent => match required(value)? {
            "none" => FormatSetting::DisableIndent,
            value => FormatSetting::SetIndent(parse_num(value)?),
        },
        OptionId::StartIndent => {
            let value = required(value)?;
            if value.eq_ignore_ascii_case("a") || value.eq_ignore_ascii_case("auto") {
                FormatSetting::StartIndentAuto
            } else {
                FormatSetting::SetStartIndent(parse_num(value)?)
            }
        }
        OptionId::IndentContains => match required(value)? {
            "restart" => FormatSetting::RestartContains,
            value => FormatSetting::SetContainsIndent(parse_num(value)?),
        },
        OptionId::IndentConstruct(construct) => {
            FormatSetting::ConstructIndent(construct, parse_num(required(value)?)?)
        }
        OptionId::IncludeLeft => FormatSetting::IncludeLeft(parse_bool(required(value)?)?),
        OptionId::LabelLeft => FormatSetting::LabelLeft(parse_bool(required(value)?)?),
        OptionId::MaxIndent => FormatSetting::MaxIndent(parse_num(required(value)?)?),
        OptionId::Openmp => FormatSetting::Openmp(parse_bool(required(value)?)?),
        OptionId::IndentAmpersand => FormatSetting::IndentAmpersand(parse_optional_bool(value)?),
        OptionId::IndentContinuation => match required(value)? {
            "none" | "-" => FormatSetting::DisableContinuationIndent,
            "default" | "d" => FormatSetting::EnableDefaultContinuationIndent,
            value => FormatSetting::SetContinuationIndent(parse_num(value)?),
        },
        OptionId::AlignParen => FormatSetting::AlignParen(parse_optional_level(value)?),
        OptionId::WsRemred => {
            FormatSetting::WhitespaceReduction(parse_whitespace_reduction(value)?)
        }
        OptionId::AlignDeclarations => {
            FormatSetting::AlignDeclarations(parse_bool(required(value)?)?)
        }
        OptionId::AlignComments => FormatSetting::AlignComments(parse_bool(required(value)?)?),
        OptionId::IndentOnly => FormatSetting::Mode(FormatMode::IndentOnly),
        OptionId::Full => FormatSetting::Mode(FormatMode::Full),
        OptionId::NormalizeOnly => FormatSetting::Mode(FormatMode::NormalizeOnly),
        OptionId::CanonicalizeOnly => FormatSetting::Mode(FormatMode::CanonicalizeOnly),
        OptionId::CanonicalizeAndIndent => FormatSetting::Mode(FormatMode::CanonicalizeAndIndent),
        OptionId::Wrap => FormatSetting::Wrap(parse_optional_bool(value)?),
        OptionId::NoWrap => FormatSetting::Wrap(!parse_optional_bool(value)?),
        OptionId::Rewrap => FormatSetting::Rewrap(parse_optional_bool(value)?),
        OptionId::LineLength => FormatSetting::LineLength(parse_num(required(value)?)?),
        OptionId::TargetStandard => FormatSetting::TargetStandard(parse_style_choice(
            "target-standard",
            required(value)?,
            &[
                ("f95", FortranStandard::F95),
                ("f2003", FortranStandard::F2003),
                ("f2008", FortranStandard::F2008),
                ("f2018", FortranStandard::F2018),
                ("f2023", FortranStandard::F2023),
            ],
        )?),
        OptionId::UppercaseSingleL => FormatSetting::UppercaseSingleL(parse_optional_bool(value)?),
        OptionId::Define => {
            let Some(define) = parse_define(required(value)?) else {
                return Ok(None);
            };
            FormatSetting::Define(define)
        }
        OptionId::KeywordCase => FormatSetting::KeywordCase(parse_style_choice(
            "keyword-case",
            required(value)?,
            &[
                ("lower", KeywordCase::Lower),
                ("upper", KeywordCase::Upper),
                ("preserve", KeywordCase::Preserve),
            ],
        )?),
        OptionId::OpenmpCase => FormatSetting::OpenmpCase(parse_bool(required(value)?)?),
        OptionId::RelationalSymbols => {
            FormatSetting::RelationalSymbols(parse_bool(required(value)?)?)
        }
        OptionId::ArrayBrackets => FormatSetting::ArrayBrackets(parse_bool(required(value)?)?),
        OptionId::CompactMultiplicative => {
            FormatSetting::CompactMultiplicative(parse_bool(required(value)?)?)
        }
        OptionId::JoinGoto => FormatSetting::JoinGoto(parse_bool(required(value)?)?),
        OptionId::SplitCompoundKeywords => {
            FormatSetting::SplitCompoundKeywords(parse_bool(required(value)?)?)
        }
        OptionId::StripEmptyArgs => FormatSetting::StripEmptyArgs(parse_bool(required(value)?)?),
        OptionId::RemoveRedundantParens => {
            FormatSetting::RemoveRedundantParens(parse_bool(required(value)?)?)
        }
        OptionId::NormalizeSemicolons => {
            FormatSetting::NormalizeSemicolons(parse_bool(required(value)?)?)
        }
        OptionId::RemoveTerminalReturn => {
            FormatSetting::RemoveTerminalReturn(parse_bool(required(value)?)?)
        }
        OptionId::ProgramUnitSpacing => {
            FormatSetting::ProgramUnitSpacing(parse_bool(required(value)?)?)
        }
        OptionId::MaxBlankLines => {
            let value = required(value)?;
            FormatSetting::MaxBlankLines(if value == "preserve" {
                None
            } else {
                Some(parse_num(value)?)
            })
        }
        OptionId::DelimiterSpacing => {
            FormatSetting::DelimiterSpacing(parse_bool(required(value)?)?)
        }
        OptionId::CommentSpacing => FormatSetting::CommentSpacing(parse_bool(required(value)?)?),
        OptionId::ContinuationMarkers => {
            FormatSetting::ContinuationMarkers(parse_bool(required(value)?)?)
        }
        OptionId::RefactorEnd => {
            let (enabled, uppercase) = match value {
                None => (true, false),
                Some("upcase") => (true, true),
                Some(value) => (parse_bool(value)?, false),
            };
            FormatSetting::RefactorEnd { enabled, uppercase }
        }
        OptionId::Config
        | OptionId::NoConfig
        | OptionId::LastIndent
        | OptionId::LastUsable
        | OptionId::All
        | OptionId::AllFiles
        | OptionId::NoSubmodules
        | OptionId::ContextPath
        | OptionId::ProjectContext
        | OptionId::Stdin
        | OptionId::Stdout
        | OptionId::Isolated
        | OptionId::Check
        | OptionId::Diff
        | OptionId::ShowFiles
        | OptionId::QueryFormat
        | OptionId::Exclude
        | OptionId::ExtendExclude
        | OptionId::InputFormat
        | OptionId::OutputFormat
        | OptionId::Help
        | OptionId::Version => return Ok(None),
    };
    Ok(Some(setting))
}

pub(crate) fn parse_num(value: &str) -> Result<usize, FormatError> {
    value
        .parse::<isize>()
        .ok()
        .filter(|value| *value >= 0)
        .map(|value| value as usize)
        .ok_or_else(|| {
            FormatError::InvalidOption(format!("expected non-negative integer, got {value}"))
        })
}

pub(crate) fn parse_bool(value: &str) -> Result<bool, FormatError> {
    match value {
        "0" | "false" | "no" => Ok(false),
        "1" | "true" | "yes" => Ok(true),
        _ => Err(FormatError::InvalidOption(format!(
            "expected boolean (0/1, true/false, yes/no), got {value}"
        ))),
    }
}

pub(crate) fn parse_optional_bool(value: Option<&str>) -> Result<bool, FormatError> {
    value
        .map(parse_bool)
        .transpose()
        .map(|value| value.unwrap_or(true))
}

pub(crate) fn parse_input_format(value: &str) -> Result<bool, FormatError> {
    match value.to_ascii_lowercase().as_str() {
        "free" => Ok(true),
        "auto" => Ok(false),
        "fixed" => Err(FormatError::Unsupported(
            "fixed-form input/output is not supported".into(),
        )),
        other => Err(FormatError::InvalidOption(format!(
            "--input-format={other}"
        ))),
    }
}

pub(crate) fn parse_output_format(value: &str) -> Result<(), FormatError> {
    match value.to_ascii_lowercase().as_str() {
        "free" | "same" => Ok(()),
        "fixed" => Err(FormatError::Unsupported(
            "fixed-form input/output is not supported".into(),
        )),
        other => Err(FormatError::InvalidOption(format!(
            "--output-format={other}"
        ))),
    }
}

fn required(value: Option<&str>) -> Result<&str, FormatError> {
    value.ok_or_else(|| FormatError::InvalidOption("missing option value".into()))
}

fn parse_define(spec: &str) -> Option<MacroDefine> {
    let (name, value) = match spec.split_once('=') {
        Some((name, value)) => (name, Some(value.to_string())),
        None => (spec, None),
    };
    (!name.is_empty()).then(|| MacroDefine {
        name: name.to_string(),
        value,
    })
}

fn parse_style_choice<T: Copy>(
    option: &str,
    value: &str,
    choices: &[(&str, T)],
) -> Result<T, FormatError> {
    choices
        .iter()
        .find(|(allowed, _)| *allowed == value)
        .map(|(_, parsed)| *parsed)
        .ok_or_else(|| {
            let allowed = choices
                .iter()
                .map(|(allowed, _)| *allowed)
                .collect::<Vec<_>>()
                .join(", ");
            FormatError::InvalidOption(format!(
                "--{option} has invalid value `{value}`; allowed values: {allowed}"
            ))
        })
}

fn parse_optional_level(value: Option<&str>) -> Result<usize, FormatError> {
    match value {
        None | Some("true") => Ok(1),
        Some("false") => Ok(0),
        Some(value) => parse_num(value),
    }
}

fn parse_whitespace_reduction(value: Option<&str>) -> Result<usize, FormatError> {
    match value {
        None | Some("true") => Ok(1),
        Some("false") => Ok(0),
        Some(value) => parse_num(value),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_bool, parse_format_setting, FormatSetting};
    use crate::{
        cli::{options::OptionId, parse, Command},
        config::FortranStandard,
    };
    use std::fs;

    fn run(args: &[&str]) -> crate::config::FormatConfig {
        let argv = std::iter::once("forformat")
            .chain(args.iter().copied())
            .map(str::to_owned);
        let Command::Run(invocation) = parse(argv).unwrap() else {
            panic!("expected a formatting command")
        };
        invocation.config
    }

    #[test]
    fn shared_value_parser_covers_updated_main_grammar() {
        assert_eq!(
            parse_format_setting(OptionId::AlignParen, Some("true")).unwrap(),
            Some(FormatSetting::AlignParen(1))
        );
        assert_eq!(
            parse_format_setting(OptionId::AlignParen, Some("false")).unwrap(),
            Some(FormatSetting::AlignParen(0))
        );
        assert_eq!(
            parse_format_setting(OptionId::TargetStandard, Some("f95")).unwrap(),
            Some(FormatSetting::TargetStandard(FortranStandard::F95))
        );
    }

    #[test]
    fn later_numeric_indent_values_restore_their_modes() {
        let config = run(&[
            "--no-config",
            "--indent=none",
            "--indent=4",
            "--indent-continuation=none",
            "--indent-continuation=5",
            "--indent-contains=restart",
            "--indent-contains=6",
        ]);
        assert!(config.apply_indent);
        assert_eq!(config.indent, 4);
        assert!(config.indent_continuation);
        assert_eq!(config.continuation_indent, 5);
        assert!(!config.contains_restart);
        assert_eq!(config.contains_indent, 6);

        let config = run(&[
            "--no-config",
            "--indent=4",
            "--indent=none",
            "--indent-continuation=5",
            "--indent-continuation=none",
            "--indent-contains=6",
            "--indent-contains=restart",
        ]);
        assert!(!config.apply_indent);
        assert!(!config.indent_continuation);
        assert!(config.contains_restart);
    }

    #[test]
    fn long_and_short_contains_options_have_the_same_last_wins_semantics() {
        let long = run(&[
            "--no-config",
            "--indent-contains=restart",
            "--indent-contains=4",
        ]);
        let short = run(&["--no-config", "-C-", "-C4"]);
        assert_eq!(long.contains_restart, short.contains_restart);
        assert_eq!(long.contains_indent, short.contains_indent);
        assert!(!long.contains_restart);
        assert_eq!(long.contains_indent, 4);
    }

    #[test]
    fn cli_numeric_indent_values_override_disabled_toml_modes() {
        let path = std::env::temp_dir().join(format!(
            "forformat-state-override-{}.toml",
            std::process::id()
        ));
        fs::write(
            &path,
            "indent = 'none'\nindent_continuation = 'none'\nindent_contains = 'restart'\n",
        )
        .unwrap();
        let config_arg = format!("--config={}", path.display());
        let config = run(&[
            &config_arg,
            "--indent=4",
            "--indent-continuation=5",
            "--indent-contains=6",
        ]);
        let _ = fs::remove_file(&path);

        assert!(config.apply_indent);
        assert_eq!(config.indent, 4);
        assert!(config.indent_continuation);
        assert_eq!(config.continuation_indent, 5);
        assert!(!config.contains_restart);
        assert_eq!(config.contains_indent, 6);
    }

    #[test]
    fn boolean_parse_error_lists_all_accepted_spellings() {
        let error = parse_bool("maybe").unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid option: expected boolean (0/1, true/false, yes/no), got maybe"
        );
    }
}
