//! Fortran vocabularies, generated from the frozen Python reference.
//!
//! DO NOT EDIT BY HAND.  Regenerate with `python3 tools/gen_vocab.py`, which
//! reads `tools/reference/standardize_fortran.py`
//! (sha256 `8286229d8e11a8e46b50703c0706079d3c3a935edd9501a22798bbbdb8ed935e`).
//!
//! Everything here is lowercase and sorted so lookups are a branch-predictable
//! binary search with no allocation and no hash map in the hot path.

/// Case-insensitive membership test over one of the sorted tables below.
pub fn contains(table: &[&str], word: &[u8]) -> bool {
    lookup(table, word).is_some()
}

/// The canonical lowercase spelling of `word`, when the table holds it.
pub fn lookup<'a>(table: &'a [&'a str], word: &[u8]) -> Option<&'a str> {
    let index = table
        .binary_search_by(|entry| compare(entry.as_bytes(), word))
        .ok()?;
    Some(table[index])
}

/// The second element of a pair table, keyed case-insensitively by the first.
pub fn lookup_pair<'a>(table: &'a [(&'a str, &'a str)], word: &[u8]) -> Option<&'a str> {
    let index = table
        .binary_search_by(|entry| compare(entry.0.as_bytes(), word))
        .ok()?;
    Some(table[index].1)
}

/// Compare a lowercase table entry against an arbitrary-case word.
fn compare(entry: &[u8], word: &[u8]) -> core::cmp::Ordering {
    let mut left = entry.iter();
    let mut right = word.iter();
    loop {
        match (left.next(), right.next()) {
            (None, None) => return core::cmp::Ordering::Equal,
            (None, Some(_)) => return core::cmp::Ordering::Less,
            (Some(_), None) => return core::cmp::Ordering::Greater,
            (Some(a), Some(b)) => {
                let b = b.to_ascii_lowercase();
                if *a != b {
                    return a.cmp(&b);
                }
            }
        }
    }
}

/// Fortran 90-2018 language keywords, including multi-word statement components.
///
/// Sorted, lowercase, and looked up with [`contains`].
pub static FORTRAN_KEYWORDS: &[&str] = &[
    "abstract",
    "all",
    "allocatable",
    "allocate",
    "assign",
    "assigned",
    "assignment",
    "associate",
    "asynchronous",
    "backspace",
    "bind",
    "block",
    "blockdata",
    "call",
    "case",
    "change",
    "character",
    "class",
    "close",
    "codimension",
    "common",
    "complex",
    "concurrent",
    "contains",
    "contiguous",
    "continue",
    "critical",
    "cycle",
    "data",
    "deallocate",
    "default",
    "deferred",
    "dimension",
    "do",
    "double",
    "elemental",
    "else",
    "elseif",
    "elsewhere",
    "end",
    "endassociate",
    "endblock",
    "endblockdata",
    "enddo",
    "endfile",
    "endforall",
    "endfunction",
    "endif",
    "endinterface",
    "endmodule",
    "endprogram",
    "endselect",
    "endsubroutine",
    "endtype",
    "endwhere",
    "entry",
    "enum",
    "enumerator",
    "equivalence",
    "error",
    "event",
    "exit",
    "extends",
    "external",
    "fail",
    "final",
    "flush",
    "forall",
    "form",
    "format",
    "function",
    "generic",
    "go",
    "goto",
    "if",
    "image",
    "images",
    "implicit",
    "import",
    "impure",
    "in",
    "include",
    "inout",
    "inquire",
    "integer",
    "intent",
    "interface",
    "intrinsic",
    "kind",
    "local",
    "local_init",
    "lock",
    "logical",
    "memory",
    "module",
    "mold",
    "namelist",
    "non_intrinsic",
    "non_overridable",
    "non_recursive",
    "none",
    "nopass",
    "notify",
    "nullify",
    "only",
    "open",
    "operator",
    "optional",
    "out",
    "parameter",
    "pass",
    "pause",
    "pointer",
    "post",
    "print",
    "private",
    "procedure",
    "program",
    "protected",
    "public",
    "pure",
    "rank",
    "read",
    "real",
    "recursive",
    "reduce",
    "result",
    "return",
    "rewind",
    "rewrite",
    "save",
    "select",
    "sequence",
    "shared",
    "source",
    "stop",
    "submodule",
    "subroutine",
    "sync",
    "target",
    "team",
    "then",
    "to",
    "type",
    "unlock",
    "until_count",
    "use",
    "value",
    "volatile",
    "wait",
    "where",
    "while",
    "write",
];

