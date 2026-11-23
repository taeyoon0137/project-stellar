//! 텍스트 원본(View·문서·token·catalog·Context 파일)의 3-way semantic merge (ADR-0113 결정 5).
//!
//! 텍스트 원본 파일 하나는 프로젝트 전체 의미의 일부라서 그 파일만으로는 해석할 수 없습니다(View 목록은 문서 파일에,
//! Text style 표는 catalog에 있음). 그래서 병합 입력은 세 쪽의 **원본 파일 집합 전체**입니다([`TextSides`]).
//!
//! 1. 세 쪽을 같은 codec(`source_v4::project::decode`)으로 읽습니다. 한쪽이라도 읽지 못하면 병합하지 않습니다.
//! 2. 문서는 레코드를 semantic snapshot으로 올려 엔진으로 병합합니다(XIV.10). 입력은 무손실 왕복이어야 합니다
//!    (`merge_apply::document`와 같은 조건).
//! 3. 문서 밖 의미는 단위별 3-way입니다. 이미지 record·token set은 id별로, Component Context는 통째로 비교해 한쪽만
//!    바뀌었으면 그쪽을, 둘이 같으면 그 값을 받습니다. 두 쪽이 다르게 바꿨으면 병합하지 않습니다([`TextMergeError`]).
//!    컴포넌트 정의는 파일마다 컴포넌트 원본 병합(XXIV.2.1)이 따로 다루므로 이 병합의 쓰기 결과에 넣지 않습니다
//!    ([`TextMergeBase::encode`]는 `components/` 파일을 빼고 돌려줌). 인코딩 검증에는 한쪽만 바뀐 쪽을, 둘 다 바뀌었으면
//!    ours 정의를 씁니다.
//! 4. 결과 snapshot을 문서 레코드로 되돌려 원본 파일 집합으로 인코딩하고, 다시 읽은 의미가 결과와 같을 때만 받습니다.
//!
//! Git merge driver는 한 경로의 세 bytes만 받습니다([`merge_text_file`]). driver는 저장소 최상위에서 실행되고 그때
//! 작업 트리는 merge 전 ours이므로, 나머지 원본 파일은 작업 트리 것을 세 쪽 공통 문맥으로 씁니다. 결과가 그 경로 밖의
//! 원본 파일까지 바꿔야 하면 driver로 표현할 수 없어 병합하지 않습니다(Git이 conflict로 남기고 conflict 세션이 세 stage
//! 전체로 다시 병합함).
//!
//! 이 모듈은 파일을 쓰지 않습니다.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use stellar_document::context::set::ContextSet;
use stellar_document::model::DocumentModel;
use stellar_document::records::DocumentRecords;
use stellar_project::document_io::session::{SourceLibrary, STYLE_DEFAULTS_PREFIX};
use stellar_project::document_io::source_v4::project::{decode, encode, TextProject};
use stellar_semantic_diff::snapshot::SemanticSnapshot;
use stellar_semantic_merge::apply::{reconcile, Reconciliation};
use stellar_semantic_merge::model::MergeInput;

use crate::candidate::file_merge::FileMergeOutcome;
use crate::compare::ConflictEntry;
use crate::core_types::ids::StableId;
use crate::diff::snapshot_of;
use crate::merge_apply::document::project_documents;

/// 텍스트 원본 폴더(프로젝트 기준).
pub const DESIGN_PREFIX: &str = "design/";
const EXT: &str = ".stellar.yaml";

/// merge driver 등록이 `.gitattributes`에 쓰는 텍스트 원본 경로 pattern([`is_text_source_path`]와 같은 집합).
pub const ATTRIBUTE_PATTERNS: [&str; 5] = [
    "design/document.stellar.yaml",
    "design/pages/**/*.stellar.yaml",
    "design/assets/catalog.stellar.yaml",
    "design/context.stellar.yaml",
    "design/tokens/*.stellar.yaml",
];

/// `design/` 기준 경로 → 원본 텍스트.
pub type Files = BTreeMap<String, String>;

/// 프로젝트 기준 `path`가 이 병합이 다루는 텍스트 원본 파일인지(문서·View·Frame 하위 파일·catalog·Context·token).
/// 컴포넌트 원본(`design/components/`)과 기본값 세트(`design/style-defaults/`)는 아닙니다.
pub fn is_text_source_path(path: &Path) -> bool {
    design_relative(path).is_some_and(|relative| {
        !relative.starts_with("components/")
            && !relative.starts_with(STYLE_DEFAULTS_PREFIX)
            && (relative == "document.stellar.yaml"
                || relative == "assets/catalog.stellar.yaml"
                || relative == "context.stellar.yaml"
                || relative.starts_with("pages/")
                || (relative.starts_with("tokens/") && relative.matches('/').count() == 1))
    })
}

