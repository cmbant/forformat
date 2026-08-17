use super::{extract, scoped_declared_names, DeclaredSpelling, TypeMaps};
use crate::{analysis::scope::ScopeTree, transform::document::Document};

fn facts(source: &[u8]) -> super::FileFacts {
    let document = Document::from_bytes(source);
    let analysis = document.analyze().unwrap();
    let scopes = ScopeTree::build(&analysis);
    extract(&analysis, &scopes)
}

fn scoped(source: &[u8]) -> super::DeclaredNameIndex {
    let document = Document::from_bytes(source);
    let analysis = document.analyze().unwrap();
    let scopes = ScopeTree::build(&analysis);
    scoped_declared_names(&analysis, &scopes)
}

mod facts;
mod index;
