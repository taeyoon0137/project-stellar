//! 추가 root 문서 원본 v4(ADR-0131 결정 3).
//!
//! 텍스트 원본의 문서([`super::project`])는 root 문서 하나(`design/document.stellar.yaml`)입니다. 옛 chunk 프로젝트처럼
//! root 문서가 둘 이상이면 두 번째부터는 문서마다 폴더 하나에 같은 배치로 씁니다.
//!
//! ```text
//! design/
//!   documents.stellar.yaml                       추가 root 문서 목록(generation root 순서)
//!   documents/<slug>/document.stellar.yaml       문서 하나(첫 문서와 같은 형식)
//!   documents/<slug>/pages/<page>/<view>.stellar.yaml
//!   documents/<slug>/assets/catalog.stellar.yaml 그 문서의 Text style 표(이미지·글꼴은 첫 문서 catalog)
//!   documents/<slug>/context.stellar.yaml        그 문서의 Component Context(있을 때만)
//! ```
//!
//! 폴더 하나는 첫 문서와 같은 codec([`super::project::encode`]·[`super::project::decode`])으로 쓰고 읽습니다. 컴포넌트
//! 정의·token set·이미지 record는 프로젝트 전체 의미라 첫 문서 쪽에만 씁니다. 첫 문서만 다루는 경로(앱 창 session·
//! CLI·MCP)는 이 파일들을 바꾸지 않고, 원본 저장([`crate::document_io::session::plan_changes`])도 지우지 않습니다.
//! 첫 문서 codec([`super::project::decode`])은 이 경로들을 문서 의미 밖으로 보고 건너뜁니다. 읽기 검증은
//! [`decode`]가 합니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::ids::StableId;
use stellar_document::model::DocumentModel;

use super::super::text_codec::{field, fields, scalar, text};
use super::super::text_source::TextSourceError;
use super::css::{invalid, unsupported};
use super::project::{
    decode as decode_project, encode as encode_project, TextProject, DOCUMENT_PATH,
};
use super::slug::unique_slugs;
use super::VERSION;
use crate::yaml::{load_restricted, write_canonical_checked, YamlValue};

/// 추가 root 문서 목록 파일(`design/` 기준).
pub const DOCUMENTS_PATH: &str = "documents.stellar.yaml";
/// 추가 root 문서 폴더(`design/` 기준, 끝 `/` 포함).
pub const DOCUMENTS_DIR: &str = "documents/";
/// 목록 파일 형식 이름.
pub const DOCUMENTS_FORMAT: &str = "stellar-documents";

/// `design/` 기준 경로가 추가 root 문서의 파일(목록 파일 포함)인지.
pub fn is_documents_path(path: &str) -> bool {
    path == DOCUMENTS_PATH || path.starts_with(DOCUMENTS_DIR)
}

/// 문서 모델의 독립 사본(레코드와 Context).
pub(crate) fn copy_model(model: &DocumentModel) -> Result<DocumentModel, TextSourceError> {
    let records = model.to_records().map_err(invalid)?;
    let (mut copy, _) = DocumentModel::from_records(records).map_err(invalid)?;
    copy.replace_context(model.context().live(model));
    Ok(copy)
}

fn single(document: DocumentModel) -> TextProject {
    TextProject {
        document,
        components: Default::default(),
        token_sets: Vec::new(),
        images: Default::default(),
    }
}

/// 추가 root 문서(`documents`, generation root 순서)를 파일 집합(`design/` 기준)으로 씁니다. 비었으면 빈 집합입니다.
/// `primary`는 첫 문서 id이고 추가 문서 id와 겹치면 거부합니다. 다시 읽은 의미가 같을 때만 씁니다.
pub fn encode(
    primary: StableId,
    documents: &[DocumentModel],
) -> Result<BTreeMap<String, String>, TextSourceError> {
    let mut files = BTreeMap::new();
    if documents.is_empty() {
        return Ok(files);
    }
    let mut ids = BTreeSet::from([primary]);
    if let Some(dup) = documents.iter().find(|d| !ids.insert(d.document_id())) {
        return Err(invalid(format!(
            "root 문서 id가 겹칩니다: {}",
            dup.document_id()
        )));
    }
    let slugs = unique_slugs(
        documents
            .iter()
            .map(|d| (d.document_id(), d.definition().display_name())),
        "document",
    );
    let mut list = Vec::new();
    for document in documents {
        let dir = format!("{DOCUMENTS_DIR}{}/", slugs[&document.document_id()]);
        let encoded = encode_project(&single(copy_model(document)?))?;
        for (path, text) in encoded.files {
            files.insert(format!("{dir}{path}"), text);
        }
        list.push(scalar(format!("{dir}{DOCUMENT_PATH}")));
    }
    let index = YamlValue::Mapping(vec![
        (scalar("format"), scalar(DOCUMENTS_FORMAT)),
        (scalar("schema_version"), scalar(VERSION)),
        (scalar("documents"), YamlValue::Sequence(list)),
    ]);
    files.insert(
        DOCUMENTS_PATH.to_owned(),
        write_canonical_checked(&index).map_err(invalid)?,
    );
    let decoded = decode(primary, &files)?;
    if decoded.len() != documents.len()
        || decoded
            .iter()
            .zip(documents)
            .map(|(a, b)| Ok(semantic(a)? == semantic(b)?))
            .collect::<Result<Vec<bool>, TextSourceError>>()?
            .contains(&false)
    {
        return Err(unsupported(
            "추가 root 문서를 원본으로 정확히 복원할 수 없습니다",
        ));
    }
    Ok(files)
}