/// 프로젝트 기준 `path`의 `design/` 기준 경로(`.stellar.yaml` 파일만).
pub fn design_relative(path: &Path) -> Option<String> {
    let text = path.to_str()?.replace('\\', "/");
    let relative = text.strip_prefix(DESIGN_PREFIX)?;
    (relative.ends_with(EXT)
        && !relative
            .split('/')
            .any(|part| part.is_empty() || part == ".."))
    .then(|| relative.to_owned())
}

/// 세 쪽 원본 파일 집합.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextSides {
    /// base.
    pub base: Files,
    /// ours.
    pub ours: Files,
    /// theirs.
    pub theirs: Files,
}

/// 병합하지 못한 이유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextMergeError {
    /// 한쪽 원본을 읽을 수 없습니다.
    Decode {
        /// `base`·`ours`·`theirs`.
        side: &'static str,
        /// 원인.
        reason: String,
    },
    /// 세 쪽 root 문서가 다릅니다.
    DocumentMismatch,
    /// 한쪽 문서를 snapshot으로 무손실 왕복할 수 없습니다.
    Lossy {
        /// `base`·`ours`·`theirs`·`result`.
        side: &'static str,
    },
    /// 문서 밖 의미를 두 쪽이 다르게 바꿨습니다.
    Library {
        /// 단위 이름(`image`·`token-set`·`context`).
        unit: &'static str,
    },
    /// 결과를 원본으로 쓸 수 없습니다.
    Encode(String),
    /// 병합 엔진 실패.
    Merge(String),
    /// driver: 결과가 이 경로 밖의 원본 파일도 바꿔야 합니다.
    OtherFiles(Vec<String>),
}

impl fmt::Display for TextMergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextMergeError::Decode { side, reason } => {
                write!(f, "{side} 텍스트 원본을 읽을 수 없습니다: {reason}")
            }
            TextMergeError::DocumentMismatch => f.write_str("세 쪽의 root 문서가 다릅니다"),
            TextMergeError::Lossy { side } => write!(
                f,
                "{side} 문서를 semantic snapshot으로 무손실 왕복할 수 없어 병합하지 않습니다"
            ),
            TextMergeError::Library { unit } => {
                write!(f, "문서 밖 의미({unit})를 두 쪽이 다르게 바꿨습니다")
            }
            TextMergeError::Encode(e) => {
                write!(f, "병합 결과를 텍스트 원본으로 쓸 수 없습니다: {e}")
            }
            TextMergeError::Merge(e) => write!(f, "semantic merge 실패: {e}"),
            TextMergeError::OtherFiles(paths) => write!(
                f,
                "병합 결과가 이 파일 밖의 원본도 바꿔야 해 driver로 쓸 수 없습니다: {}",
                paths.join(", ")
            ),
        }
    }
}

impl std::error::Error for TextMergeError {}

/// 3-way 값 하나: 한쪽만 바뀌었으면 그쪽, 같으면 그 값, 둘 다 다르게 바뀌었으면 `None`.
fn three_way<T: PartialEq + Clone>(base: &T, ours: &T, theirs: &T) -> Option<T> {
    if ours == theirs || theirs == base {
        Some(ours.clone())
    } else if ours == base {
        Some(theirs.clone())
    } else {
        None
    }
}

/// id별 3-way(없음도 값).
fn three_way_map<K: Ord + Clone, V: PartialEq + Clone>(
    base: &BTreeMap<K, V>,
    ours: &BTreeMap<K, V>,
    theirs: &BTreeMap<K, V>,
    unit: &'static str,
) -> Result<BTreeMap<K, V>, TextMergeError> {
    let mut keys: Vec<&K> = base
        .keys()
        .chain(ours.keys())
        .chain(theirs.keys())
        .collect();
    keys.sort();
    keys.dedup();
    let mut out = BTreeMap::new();
    for key in keys {
        match three_way(&base.get(key), &ours.get(key), &theirs.get(key)) {
            Some(Some(value)) => {
                out.insert(key.clone(), value.clone());
            }
            Some(None) => {}
            None => return Err(TextMergeError::Library { unit }),
        }
    }
    Ok(out)
}

