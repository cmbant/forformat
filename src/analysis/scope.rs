//! Program-unit and scope structure of one file.
//!
//! Declaration extraction and case resolution both need to know which program
//! unit a line belongs to, whether that unit is past its `CONTAINS`, and
//! whether an enclosing construct is an interface body (where a `MODULE
//! PROCEDURE` is a member, not a definition).
//!
//! The scope machine reuses the classifier rather than re-scanning text with
//! its own regexes: the recognizers already encode which statements open and
//! close a program unit, and that agreement is what keeps scopes and
//! indentation from drifting apart.

use crate::{
    classify::{StatementClass, StatementKind},
    transform::document::Analysis,
};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    /// The implicit outermost scope of the file.
    File,
    Module,
    Submodule,
    Program,
    /// A subroutine, function, block data unit, or separate module procedure.
    Procedure,
    /// An `INTERFACE` or `ABSTRACT INTERFACE` block: its bodies declare
    /// signatures, not definitions.
    Interface,
    /// A derived-type definition, whose members are components and type-bound
    /// procedures rather than variables.
    DerivedType,
    /// A `BLOCK` or `ASSOCIATE` construct.  It is not a program unit, but it
    /// does own declarations: a name declared in a `BLOCK` is invisible after
    /// the matching `END BLOCK`, so it must not be attributed to the host
    /// procedure the way an ordinary body declaration is.
    Construct,
}

