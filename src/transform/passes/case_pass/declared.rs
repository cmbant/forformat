//! Declared-name spelling engine and occurrence classification.

use super::{
    associations::{
        apply_select_guard, associate_spelling, association_opening_scope,
        is_associate_alias_declaration, is_select_alias_declaration, is_select_type_rank_keyword,
        select_association_spec, AssociateFrame, AssociationScope,
    },
    members::{
        component_owner_names, exact_member_owner, inherited_component_spelling,
        inherited_type_procedure_spelling, member_owner_type,
    },
    syntax::{
        active_procedure, implicit_guard_applies, is_declaration_entity, is_external_reference,
        is_intrinsic_kind_name, is_numeric_literal_kind_name, is_type_spec_name, is_use_intrinsic,
        is_use_module, is_use_only_keyword, is_use_rename_local, is_use_statement, named_end_space,
        preceded_by_percent, scope_header_space, use_module_index,
    },
};
use crate::{
    analysis::{
        names::{resolve, NameSpace},
        project::ResolvedType,
        scoped_declared_names, CaseMap, DeclaredNameIndex, DeclaredSpelling,
    },
    classify::{classify, StatementKind},
    error::FormatError,
    source::{
        tokens::{tokenize, Token, TokenKind},
        LexState,
    },
    transform::{
        document::Document,
        edit::EditBuffer,
        passes::provenance::{source_spans, spread_replacement},
        pipeline::{Changed, PassContext},
    },
};
use std::{collections::HashMap, ops::Range};

include!("declared/model.rs");
include!("declared/engine.rs");
include!("declared/rules1.rs");
include!("declared/rules2.rs");
include!("declared/evidence.rs");