/// 한쪽 해석 결과.
struct Side {
    records: DocumentRecords,
    context: ContextSet,
    library: SourceLibrary,
}

fn side(files: &Files, name: &'static str) -> Result<Side, TextMergeError> {
    let project = decode(files).map_err(|e| TextMergeError::Decode {
        side: name,
        reason: e.to_string(),
    })?;
    let (model, library) = SourceLibrary::split(project);
    let context = model.context().live(&model);
    let mut records = model.to_records().map_err(|e| TextMergeError::Decode {
        side: name,
        reason: e.to_string(),
    })?;
    records.sort_canonical();
    Ok(Side {
        records,
        context,
        library,
    })
}

fn lossless(
    records: &DocumentRecords,
    name: &'static str,
) -> Result<SemanticSnapshot, TextMergeError> {
    let snapshot = snapshot_of(&[records]).map_err(|e| TextMergeError::Merge(e.0.clone()))?;
    let back = stellar_semantic_diff::records::document_records(&snapshot, records.definition.id)
        .map_err(|e| TextMergeError::Merge(e.to_string()))?;
    if &back.records != records {
        return Err(TextMergeError::Lossy { side: name });
    }
    Ok(snapshot)
}

/// 결과를 원본으로 쓰는 데 필요한 문서 밖 의미(병합한 값).
#[derive(Debug, Clone)]
pub struct TextMergeBase {
    document: StableId,
    library: SourceLibrary,
    context: ContextSet,
    ours: Files,
}

impl TextMergeBase {
    /// root 문서.
    pub fn document(&self) -> StableId {
        self.document
    }

    /// ours 원본 파일 집합.
    pub fn ours(&self) -> &Files {
        &self.ours
    }

    /// 결과 snapshot을 원본 파일 집합으로 인코딩합니다. `components/` 파일은 뺍니다(모듈 문서 3번). 다시 읽은 의미가
    /// 결과와 다르면 거부합니다.
    pub fn encode(&self, merged: &SemanticSnapshot) -> Result<Files, TextMergeError> {
        let mut documents = project_documents(merged).map_err(|e| match e {
            crate::merge_apply::document::DocumentMergeError::Lossy { side } => {
                TextMergeError::Lossy { side }
            }
            other => TextMergeError::Merge(other.to_string()),
        })?;
        let records = match (documents.pop(), documents.is_empty()) {
            (Some(records), true) if records.definition.id == self.document => records,
            _ => return Err(TextMergeError::DocumentMismatch),
        };
        let (mut model, _) = DocumentModel::from_records(records)
            .map_err(|e| TextMergeError::Encode(format!("{e:?}")))?;
        model.replace_context(self.context.clone());
        let project: TextProject = self.library.join(model);
        let key = project
            .semantic_key()
            .map_err(|e| TextMergeError::Encode(e.to_string()))?;
        let encoded = encode(&project).map_err(|e| TextMergeError::Encode(e.to_string()))?;
        let back = decode(&encoded.files)
            .and_then(|project| project.semantic_key())
            .map_err(|e| TextMergeError::Encode(e.to_string()))?;
        if back != key {
            return Err(TextMergeError::Lossy { side: "result" });
        }
        let mut files = encoded.files;
        files.retain(|path, _| !path.starts_with("components/"));
        Ok(files)
    }
}

/// 세 쪽 원본을 읽어 문서 병합 입력과 문서 밖 의미 병합 결과를 만듭니다(모듈 문서 1~3번).
pub fn prepare_text_merge(
    sides: &TextSides,
) -> Result<(MergeInput, TextMergeBase), TextMergeError> {
    let base = side(&sides.base, "base")?;
    let ours = side(&sides.ours, "ours")?;
    let theirs = side(&sides.theirs, "theirs")?;
    let document = base.records.definition.id;
    if ours.records.definition.id != document || theirs.records.definition.id != document {
        return Err(TextMergeError::DocumentMismatch);
    }
    let images = three_way_map(
        &base.library.images,
        &ours.library.images,
        &theirs.library.images,
        "image",
    )?;
    let tokens = |library: &SourceLibrary| -> BTreeMap<StableId, _> {
        library
            .token_sets
            .iter()
            .map(|set| (set.id, set.clone()))
            .collect()
    };
    let token_sets = three_way_map(
        &tokens(&base.library),
        &tokens(&ours.library),
        &tokens(&theirs.library),
        "token-set",
    )?
    .into_values()
    .collect();
    let context = three_way(&base.context, &ours.context, &theirs.context)
        .ok_or(TextMergeError::Library { unit: "context" })?;
    let components = if ours.library.components.encode() == base.library.components.encode() {
        theirs.library.components.clone()
    } else {
        ours.library.components.clone()
    };
    let input = MergeInput {
        base: lossless(&base.records, "base")?,
        ours: lossless(&ours.records, "ours")?,
        theirs: lossless(&theirs.records, "theirs")?,
    };
    Ok((
        input,
        TextMergeBase {
            document,
            library: SourceLibrary {
                components,
                token_sets,
                images,
            },
            context,
            ours: sides.ours.clone(),
        },
    ))
}

