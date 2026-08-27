mod file;

pub(crate) use file::{config_args, ConfigArguments};

/// What the formatter is allowed to change.
///
/// `Full` is the product default and adds normalization and wrapping.
/// `IndentOnly` is the findent 4.3.8~pre01 contract and stays byte-exact forever
/// (I6).  `NormalizeOnly` runs the text passes without the structural layout,
/// which is how a single normalization rule can be tested independently of
/// structural layout.  `CanonicalizeOnly` is `NormalizeOnly` minus presentation
/// whitespace: token and spelling canonicalization without reflowing the
/// author's spacing. `CanonicalizeAndIndent` composes that canonicalization
/// policy with the existing indent-only layout engine, without wrapping or
/// post-layout presentation passes.
///
/// The five modes are one field on purpose.  Canonicalization used to be
/// `NormalizeOnly` plus a separate `style.normalize_whitespace = false`, which
/// made `--canonicalize-only --normalize-only` depend on argument order — the
/// second option reset the whitespace half of the first.  Whether whitespace is
/// presentation the formatter owns is a property of the mode, so it is answered
/// by [`FormatMode::normalizes_whitespace`] and cannot disagree with `mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatMode {
    IndentOnly,
    NormalizeOnly,
    CanonicalizeOnly,
    CanonicalizeAndIndent,
    Full,
}

/// Highest Fortran language revision that formatting may introduce.
///
/// This is an output-generation policy, not a validator or downgrader: source
/// that already uses newer syntax is left to the ordinary formatter rules. The
/// default is Fortran 2003 so existing full-mode output remains unchanged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum FortranStandard {
    F95,
    #[default]
    F2003,
    F2008,
    F2018,
    F2023,
}

impl FortranStandard {
    /// Whether formatting may modernize `(/ ... /)` into the square-bracket
    /// array-constructor syntax introduced in Fortran 2003.
    pub const fn allows_square_array_constructors(self) -> bool {
        !matches!(self, FortranStandard::F95)
    }
}

/// Each predicate below names one question the pipeline asks of the mode, and
/// is the only place that question is answered.  Several of them currently
/// select the same single variant, and that is deliberate rather than
/// redundant: they mean different things, so a sixth mode would answer them
/// differently, and a caller written as `mode == FormatMode::Full` would have
/// silently picked whichever meaning it happened to be next to.  Ask the
/// question, not the variant.
impl FormatMode {
    /// Whether the normalization pipeline runs at all.
    ///
    /// The mode that says no is the byte-exact findent path (I6): it goes
    /// straight to the layout engine, needs no analysis and no project context,
    /// and rewrites no byte that is not leading or trailing whitespace.
    pub fn normalizes(self) -> bool {
        matches!(
            self,
            FormatMode::NormalizeOnly
                | FormatMode::CanonicalizeOnly
                | FormatMode::CanonicalizeAndIndent
                | FormatMode::Full
        )
    }

    /// Whether the layout engine chooses this mode's columns.
    pub fn lays_out(self) -> bool {
        matches!(
            self,
            FormatMode::IndentOnly | FormatMode::CanonicalizeAndIndent | FormatMode::Full
        )
    }

    /// Whether the post-layout alignment passes run after the emitter.
    ///
    /// The whitespace reducer protects an authored gap before a `::` or a
    /// trailing `!` only when one of those passes will afterwards decide its
    /// real column.  Protecting it in a mode where nothing follows would leave
    /// the gap merely un-collapsed, which is not what `--ws-remred` means.
    pub fn aligns_after_layout(self) -> bool {
        matches!(self, FormatMode::Full)
    }

    /// Whether presentation whitespace belongs to the formatter in this mode.
    ///
    /// The canonicalization modes say no: they keep authored interior spacing
    /// and line structure while still canonicalizing tokens and spellings.
    /// `CanonicalizeAndIndent` then changes only the leading/trailing whitespace
    /// owned by the indent-only engine. Whitespace at end of line is invisible
    /// rather than a formatting choice, and every mode removes it.
    pub fn normalizes_whitespace(self) -> bool {
        !matches!(
            self,
            FormatMode::CanonicalizeOnly | FormatMode::CanonicalizeAndIndent
        )
    }

    /// Whether the reflow wrapper runs, and therefore whether `rewrap` can mean
    /// anything.
    pub fn wraps(self) -> bool {
        matches!(self, FormatMode::Full)
    }
}

/// Line-length policy for the reflow engine.
///
/// `line_length` is a budget, not a guarantee: a statement with no safe break
/// point is emitted long and reported by a decline diagnostic rather than split
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

/// Numeric setting whose level also acts as its enable switch.
///
/// Zero disables the feature; positive values enable it while retaining the
/// exact optional numeric CLI value. Keeping both facts in one value makes
/// contradictory enabled/value states unrepresentable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FeatureLevel(usize);

impl FeatureLevel {
    pub const fn new(value: usize) -> Self {
        Self(value)
    }

    pub const fn enabled(self) -> bool {
        self.0 != 0
    }

    pub const fn value(self) -> usize {
        self.0
    }
}

impl From<usize> for FeatureLevel {
    fn from(value: usize) -> Self {
        Self::new(value)
    }
}

