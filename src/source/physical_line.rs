use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Newline {
    Lf,
    CrLf,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalLineKind {
    Blank,
    Comment,
    Code,
    Preprocessor,
    FindentFix,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalLine {
    pub span: Range<u32>,
    pub newline: Newline,
    pub kind: PhysicalLineKind,
    pub code_span: Range<u32>,
    pub comment_span: Option<Range<u32>>,
    /// Whether this is free-form conditional-compilation source introduced by
    /// a `!$` sentinel.
    ///
    /// The field keeps its historical `omp` name for public-API compatibility;
    /// formatter code should prefer [`PhysicalLine::is_conditional_compilation`]
    /// when the distinction from OpenMP directives matters.
    pub omp: bool,
}

impl PhysicalLine {
    /// Whether this line belongs to the free-form conditional-compilation
    /// stream introduced by a `!$` sentinel.
    pub fn is_conditional_compilation(&self) -> bool {
        self.omp
    }
}