/// 세 쪽 원본을 병합합니다. conflict가 없으면 결과 원본 파일 집합(`components/` 제외), 있으면 conflict 목록입니다.
pub fn merge_text(sides: &TextSides) -> Result<TextMerge, TextMergeError> {
    let (input, base) = prepare_text_merge(sides)?;
    let decision = reconcile(input).map_err(|e| TextMergeError::Merge(e.to_string()))?;
    match &decision {
        Reconciliation::Conflicted(candidate) => Ok(TextMerge::Conflicts(
            candidate
                .conflicts()
                .iter()
                .map(ConflictEntry::from)
                .collect(),
        )),
        Reconciliation::Blocked { blocked, .. } => Err(TextMergeError::Merge(format!(
            "병합 결과가 새 snapshot 위반 {}건을 만듭니다",
            blocked.violations.len()
        ))),
        _ => {
            let snapshot = decision.result().expect("쓸 수 있는 결과");
            Ok(TextMerge::Merged(base.encode(snapshot)?))
        }
    }
}

/// [`merge_text`] 결과.
#[derive(Debug, Clone, PartialEq)]
pub enum TextMerge {
    /// 결과 원본 파일 집합(`components/` 제외).
    Merged(Files),
    /// 자동 병합하지 못한 conflict(엔진 순서).
    Conflicts(Vec<ConflictEntry>),
}

/// 프로젝트 `root`의 작업 트리 텍스트 원본 파일 집합(기본값 세트 제외, driver 문맥).
pub fn read_context(root: &Path) -> Result<Files, String> {
    stellar_project::document_io::session::read_source_dir(
        &root.join(DESIGN_PREFIX.trim_end_matches('/')),
    )
    .map_err(|e| e.to_string())
}

fn with_side(context: &Files, relative: &str, bytes: &[u8]) -> Result<Files, TextMergeError> {
    let mut files = context.clone();
    files.remove(relative);
    if !bytes.is_empty() {
        let text = String::from_utf8(bytes.to_vec()).map_err(|_| TextMergeError::Decode {
            side: "input",
            reason: "UTF-8이 아닌 텍스트 원본".into(),
        })?;
        files.insert(relative.to_owned(), text);
    }
    Ok(files)
}

/// Git merge driver 한 경로(`relative`, `design/` 기준)의 세 bytes를 병합합니다(모듈 문서). `context`는 작업 트리의
/// 나머지 원본 파일입니다(그 경로가 들어 있으면 무시함). 빈 bytes는 그쪽에 파일이 없음입니다.
pub fn merge_text_file(
    context: &Files,
    relative: &str,
    base: &[u8],
    ours: &[u8],
    theirs: &[u8],
) -> Result<FileMergeOutcome, TextMergeError> {
    let sides = TextSides {
        base: with_side(context, relative, base)?,
        ours: with_side(context, relative, ours)?,
        theirs: with_side(context, relative, theirs)?,
    };
    let files = match merge_text(&sides)? {
        TextMerge::Conflicts(conflicts) => return Ok(FileMergeOutcome::Conflicts(conflicts)),
        TextMerge::Merged(files) => files,
    };
    let mut others: Vec<String> = files
        .keys()
        .chain(sides.ours.keys())
        .filter(|path| path.as_str() != relative && !path.starts_with("components/"))
        .filter(|path| files.get(*path) != sides.ours.get(*path))
        .cloned()
        .collect();
    others.sort();
    others.dedup();
    if !others.is_empty() {
        return Err(TextMergeError::OtherFiles(others));
    }
    Ok(match files.get(relative) {
        Some(text) => FileMergeOutcome::Merged(text.clone().into_bytes()),
        None => FileMergeOutcome::Deleted,
    })
}