impl From<bool> for FeatureLevel {
    fn from(enabled: bool) -> Self {
        Self::new(enabled as usize)
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

/// Case policy for recognized Fortran keywords and intrinsic spellings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeywordCase {
    Lower,
    Upper,
    Preserve,
}

/// Opinionated full-mode normalization choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleConfig {
    pub keyword_case: KeywordCase,
    /// Uppercase reserved OpenMP sentinels (`!$OMP`, `!$OMPX`) and the
    /// directive keywords that follow them, independently of `keyword_case`.
    ///
    /// `!$OMP PARALLEL DO` in otherwise lowercase source is the near-universal
    /// convention, so this defaults on and a directive does not follow
    /// `keyword_case` unless it is turned off.  It governs reserved directives
    /// only: a conditional-compilation `!$ ` line is ordinary Fortran wearing a
    /// sentinel, so its body follows `keyword_case` like any other statement.
    pub openmp_case: bool,
    pub relational_symbols: bool,
    pub array_brackets: bool,
    /// Insert `::` in recognized declaration forms whose legacy spelling
    /// permits the separator to be omitted.
    pub modernize_declarations: bool,
    pub compact_multiplicative: bool,
    pub join_goto: bool,
    pub split_compound_keywords: bool,
    pub strip_empty_args: bool,
    pub remove_redundant_parens: bool,
    /// Drop semicolons that separate no pair of non-empty statements.
    pub normalize_semicolons: bool,
    pub remove_terminal_return: bool,
    pub program_unit_spacing: bool,
    pub max_blank_lines: Option<usize>,
    pub delimiter_spacing: bool,
    pub comment_spacing: bool,
    pub continuation_markers: bool,
}

impl StyleConfig {
    /// Case policy for a reserved OpenMP sentinel and its directive keywords.
    ///
    /// `openmp_case` is a switch rather than its own [`KeywordCase`] because
    /// there is only one convention worth naming separately: uppercase
    /// directives over lowercase Fortran.  Turning it off hands directives back
    /// to `keyword_case`, which is also how `--keyword-case=preserve` reaches
    /// them.
    pub fn openmp_keyword_case(&self) -> KeywordCase {
        if self.openmp_case {
            KeywordCase::Upper
        } else {
            self.keyword_case
        }
    }
}

impl Default for StyleConfig {
    fn default() -> Self {
        Self {
            keyword_case: KeywordCase::Lower,
            openmp_case: true,
            relational_symbols: true,
            array_brackets: true,
            modernize_declarations: false,
            compact_multiplicative: true,
            join_goto: true,
            split_compound_keywords: true,
            strip_empty_args: true,
            remove_redundant_parens: true,
            normalize_semicolons: true,
            remove_terminal_return: true,
            program_unit_spacing: true,
            max_blank_lines: Some(2),
            delimiter_spacing: true,
            comment_spacing: true,
            continuation_markers: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatConfig {
    /// Selects which stages of the pipeline run.  Everything below this field
    /// is either shared or specific to one stage.
    pub mode: FormatMode,
    /// Highest Fortran revision syntax that formatting may introduce.
    pub target_standard: FortranStandard,
    /// Full-mode reflow policy.
    pub wrap: WrapConfig,
    /// Repack already-continued eligible statements through the normal wrapper.
    pub rewrap: bool,
    /// Command-line macro definitions, in the order given.
    pub defines: Vec<MacroDefine>,
    /// Full-mode lexical and structural style choices.
    pub style: StyleConfig,
    /// Uppercase a lone `l` used as a name, a Python-side option retained for
    /// compatibility with established command-line profiles.
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
    /// Parenthesis-alignment level; zero disables the feature.
    pub align_paren: FeatureLevel,
    pub openmp: bool,
    pub contains_restart: bool,
    pub contains_indent: usize,
    pub case_indent: usize,
    pub entry_indent: usize,
    pub refactor_end: bool,
    pub uppercase_end: bool,
    /// Redundant-whitespace-reduction level; zero disables the feature.
    pub ws_remred: FeatureLevel,
    pub construct_indents: ConstructIndents,
    /// Whether step 17 may shrink the whitespace before a declaration's `::`
    /// to fit a shared block column. Declarations are hand-aligned often
    /// enough that this defaults on.
    pub align_declarations: bool,
    /// Whether step 17b may shrink the whitespace before a trailing comment
    /// to fit a shared run column. Off by default: unlike a declaration's
    /// `::`, a comment's gap is not a separator with an owed minimum, so
    /// there is no default width to fall back to if the author's is not
    /// kept — shrinking it is a layout opinion this formatter does not
    /// impose unless asked.
    pub align_comments: bool,
}

impl FormatConfig {
    /// Apply the derived indentation defaults associated with `-i N` / `--indent N`.
    ///
    /// Individual construct/continuation options may still override these later;
    /// this only centralizes the shared baseline those two CLI spellings establish.
    pub(crate) fn set_indent(&mut self, indent: usize) {
        self.indent = indent;
        self.construct_indents.set_all(indent);
        self.contains_indent = indent;
        self.continuation_indent = indent;
        self.case_indent = indent.saturating_sub(indent / 2);
        self.entry_indent = self.case_indent;
    }

    /// Return the minimal effective override required by the output-language
    /// target, if any. Applying this after all user settings makes the standard
    /// gate independent of CLI/config ordering and also covers Rust API callers.
    pub(crate) fn target_standard_override(&self) -> Option<Self> {
        if self.style.array_brackets && !self.target_standard.allows_square_array_constructors() {
            let mut adjusted = self.clone();
            adjusted.style.array_brackets = false;
            Some(adjusted)
        } else {
            None
        }
    }
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
            mode: FormatMode::Full,
            target_standard: FortranStandard::default(),
            wrap: WrapConfig::default(),
            rewrap: false,
            defines: Vec::new(),
            style: StyleConfig::default(),
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
            align_paren: FeatureLevel::default(),
            openmp: true,
            contains_restart: false,
            contains_indent: 3,
            case_indent: 2,
            entry_indent: 2,
            refactor_end: false,
            uppercase_end: false,
            ws_remred: FeatureLevel::default(),
            construct_indents: ConstructIndents::with_indent(3),
            align_declarations: true,
            align_comments: false,
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
