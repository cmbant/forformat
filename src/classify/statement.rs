#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementClass {
    Definition,
    Executable,
    EndDefinition,
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementKind {
    Unknown,
    Blank,
    Program,
    Module,
    Submodule,
    Subroutine,
    Function,
    BlockData,
    Interface,
    AbstractInterface,
    Type,
    Contains,
    Procedure,
    EndProcedure,
    If,
    Else,
    ElseIf,
    EndIf,
    Do,
    EndDo,
    Select,
    Case,
    EndSelect,
    Where,
    ElseWhere,
    EndWhere,
    Forall,
    EndForall,
    Associate,
    EndAssociate,
    Block,
    EndBlock,
    Critical,
    EndCritical,
    ChangeTeam,
    EndTeam,
    Enum,
    EndEnum,
    Structure,
    EndStructure,
    Union,
    EndUnion,
    Map,
    EndMap,
    Entry,
    Include,
    LabelContinue,
    Preprocessor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementInfo {
    pub kind: StatementKind,
    pub class: StatementClass,
    pub construct_name: Option<Vec<u8>>,
    pub entity_name: Option<Vec<u8>>,
    pub statement_label: Option<u32>,
    pub referenced_labels: Vec<u32>,
    pub payload: Vec<u8>,
    pub contains_hollerith: bool,
    /// A valid-looking comma-prefixed external procedure declaration that
    /// findent leaves unframed, but whose matching END still affects its
    /// fallback indentation state.
    pub unframed_procedure: Option<StatementKind>,
    /// Explicit target for `END FUNCTION`/`END SUBROUTINE` when the generic
    /// end classifier intentionally keeps the statement kind opaque.
    pub end_kind: Option<StatementKind>,
}

impl StatementInfo {
    pub fn unknown(text: &[u8]) -> Self {
        Self {
            kind: StatementKind::Unknown,
            class: StatementClass::Neutral,
            construct_name: None,
            entity_name: None,
            statement_label: None,
            referenced_labels: Vec::new(),
            payload: text.to_vec(),
            contains_hollerith: has_hollerith(text),
            unframed_procedure: None,
            end_kind: None,
        }
    }
}
fn has_hollerith(s: &[u8]) -> bool {
    let mut i = 0;
    while i < s.len() {
        if s[i].is_ascii_digit() && (i == 0 || !s[i - 1].is_ascii_alphanumeric()) {
            let mut j = i;
            while j < s.len() && s[j].is_ascii_digit() {
                j += 1;
            }
            if s.get(j).is_some_and(|x| *x == b'h' || *x == b'H') {
                return true;
            }
        }
        i += 1;
    }
    false
}
