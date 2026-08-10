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
    pub omp: bool,
}