/// I/O and statement specifiers such as `unit`, `iostat`, `status`.
///
/// Sorted, lowercase, and looked up with [`contains`].
pub static FORTRAN_SPECIFIERS: &[&str] = &[
    "access",
    "acquired_lock",
    "action",
    "advance",
    "blank",
    "decimal",
    "delim",
    "direct",
    "encoding",
    "eor",
    "err",
    "errmsg",
    "exist",
    "file",
    "fmt",
    "form",
    "formatted",
    "id",
    "iomsg",
    "iostat",
    "leading_zero",
    "name",
    "new_index",
    "newunit",
    "nextrec",
    "nml",
    "number",
    "opened",
    "pad",
    "pending",
    "pos",
    "position",
    "quiet",
    "readwrite",
    "rec",
    "recl",
    "round",
    "sequential",
    "sign",
    "size",
    "stat",
    "status",
    "stream",
    "unformatted",
    "unit",
];

/// Intrinsic procedures. Never override a locally declared identifier (I4).
///
/// Sorted, lowercase, and looked up with [`contains`].
pub static INTRINSIC_PROCEDURES: &[&str] = &[
    "abs",
    "acos",
    "allocated",
    "asin",
    "atan",
    "atan2",
    "ceiling",
    "cmplx",
    "conjg",
    "cos",
    "cosh",
    "cpu_time",
    "dim",
    "dot_product",
    "exp",
    "floor",
    "huge",
    "iand",
    "ibclr",
    "ibits",
    "ibset",
    "ieor",
    "index",
    "int",
    "is_iostat_end",
    "is_iostat_eor",
    "ishft",
    "iso_fortran_env",
    "lbound",
    "len",
    "len_trim",
    "log",
    "log10",
    "max",
    "maxloc",
    "maxval",
    "merge",
    "min",
    "minloc",
    "minval",
    "mod",
    "modulo",
    "nint",
    "pack",
    "precision",
    "product",
    "random_number",
    "repeat",
    "reshape",
    "sign",
    "sin",
    "sinh",
    "size",
    "sqrt",
    "tan",
    "tanh",
    "tiny",
    "trim",
    "ubound",
    "unpack",
    "verify",
];

/// Intrinsic procedures plus intrinsic module and type names.
///
/// Sorted, lowercase, and looked up with [`contains`].
pub static INTRINSIC_NAMES: &[&str] = &[
    "abs",
    "acos",
    "allocated",
    "and",
    "asin",
    "atan",
    "atan2",
    "ceiling",
    "cmplx",
    "conjg",
    "cos",
    "cosh",
    "cpu_time",
    "dim",
    "dot_product",
    "eq",
    "eqv",
    "exp",
    "false",
    "floor",
    "ge",
    "gt",
    "huge",
    "iand",
    "ibclr",
    "ibits",
    "ibset",
    "ieor",
    "index",
    "int",
    "is_iostat_end",
    "is_iostat_eor",
    "ishft",
    "iso_fortran_env",
    "lbound",
    "le",
    "len",
    "len_trim",
    "log",
    "log10",
    "lt",
    "max",
    "maxloc",
    "maxval",
    "merge",
    "min",
    "minloc",
    "minval",
    "mod",
    "modulo",
    "ne",
    "neqv",
    "nint",
    "not",
    "or",
    "pack",
    "precision",
    "product",
    "random_number",
    "repeat",
    "reshape",
    "sign",
    "sin",
    "sinh",
    "size",
    "sqrt",
    "tan",
    "tanh",
    "tiny",
    "trim",
    "true",
    "ubound",
    "unpack",
    "verify",
];