impl ScopeKind {
    /// True for the scopes that own declarations of their own.
    pub fn is_program_unit(self) -> bool {
        matches!(
            self,
            ScopeKind::Module | ScopeKind::Submodule | ScopeKind::Program | ScopeKind::Procedure
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub kind: ScopeKind,
    pub name: Option<Vec<u8>>,
    pub parent: Option<usize>,
    /// Physical lines covered, from the header line through the `END` line.
    pub lines: Range<usize>,
    /// The line carrying this scope's `CONTAINS`, when it has one.  Everything
    /// before it is the specification part.
    pub contains_at: Option<usize>,
}

impl Scope {
    /// True when `line` is in this scope's specification part.
    pub fn is_specification(&self, line: usize) -> bool {
        self.contains_at.is_none_or(|contains| line < contains)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeTree {
    pub scopes: Vec<Scope>,
    /// The innermost scope owning each physical line.
    line_scope: Vec<usize>,
}

impl ScopeTree {
    /// Derive the scope structure of an analyzed document.
    pub fn build(analysis: &Analysis) -> Self {
        let line_count = analysis.buffer.lines.len();
        let mut scopes = vec![Scope {
            kind: ScopeKind::File,
            name: None,
            parent: None,
            lines: 0..line_count,
            contains_at: None,
        }];
        let mut line_scope = vec![0usize; line_count];
        let mut stack = vec![0usize];

        for (index, group) in analysis.groups.iter().enumerate() {
            let before = *stack.last().unwrap_or(&0);
            let mut opened: Option<usize> = None;
            for info in analysis.infos.get(index).into_iter().flatten() {
                let parent = *stack.last().unwrap_or(&0);
                if let Some(kind) = opening_kind(info.kind, scopes[parent].kind) {
                    scopes.push(Scope {
                        kind,
                        name: info.entity_name.clone(),
                        parent: Some(parent),
                        lines: group.lines.start..line_count,
                        contains_at: None,
                    });
                    let new = scopes.len() - 1;
                    stack.push(new);
                    opened = Some(new);
                    continue;
                }
                if info.kind == StatementKind::Contains {
                    if let Some(scope) = scopes.get_mut(parent) {
                        scope.contains_at.get_or_insert(group.lines.start);
                    }
                    continue;
                }
                if closes_scope(info.kind, info.class) && stack.len() > 1 {
                    let closed = stack.pop().expect("the file scope is never popped");
                    scopes[closed].lines.end = group.lines.end;
                }
            }
            let owner = opened.unwrap_or(before);
            for line in group.lines.clone() {
                if let Some(slot) = line_scope.get_mut(line) {
                    *slot = owner;
                }
            }
        }
        // A truncated file leaves scopes open; they run to the end of the file.
        for index in stack.into_iter().skip(1) {
            scopes[index].lines.end = line_count;
        }
        Self { scopes, line_scope }
    }

    pub fn scope_of_line(&self, line: usize) -> Option<&Scope> {
        self.scopes.get(*self.line_scope.get(line)?)
    }

    pub fn index_of_line(&self, line: usize) -> usize {
        self.line_scope.get(line).copied().unwrap_or(0)
    }

    /// Walk from a scope out to the file scope.
    pub fn ancestors(&self, mut index: usize) -> Vec<usize> {
        let mut chain = vec![index];
        while let Some(parent) = self.scopes.get(index).and_then(|scope| scope.parent) {
            chain.push(parent);
            index = parent;
        }
        chain
    }

    /// The innermost enclosing program unit of a line, skipping interface and
    /// derived-type scopes.
    pub fn program_unit_of_line(&self, line: usize) -> Option<&Scope> {
        self.ancestors(self.index_of_line(line))
            .into_iter()
            .map(|index| &self.scopes[index])
            .find(|scope| scope.kind.is_program_unit())
    }

    /// True when the line sits inside an interface body, where a declaration
    /// describes a signature rather than a definition.
    pub fn in_interface(&self, line: usize) -> bool {
        self.ancestors(self.index_of_line(line))
            .into_iter()
            .any(|index| self.scopes[index].kind == ScopeKind::Interface)
    }

    /// The derived type whose definition encloses this line, if any.
    pub fn enclosing_type(&self, line: usize) -> Option<&Scope> {
        self.ancestors(self.index_of_line(line))
            .into_iter()
            .map(|index| &self.scopes[index])
            .find(|scope| scope.kind == ScopeKind::DerivedType)
    }
}

/// The scope a statement opens, given the scope it appears in.
fn opening_kind(kind: StatementKind, parent: ScopeKind) -> Option<ScopeKind> {
    match kind {
        StatementKind::Module => Some(ScopeKind::Module),
        StatementKind::Submodule => Some(ScopeKind::Submodule),
        StatementKind::Program => Some(ScopeKind::Program),
        StatementKind::Subroutine | StatementKind::Function | StatementKind::BlockData => {
            Some(ScopeKind::Procedure)
        }
        StatementKind::Interface | StatementKind::AbstractInterface => Some(ScopeKind::Interface),
        StatementKind::Type => Some(ScopeKind::DerivedType),
        StatementKind::Block | StatementKind::Associate => Some(ScopeKind::Construct),
        // `MODULE PROCEDURE name` is an interface member or a type-bound
        // binding in those scopes, and a real definition in a submodule.  This
        // mirrors the same decision in the indentation planner.
        StatementKind::Procedure => {
            (!matches!(parent, ScopeKind::Interface | ScopeKind::DerivedType))
                .then_some(ScopeKind::Procedure)
        }
        _ => None,
    }
}

fn closes_scope(kind: StatementKind, class: StatementClass) -> bool {
    if matches!(
        kind,
        StatementKind::EndProcedure | StatementKind::EndBlock | StatementKind::EndAssociate
    ) {
        return true;
    }
    // `END`, `END MODULE`, `END TYPE`, `END INTERFACE`, `END SUBROUTINE` and
    // friends all classify as a definition end; the construct ends (`END IF`,
    // `END DO`) do not.
    class == StatementClass::EndDefinition
}

#[cfg(test)]
mod tests {
    use super::{ScopeKind, ScopeTree};
    use crate::transform::document::Document;

    fn tree(source: &[u8]) -> ScopeTree {
        ScopeTree::build(&Document::from_bytes(source).analyze().unwrap())
    }

    #[test]
    fn module_procedure_and_type_scopes_nest_and_close() {
        let tree = tree(
            b"module m\n\
              type :: t\n\
                real :: c\n\
              end type t\n\
            contains\n\
              subroutine s(x)\n\
                real :: x\n\
              end subroutine s\n\
            end module m\n",
        );
        let kinds: Vec<_> = tree.scopes.iter().map(|scope| scope.kind).collect();
        assert_eq!(
            kinds,
            [
                ScopeKind::File,
                ScopeKind::Module,
                ScopeKind::DerivedType,
                ScopeKind::Procedure
            ]
        );
        assert_eq!(tree.scopes[1].lines, 0..9);
        assert_eq!(tree.scopes[2].lines, 1..4);
        assert_eq!(tree.scopes[3].lines, 5..8);
        assert_eq!(tree.scopes[1].contains_at, Some(4));
        assert!(!tree.scopes[1].is_specification(6));
        assert!(tree.scopes[1].is_specification(1));

        assert_eq!(
            tree.enclosing_type(2).unwrap().name.as_deref(),
            Some(b"t".as_slice())
        );
        assert_eq!(
            tree.program_unit_of_line(6).unwrap().name.as_deref(),
            Some(b"s".as_slice())
        );
        assert_eq!(
            tree.program_unit_of_line(2).unwrap().name.as_deref(),
            Some(b"m".as_slice())
        );
    }

    #[test]
    fn block_and_associate_constructs_open_and_close_a_scope() {
        let tree = tree(
            b"subroutine s(obj)\n\
                associate (a => obj%c)\n\
                  x = a\n\
                end associate\n\
                outer: block\n\
                  integer :: b\n\
                end block outer\n\
                y = 1\n\
              end subroutine s\n",
        );
        let kinds: Vec<_> = tree.scopes.iter().map(|scope| scope.kind).collect();
        assert_eq!(
            kinds,
            [
                ScopeKind::File,
                ScopeKind::Procedure,
                ScopeKind::Construct,
                ScopeKind::Construct
            ]
        );
        assert_eq!(tree.scopes[2].lines, 1..4);
        assert_eq!(tree.scopes[3].lines, 4..7);
        // A construct is not a program unit, so the host still governs.
        assert_eq!(
            tree.program_unit_of_line(5).unwrap().name.as_deref(),
            Some(b"s".as_slice())
        );
        assert_eq!(tree.index_of_line(5), 3);
        assert_eq!(tree.index_of_line(7), 1);
    }

    #[test]
    fn an_interface_body_is_marked_and_its_module_procedure_is_not_a_definition() {
        let tree = tree(
            b"module m\n\
            interface\n\
              subroutine s()\n\
              end subroutine\n\
              module procedure p\n\
            end interface\n\
            end module\n",
        );
        assert!(tree.in_interface(2));
        assert!(!tree.in_interface(6));
        let procedures = tree
            .scopes
            .iter()
            .filter(|scope| scope.kind == ScopeKind::Procedure)
            .count();
        assert_eq!(
            procedures, 1,
            "module procedure in an interface opens no scope"
        );
    }

    #[test]
    fn a_truncated_file_leaves_every_scope_closed_at_the_end() {
        let tree = tree(b"module m\ncontains\nsubroutine s()\nx = 1\n");
        for scope in &tree.scopes {
            assert!(scope.lines.start <= scope.lines.end);
            assert!(scope.lines.end <= 4);
        }
        assert_eq!(
            tree.program_unit_of_line(3).unwrap().name.as_deref(),
            Some(b"s".as_slice())
        );
    }
}