/// 문서 하나의 의미 비교 key.
pub fn semantic(document: &DocumentModel) -> Result<super::project::SemanticKey, TextSourceError> {
    single(copy_model(document)?).semantic_key()
}

/// 파일 집합(`design/` 기준, 첫 문서 파일과 섞여 있어도 됨)에서 추가 root 문서를 순서대로 읽습니다. 목록 파일이 없으면
/// 추가 문서 폴더도 없어야 합니다. 목록이 참조하지 않은 파일, 폴더 안의 컴포넌트·token·이미지 record, 첫 문서나 다른
/// 추가 문서와 같은 문서 id는 거부합니다.
pub fn decode(
    primary: StableId,
    files: &BTreeMap<String, String>,
) -> Result<Vec<DocumentModel>, TextSourceError> {
    let Some(index) = files.get(DOCUMENTS_PATH) else {
        if let Some(path) = files.keys().find(|p| p.starts_with(DOCUMENTS_DIR)) {
            return Err(invalid(format!(
                "추가 root 문서 목록 없이 문서 폴더 파일이 있습니다: {path}"
            )));
        }
        return Ok(Vec::new());
    };
    let value = load_restricted(index).map_err(invalid)?;
    fields(&value, &["format", "schema_version", "documents"])?;
    if text(field(&value, "format")?)? != DOCUMENTS_FORMAT {
        return Err(unsupported("추가 root 문서 목록 형식"));
    }
    if text(field(&value, "schema_version")?)? != VERSION {
        return Err(unsupported("추가 root 문서 목록 버전"));
    }
    let entries = field(&value, "documents")?
        .as_sequence()
        .ok_or_else(|| invalid("documents는 sequence여야 합니다"))?;
    if entries.is_empty() {
        return Err(invalid("빈 추가 root 문서 목록은 쓰지 않습니다"));
    }
    let mut used: BTreeSet<&str> = BTreeSet::from([DOCUMENTS_PATH]);
    let mut ids = BTreeSet::from([primary]);
    let mut out = Vec::new();
    for entry in entries {
        let path = text(entry)?;
        let dir = path
            .strip_prefix(DOCUMENTS_DIR)
            .and_then(|rest| rest.strip_suffix(DOCUMENT_PATH))
            .filter(|slug| {
                slug.ends_with('/')
                    && slug.len() > 1
                    && !slug[..slug.len() - 1].contains('/')
                    && slug[..slug.len() - 1] != *"."
                    && slug[..slug.len() - 1] != *".."
            })
            .map(|slug| format!("{DOCUMENTS_DIR}{slug}"))
            .ok_or_else(|| invalid(format!("추가 root 문서 경로 형식: {path}")))?;
        let sub: BTreeMap<String, String> = files
            .iter()
            .filter_map(|(p, t)| {
                p.strip_prefix(&dir)
                    .map(|rest| (rest.to_owned(), t.clone()))
            })
            .collect();
        if sub.keys().any(|p| is_documents_path(p)) {
            return Err(invalid(format!(
                "추가 root 문서 폴더 안에 다른 추가 문서가 있습니다: {dir}"
            )));
        }
        let project = decode_project(&sub).map_err(|e| invalid(format!("{dir}: {e}")))?;
        if !project.components.is_empty()
            || !project.token_sets.is_empty()
            || !project.images.is_empty()
        {
            return Err(invalid(format!(
                "{dir}: 추가 root 문서 폴더에는 문서 파일만 씁니다(컴포넌트·token·이미지는 첫 문서 쪽)"
            )));
        }
        if !ids.insert(project.document.document_id()) {
            return Err(invalid(format!("{dir}: root 문서 id가 겹칩니다")));
        }
        for p in files.keys().filter(|p| p.starts_with(&dir)) {
            used.insert(p.as_str());
        }
        out.push(project.document);
    }
    if let Some(unused) = files
        .keys()
        .find(|p| is_documents_path(p) && !used.contains(p.as_str()))
    {
        return Err(invalid(format!(
            "추가 root 문서 목록이 참조하지 않은 파일: {unused}"
        )));
    }
    Ok(out)
}
