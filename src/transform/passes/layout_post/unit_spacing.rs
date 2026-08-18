use crate::{
    classify::{classify, StatementInfo, StatementKind},
    config::FormatConfig,
    error::FormatError,
    format::preprocessor::{event as cpp_event, PreprocessorEvent},
    source::regions::comment_start,
    transform::{
        document::Document,
        passes::structure::{cpp_line_continues, is_preprocessor_line},
    },
};

/// The separator step 18 still owes the next line of code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Pending {
    #[default]
    None,
    Exactly,
    AtLeast {
        authored: usize,
    },
}

impl Pending {
    fn is_owed(self) -> bool {
        self != Self::None
    }

    fn count_authored_blank(&mut self) {
        if let Self::AtLeast { authored } = self {
            *authored += 1;
        }
    }

    fn width(self) -> usize {
        const MAX_SEPARATOR: usize = 2;
        match self {
            Self::None => 0,
            Self::Exactly => 1,
            Self::AtLeast { authored } => authored.clamp(1, MAX_SEPARATOR),
        }
    }
}

/// Step 18: blank-line policy around program units and `CONTAINS`.
///
/// Structural starts are classified by the same classifier used by the scope
/// and indentation machinery. END handling remains deliberately explicit here
/// because this pass has narrower program-unit semantics than a generic
/// definition close.
pub fn program_unit_spacing(
    document: &mut Document,
    config: &FormatConfig,
) -> Result<(), FormatError> {
    let _ = config;
    let mut normalized = Vec::with_capacity(document.lines.len());
    let mut unit_depth = 0usize;
    let mut type_depth = 0usize;
    let mut interface_depth = 0usize;
    let mut pending = Pending::default();
    let mut cpp_continuation = false;

    for line in &document.lines {
        let cpp_line = cpp_continuation || is_preprocessor_line(line);
        if cpp_line {
            let closing_cpp_block =
                !cpp_continuation && cpp_event(line) == PreprocessorEvent::EndIf;
            if pending.is_owed() && !cpp_continuation && !closing_cpp_block {
                if normalized
                    .last()
                    .is_some_and(|previous: &Vec<u8>| !previous.iter().all(u8::is_ascii_whitespace))
                {
                    for _ in 0..pending.width() {
                        normalized.push(Vec::new());
                    }
                }
                pending = Pending::None;
            }
            normalized.push(line.clone());
            cpp_continuation = cpp_line_continues(line);
            continue;
        }
        cpp_continuation = false;

        let code = code_context(line);
        let info = classify(code);
        let is_blank = line.iter().all(u8::is_ascii_whitespace);
        if pending.is_owed() {
            if is_blank {
                pending.count_authored_blank();
                continue;
            }
            for _ in 0..pending.width() {
                normalized.push(Vec::new());
            }
            pending = Pending::None;
        }

        if is_blank
            && (unit_depth > 0 || interface_depth > 0)
            && normalized
                .last()
                .is_some_and(|previous: &Vec<u8>| previous.iter().all(u8::is_ascii_whitespace))
        {
            continue;
        }

        if is_type_definition_end(code) {
            type_depth = type_depth.saturating_sub(1);
        } else if info.kind == StatementKind::Type {
            type_depth += 1;
        }

        let is_end = interface_depth == 0
            && is_program_unit_end(code)
            && !(type_depth > 0 && is_procedure_end(code));
        let is_header = !is_end && is_program_unit_header(&info, code, type_depth);
        if interface_depth == 0 && is_header {
            unit_depth += 1;
        }

        let is_contains = unit_depth > 0 && type_depth == 0 && info.kind == StatementKind::Contains;
        if is_contains || is_end {
            while normalized
                .last()
                .is_some_and(|previous: &Vec<u8>| previous.iter().all(u8::is_ascii_whitespace))
            {
                normalized.pop();
            }
            if !normalized.is_empty() {
                normalized.push(Vec::new());
            }
        }

        normalized.push(line.clone());
        if is_contains {
            pending = Pending::Exactly;
        }
        if is_end {
            pending = Pending::AtLeast { authored: 0 };
            unit_depth = unit_depth.saturating_sub(1);
        }
        if is_interface_end(code) {
            interface_depth = interface_depth.saturating_sub(1);
        } else if matches!(
            info.kind,
            StatementKind::Interface | StatementKind::AbstractInterface
        ) {
            interface_depth += 1;
        }
    }
    document.set_lines(normalized);
    Ok(())
}

