/// What the formatter is allowed to change.
///
/// `IndentOnly` is the findent 4.3.7 contract and stays byte-exact forever
/// (I6).  `Full` adds normalization and wrapping.  `NormalizeOnly` runs the
/// text passes without the structural layout, which is how a single
/// normalization rule is compared against the frozen Python reference while the
/// port is incomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormatMode {
    #[default]
    IndentOnly,
    NormalizeOnly,
    Full,
}

impl FormatMode {
    pub fn normalizes(self) -> bool {
        matches!(self, FormatMode::NormalizeOnly | FormatMode::Full)
    }

    pub fn lays_out(self) -> bool {
        matches!(self, FormatMode::IndentOnly | FormatMode::Full)
    }
}

/// Line-length policy for the reflow engine.
///
/// `line_length` is a budget, not a guarantee: a statement with no safe break
/// point is emitted long and reported by the corpus check rather than split
/// unsafely (I5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapConfig {
    pub enabled: bool,
    pub line_length: usize,
}

impl Default for WrapConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            line_length: 120,
        }
    }
}

/// A `-D NAME[=VALUE]` definition.  Macro names outrank every other case rule
/// (I4), so this list is part of the case configuration, not just of any CPP
/// evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroDefine {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatConfig {
    /// Selects which stages of the pipeline run.  Everything below this field
    /// is either shared or specific to one stage.
    pub mode: FormatMode,
    /// Full-mode reflow policy.
    pub wrap: WrapConfig,
    /// Command-line macro definitions, in the order given.
    pub defines: Vec<MacroDefine>,
    /// Uppercase a lone `l` used as a name, a Python-side option retained for
    /// compatibility with the reference formatter.
    pub uppercase_single_l: bool,
    pub indent: usize,
    pub apply_indent: bool,
    pub start_indent: usize,
    pub auto_start_indent: bool,
    pub max_indent: usize,
    pub label_left: bool,
    pub include_left: bool,
    pub indent_continuation: bool,
    pub continuation_indent: usize,
    pub indent_ampersand: bool,
    /// Whether parenthesis alignment is enabled.  `align_paren_value` keeps
    /// the optional numeric CLI value without breaking boolean API callers.
    pub align_paren: bool,
    pub align_paren_value: usize,
    pub openmp: bool,
    pub contains_restart: bool,
    pub contains_indent: usize,
    pub case_indent: usize,
    pub entry_indent: usize,
    pub refactor_end: bool,
    pub uppercase_end: bool,
    /// Whether redundant-whitespace reduction is enabled.  The numeric mode
    /// is retained in `ws_remred_value` for the optional CLI contract.
    pub ws_remred: bool,
    pub ws_remred_value: usize,
    pub last_indent: bool,
    pub last_usable: bool,
    pub construct_indents: ConstructIndents,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstructIndents {
    pub associate: usize,
    pub block: usize,
    pub changeteam: usize,
    pub critical: usize,
    pub do_: usize,
    pub r#enum: usize,
    pub forall: usize,
    pub if_: usize,
    pub interface: usize,
    pub module: usize,
    pub procedure: usize,
    pub select: usize,
    pub r#type: usize,
    pub where_: usize,
}

impl Default for FormatConfig {
    fn default() -> Self {
        Self {
            // The port keeps findent's contract as the effective default; the
            // flip to `Full` is one reviewable commit at cutover (Phase 12).
            mode: FormatMode::IndentOnly,
            wrap: WrapConfig::default(),
            defines: Vec::new(),
            uppercase_single_l: false,
            indent: 3,
            apply_indent: true,
            start_indent: 0,
            auto_start_indent: false,
            max_indent: 100,
            label_left: true,
            include_left: false,
            indent_continuation: true,
            continuation_indent: 3,
            indent_ampersand: false,
            align_paren: false,
            align_paren_value: 0,
            openmp: true,
            contains_restart: false,
            contains_indent: 3,
            case_indent: 2,
            entry_indent: 2,
            refactor_end: false,
            uppercase_end: false,
            ws_remred: false,
            ws_remred_value: 0,
            last_indent: false,
            last_usable: false,
            construct_indents: ConstructIndents::with_indent(3),
        }
    }
}

impl ConstructIndents {
    pub const fn with_indent(n: usize) -> Self {
        Self {
            associate: n,
            block: n,
            changeteam: n,
            critical: n,
            do_: n,
            r#enum: n,
            forall: n,
            if_: n,
            interface: n,
            module: n,
            procedure: n,
            select: n,
            r#type: n,
            where_: n,
        }
    }
    pub fn set_all(&mut self, n: usize) {
        *self = Self::with_indent(n);
    }
}