/// OpenMP directive and clause words.
///
/// Sorted, lowercase, and looked up with [`contains`].
pub static OPENMP_KEYWORDS: &[&str] = &[
    "allocate",
    "atomic",
    "barrier",
    "cancel",
    "cancellation",
    "collapse",
    "copyin",
    "copyprivate",
    "critical",
    "declare",
    "default",
    "defaultmap",
    "depend",
    "device",
    "dist_schedule",
    "distribute",
    "do",
    "dynamic",
    "end",
    "final",
    "firstprivate",
    "flush",
    "from",
    "grainsize",
    "guided",
    "hint",
    "if",
    "in_reduction",
    "is_device_ptr",
    "lastprivate",
    "linear",
    "loop",
    "map",
    "masked",
    "master",
    "mergeable",
    "nogroup",
    "nowait",
    "num_tasks",
    "num_threads",
    "omp",
    "order",
    "ordered",
    "parallel",
    "priority",
    "private",
    "proc_bind",
    "reduction",
    "runtime",
    "safelen",
    "schedule",
    "section",
    "sections",
    "shared",
    "simd",
    "simdlen",
    "single",
    "static",
    "target",
    "task",
    "taskgroup",
    "taskloop",
    "taskwait",
    "taskyield",
    "teams",
    "thread_limit",
    "threadprivate",
    "to",
    "use_device_addr",
    "use_device_ptr",
    "workshare",
];

/// Attributes admissible after a type specification, in canonical order.
///
/// Sorted, lowercase, and looked up with [`contains`].
pub static DECLARATION_ATTRIBUTES: &[&str] = &[
    "allocatable",
    "asynchronous",
    "codimension",
    "contiguous",
    "dimension",
    "external",
    "intent",
    "intrinsic",
    "optional",
    "parameter",
    "pointer",
    "private",
    "protected",
    "public",
    "save",
    "target",
    "value",
    "volatile",
];

/// Statements written `name(...)` whose keyword takes no space before the paren.
///
/// Sorted, lowercase, and looked up with [`contains`].
pub static PARENTHESIZED_STATEMENT_NAMES: &[&str] = &[
    "allocate",
    "allocated",
    "backspace",
    "close",
    "deallocate",
    "endfile",
    "flush",
    "inquire",
    "nullify",
    "open",
    "read",
    "rewind",
    "wait",
    "write",
];

/// Arithmetic operators the reference formatter writes without surrounding spaces.
///
/// Sorted, lowercase, and looked up with [`contains`].
pub static COMPACT_ARITHMETIC_OPERATORS: &[&str] = &["*", "**", "/"];

/// Run-together keyword spellings and their separated canonical form.
///
/// Sorted by the first element.
pub static COMPOUND_KEYWORDS: &[(&str, &str)] = &[
    ("blockdata", "block data"),
    ("elseif", "else if"),
    ("endassociate", "end associate"),
    ("endblock", "end block"),
    ("endblockdata", "end block data"),
    ("enddo", "end do"),
    ("endfile", "end file"),
    ("endforall", "end forall"),
    ("endfunction", "end function"),
    ("endif", "end if"),
    ("endinterface", "end interface"),
    ("endmodule", "end module"),
    ("endprogram", "end program"),
    ("endselect", "end select"),
    ("endsubroutine", "end subroutine"),
    ("endtype", "end type"),
    ("endwhere", "end where"),
];

/// Keyword pairs whose separating whitespace is normalized to one space.
///
/// Sorted by the first element.
pub static MULTIWORD_KEYWORD_PAIRS: &[(&str, &str)] = &[
    ("abstract", "interface"),
    ("change", "team"),
    ("class", "default"),
    ("class", "is"),
    ("do", "concurrent"),
    ("double", "precision"),
    ("event", "post"),
    ("event", "wait"),
    ("fail", "image"),
    ("form", "team"),
    ("impure", "elemental"),
    ("pure", "elemental"),
    ("rank", "default"),
    ("select", "case"),
    ("select", "rank"),
    ("sync", "all"),
    ("sync", "images"),
    ("sync", "memory"),
    ("sync", "team"),
    ("type", "default"),
    ("type", "is"),
];

/// Legacy relational operators (`.eq.`) and their modern spelling.
///
/// Sorted by the first element.
pub static MODERN_OPERATOR: &[(&str, &str)] = &[
    ("eq", "=="),
    ("ge", ">="),
    ("gt", ">"),
    ("le", "<="),
    ("lt", "<"),
    ("ne", "/="),
];

/// The reference formatter's default line-length budget.
pub const MAX_LINE_LENGTH: usize = 120;

/// A wrapped line must fill at least this fraction of its budget, otherwise the
/// break point is rejected as leaving too much whitespace.
pub const MINIMUM_BREAK_FILL: f64 = 0.25;

/// Free-form source extensions, lowercase.  Uppercase spellings are accepted too.
pub static SOURCE_EXTENSIONS: &[&str] = &[".f03", ".f08", ".f18", ".f23", ".f90", ".f95"];