fn is_program_unit_header(info: &StatementInfo, code: &[u8], type_depth: usize) -> bool {
    let named_unit = matches!(
        info.kind,
        StatementKind::Program
            | StatementKind::Module
            | StatementKind::Submodule
            | StatementKind::Subroutine
            | StatementKind::Function
    ) && info.entity_name.is_some();
    named_unit || (type_depth == 0 && is_module_procedure_header(code))
}

/// `module procedure` is the only `StatementKind::Procedure` that opens a unit
/// for this spacing pass; an ordinary `procedure name` remains a declaration.
fn is_module_procedure_header(code: &[u8]) -> bool {
    let code = trimmed(code);
    if !first_word(code).is_some_and(|word| word.eq_ignore_ascii_case(b"module")) {
        return false;
    }
    let first_len = first_word(code).map_or(0, <[u8]>::len);
    let rest = skip_ascii_whitespace(&code[first_len..]);
    first_word(rest).is_some_and(|word| word.eq_ignore_ascii_case(b"procedure"))
}

fn code_context(line: &[u8]) -> &[u8] {
    comment_start(line).map_or(line, |index| &line[..index])
}

fn trimmed(code: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = code.len();
    while start < end && code[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && code[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &code[start..end]
}

fn first_word(code: &[u8]) -> Option<&[u8]> {
    let code = trimmed(code);
    let end = code
        .iter()
        .position(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
        .unwrap_or(code.len());
    (end > 0).then_some(&code[..end])
}

fn skip_ascii_whitespace(code: &[u8]) -> &[u8] {
    let start = code
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(code.len());
    &code[start..]
}

fn is_program_unit_end(code: &[u8]) -> bool {
    let code = trimmed(code);
    if code.eq_ignore_ascii_case(b"end") {
        return true;
    }
    if !code
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"end"))
        || !code.get(3).is_some_and(u8::is_ascii_whitespace)
    {
        return false;
    }
    let rest = skip_ascii_whitespace(&code[4..]);
    let Some(word) = first_word(rest) else {
        return false;
    };
    if word.eq_ignore_ascii_case(b"block") {
        let rest = skip_ascii_whitespace(&rest[word.len()..]);
        return first_word(rest).is_some_and(|second| second.eq_ignore_ascii_case(b"data"));
    }
    word.eq_ignore_ascii_case(b"module")
        || word.eq_ignore_ascii_case(b"submodule")
        || word.eq_ignore_ascii_case(b"program")
        || word.eq_ignore_ascii_case(b"function")
        || word.eq_ignore_ascii_case(b"subroutine")
        || word.eq_ignore_ascii_case(b"procedure")
}

fn is_procedure_end(code: &[u8]) -> bool {
    let code = trimmed(code);
    if !code
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"end"))
    {
        return false;
    }
    let rest = skip_ascii_whitespace(&code[3..]);
    first_word(rest).is_some_and(|word| word.eq_ignore_ascii_case(b"procedure"))
}

fn is_type_definition_end(code: &[u8]) -> bool {
    let code = trimmed(code);
    code.get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"end"))
        && code.get(3..).is_some_and(|rest| {
            let rest = skip_ascii_whitespace(rest);
            first_word(rest).is_some_and(|word| word.eq_ignore_ascii_case(b"type"))
        })
}

fn is_interface_end(code: &[u8]) -> bool {
    let code = trimmed(code);
    if !code
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"end"))
    {
        return false;
    }
    let rest = skip_ascii_whitespace(&code[3..]);
    first_word(rest).is_some_and(|word| word.eq_ignore_ascii_case(b"interface"))
}
