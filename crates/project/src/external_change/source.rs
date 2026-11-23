//! 텍스트 원본(다중 파일) 프로젝트의 열기·저장·외부 변경 반영 (V.10.6·UX.4.4, ADR-0083 결정 3·5).
//!
//! V.10.4의 다중 파일 텍스트 원본(`design/**/*.stellar.yaml`)을 창 하나의 편집 session으로 엽니다. 기본 원본
//! 전환(V.10.7) 전이라 앱 기본 경로는 아직 chunk generation이고, 이 session은 같은 규칙을 텍스트 원본 위에서
//! 실행하는 엔진 경로입니다(CLI·MCP·시험이 씀).
//!
//! | 동작 | 규칙 |
//! |---|---|
//! | 열기([`TextSourceSession::open`]) | 먼저 저장 journal의 남은 거래를 복구하고(V.10.5 [`journal::recover`]), 원본 전체를 읽어 검증합니다. 읽은 파일별 bytes hash가 base입니다 |
//! | 저장([`TextSourceSession::save`]) | 모델 전체를 다시 encode하고 base와 다른 파일만 journal 한 거래로 씁니다. 각 변경의 기대 hash는 base라, 그 사이 외부가 바꾼 파일이 있으면 아무것도 쓰지 않고 [`TextSessionError::Stale`]입니다 |
//! | 외부 변경([`TextSourceSession::poll_external`]) | 파일 hash가 base와 같으면 무시(자기 저장 포함). 깨진 원본은 반영 0이고 경로·줄·원인 진단([`SourceDiagnostic`]). 미저장 편집이 있으면 메모리를 보존하고 대기. clean이면 [`ApplyExternalSnapshot`] 하나로 반영하고 Undo 한 번으로 되돌립니다 |
//!
//! 문서 밖 의미(컴포넌트·token·이미지 catalog)는 history 대상이 아니므로(ADR-0079 결정 2) clean 반영 때 그대로
//! 바꿉니다. 그 변경은 Undo로 되돌리지 않습니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use stellar_assets::record::AssetRecord;
use stellar_commands::change::ChangeSet;
use stellar_commands::id::{CommandId, WindowSessionId};
use stellar_commands::snapshot_apply::ApplyExternalSnapshot;
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::ids::StableId;
use stellar_core_types::revision::DocumentRevision;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::model::DocumentModel;
use stellar_history::entry::HistoryError;
use stellar_history::window::WindowHistory;
use stellar_storage::journal::{self, JournalError, RecoveryReport};

use crate::document_io::session::{self as source_session, SourceBase, SourceCommitError};
use crate::document_io::source_v4::project::{decode, TextProject};
use crate::document_io::source_v4::token::TokenSetSource;
use crate::document_io::text_source::TextSourceError;
use crate::yaml::load_restricted;

/// 프로젝트 root 기준 텍스트 원본 폴더.
pub use crate::document_io::session::DESIGN_DIR;

/// 깨진 외부 원본의 진단(UX.4.4). 원본을 반영하지 않은 이유를 파일·줄 단위로 알립니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDiagnostic {
    /// `design/` 기준 파일 경로. 어느 파일인지 알 수 없으면 `None`.
    pub path: Option<String>,
    /// 1-기반 줄(문법 오류일 때).
    pub line: Option<usize>,
    /// 1-기반 열(문법 오류일 때).
    pub column: Option<usize>,
    /// 안정 code: `yaml-syntax`, `invalid-source`, `unsupported-source`, `io`.
    pub code: &'static str,
    /// 원인.
    pub cause: String,
}

impl fmt::Display for SourceDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.path, self.line, self.column) {
            (Some(path), Some(line), Some(column)) => {
                write!(f, "{path}:{line}:{column}: [{}] {}", self.code, self.cause)
            }
            (Some(path), _, _) => write!(f, "{path}: [{}] {}", self.code, self.cause),
            (None, _, _) => write!(f, "[{}] {}", self.code, self.cause),
        }
    }
}

/// 텍스트 원본 읽기 실패를 진단으로 바꿉니다. 문법 오류는 원본을 같은 제한 YAML loader로 다시 읽어 파일·줄·
/// 열을 찾습니다(오류 경로에서만 다시 읽음). 의미 오류는 메시지가 이름을 댄 파일, 없으면 바뀐 파일이 하나일 때
/// 그 파일을 경로로 씁니다.
pub fn diagnose(
    files: &BTreeMap<String, String>,
    changed: &BTreeSet<String>,
    error: &TextSourceError,
) -> SourceDiagnostic {
    for (path, text) in files {
        if let Err(syntax) = load_restricted(text) {
            return SourceDiagnostic {
                path: Some(path.clone()),
                line: Some(syntax.line),
                column: Some(syntax.col),
                code: "yaml-syntax",
                cause: syntax.message,
            };
        }
    }
    let (code, cause) = match error {
        TextSourceError::Invalid(message) => ("invalid-source", message.clone()),
        TextSourceError::Unsupported(message) => ("unsupported-source", message.clone()),
        TextSourceError::Changed => ("invalid-source", error.to_string()),
        TextSourceError::Io(io) => ("io", io.to_string()),
    };
    let named = files
        .keys()
        .filter(|path| cause.contains(path.as_str()))
        .max_by_key(|path| path.len())
        .cloned();
    let path = named.or_else(|| {
        (changed.len() == 1)
            .then(|| changed.iter().next().cloned())
            .flatten()
    });
    SourceDiagnostic {
        path,
        line: None,
        column: None,
        code,
        cause,
    }
}

/// 텍스트 원본 session 실패.
#[derive(Debug)]
pub enum TextSessionError {
    /// 원본을 읽거나 검증하지 못했습니다(열기). 진단에 파일·줄이 있습니다.
    Source(SourceDiagnostic),
    /// 저장 journal 실패(복구·잠금·I/O).
    Journal(JournalError),
    /// 원본 encode 실패(표현할 수 없는 의미).
    Encode(TextSourceError),
    /// 열거나 마지막으로 저장한 뒤 외부가 바꾼 파일이 있어 아무것도 쓰지 않았습니다.
    Stale {
        /// 바뀐 파일(`design/` 기준).
        paths: Vec<String>,
    },
    /// 문서 레코드를 만들지 못했습니다.
    Records(String),
    /// history 기록 실패.
    History(HistoryError),
}

impl fmt::Display for TextSessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextSessionError::Source(diagnostic) => write!(f, "텍스트 원본 오류: {diagnostic}"),
            TextSessionError::Journal(error) => write!(f, "저장 journal 오류: {error}"),
            TextSessionError::Encode(error) => write!(f, "텍스트 원본으로 쓸 수 없습니다: {error}"),
            TextSessionError::Stale { paths } => {
                write!(f, "외부에서 바뀐 원본이 있습니다: {}", paths.join(", "))
            }
            TextSessionError::Records(error) => write!(f, "문서 레코드 오류: {error}"),
            TextSessionError::History(error) => write!(f, "history 오류: {error}"),
        }
    }
}

impl std::error::Error for TextSessionError {}

/// 저장 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextSave {
    /// 저장할 편집이 없었습니다.
    Clean,
    /// 쓴 파일 수와 지운 파일 수.
    Saved {
        /// 새로 쓰거나 바꾼 파일 수.
        written: usize,
        /// 지운 파일 수.
        deleted: usize,
    },
}

/// 외부 변경 처리 결과.
#[derive(Debug)]
pub enum TextExternal {
    /// 파일이 base와 같습니다(자기 저장 포함).
    Unchanged,
    /// 제스처 중이라 미뤘습니다. 끝난 뒤 다시 부릅니다.
    Deferred,
    /// 깨진 원본이라 반영하지 않았습니다(UX.4.4).
    Rejected(SourceDiagnostic),
    /// 미저장 편집이 있어 메모리를 보존했습니다. 바뀐 파일(`design/` 기준). 다음 저장은 stale입니다.
    Pending {
        /// 바뀐 파일.
        paths: Vec<String>,
    },
    /// clean session에 반영했습니다.
    Applied {
        /// 문서 변경(cache 무효화 입력). 문서 밖 의미만 바뀌었으면 `None`(history 항목 없음).
        changes: Option<ChangeSet>,
        /// 문서 밖 의미(컴포넌트·token·catalog)가 바뀌었는지.
        library_changed: bool,
    },
}

/// 텍스트 원본 프로젝트의 창 하나 편집 session.
#[derive(Debug)]
pub struct TextSourceSession {
    root: PathBuf,
    document: DocumentState,
    history: WindowHistory,
    saved_revision: DocumentRevision,
    components: ComponentLibrary,
    token_sets: Vec<TokenSetSource>,
    images: BTreeMap<StableId, AssetRecord>,
    /// 마지막으로 읽거나 쓴 원본 파일 hash.
    base: SourceBase,
    recovered: RecoveryReport,
}

impl TextSourceSession {
    /// `root` 프로젝트의 텍스트 원본을 창 `window` session으로 엽니다. 남은 저장 거래를 먼저 복구합니다.
    pub fn open(
        root: &Path,
        window: WindowSessionId,
    ) -> Result<TextSourceSession, TextSessionError> {
        let recovered = journal::recover(root).map_err(TextSessionError::Journal)?;
        let design = root.join(DESIGN_DIR);
        let files = source_session::read_source_dir(&design).map_err(|error| {
            TextSessionError::Source(diagnose(&BTreeMap::new(), &BTreeSet::new(), &error))
        })?;
        let project = decode(&files).map_err(|error| {
            let all: BTreeSet<String> = files.keys().cloned().collect();
            TextSessionError::Source(diagnose(&files, &all, &error))
        })?;
        let document = DocumentState::new(project.document);
        Ok(TextSourceSession {
            root: root.to_path_buf(),
            saved_revision: document.revision(),
            document,
            history: WindowHistory::new(window),
            components: project.components,
            token_sets: project.token_sets,
            images: project.images,
            base: SourceBase::of(&files),
            recovered,
        })
    }

    /// 열 때 복구한 남은 저장 거래.
    pub fn recovered(&self) -> &RecoveryReport {
        &self.recovered
    }

    /// 작업 문서.
    pub fn document(&self) -> &DocumentState {
        &self.document
    }

    /// 창 history.
    pub fn history(&self) -> &WindowHistory {
        &self.history
    }

    /// 컴포넌트 정의.
    pub fn components(&self) -> &ComponentLibrary {
        &self.components
    }

    /// 미저장 편집이 있는지.
    pub fn is_dirty(&self) -> bool {
        self.document.revision() != self.saved_revision
    }

    /// 완료 transaction을 실행하고 history에 기록합니다.
    pub fn apply(&mut self, transaction: &CommandTransaction) -> Result<ChangeSet, HistoryError> {
        self.history
            .record_document(&mut self.document, transaction, None)
    }

    /// 마지막 항목을 되돌립니다.
    pub fn undo(&mut self) -> Result<bool, HistoryError> {
        self.history
            .undo(&mut self.document)
            .map(|outcome| outcome.is_some())
    }

    /// 제스처 preview를 시작합니다.
    pub fn begin_pointer_gesture(&mut self, gesture: u64) -> Result<(), HistoryError> {
        self.history.begin_pointer_gesture(gesture)
    }

    /// 제스처 preview를 취소합니다.
    pub fn cancel_preview(&mut self) -> Result<(), HistoryError> {
        self.history.cancel_preview(&mut self.document)
    }

    fn project(&self) -> Result<TextProject, TextSessionError> {
        let records = self
            .document
            .model()
            .to_records()
            .map_err(|error| TextSessionError::Records(error.to_string()))?;
        let (document, _) = DocumentModel::from_records(records)
            .map_err(|error| TextSessionError::Records(error.to_string()))?;
        Ok(TextProject {
            document,
            components: self.components.clone(),
            token_sets: self.token_sets.clone(),
            images: self.images.clone(),
        })
    }

    /// 편집을 journal 한 거래로 저장합니다(모듈 문서). 실패하면 base·dirty는 그대로입니다.
    pub fn save(&mut self) -> Result<TextSave, TextSessionError> {
        if !self.is_dirty() {
            return Ok(TextSave::Clean);
        }
        let revision = self.document.revision();
        let commit = match source_session::commit(&self.root, &self.base, &self.project()?) {
            Ok(commit) => commit,
            Err(SourceCommitError::Stale { paths }) => {
                return Err(TextSessionError::Stale { paths })
            }
            Err(SourceCommitError::Encode(error)) => return Err(TextSessionError::Encode(error)),
            Err(SourceCommitError::Journal(error)) => return Err(TextSessionError::Journal(error)),
        };
        self.base = commit.base;
        self.saved_revision = revision;
        Ok(TextSave::Saved {
            written: commit.written,
            deleted: commit.deleted,
        })
    }

    /// 디스크의 외부 변경을 확인하고 규칙대로 반영합니다(모듈 문서). `command`는 반영 transaction id입니다.
    pub fn poll_external(&mut self, command: CommandId) -> TextExternal {
        let design = self.root.join(DESIGN_DIR);
        let files = match source_session::read_source_dir(&design) {
            Ok(files) => files,
            Err(error) => {
                return TextExternal::Rejected(diagnose(&BTreeMap::new(), &BTreeSet::new(), &error))
            }
        };
        let now = SourceBase::of(&files);
        let changed = self.base.changed_paths(&now);
        if changed.is_empty() {
            return TextExternal::Unchanged;
        }
        let project = match decode(&files) {
            Ok(project) => project,
            Err(error) => return TextExternal::Rejected(diagnose(&files, &changed, &error)),
        };
        if self.is_dirty() {
            return TextExternal::Pending {
                paths: changed.into_iter().collect(),
            };
        }
        if self.history.history().preview_active() {
            return TextExternal::Deferred;
        }
        let incoming = match project.document.to_records() {
            Ok(records) => records,
            Err(error) => {
                return TextExternal::Rejected(SourceDiagnostic {
                    path: None,
                    line: None,
                    column: None,
                    code: "invalid-source",
                    cause: error.to_string(),
                })
            }
        };
        let apply = match ApplyExternalSnapshot::between(self.document.model(), &incoming) {
            Ok(apply) => apply,
            Err(error) => {
                return TextExternal::Rejected(SourceDiagnostic {
                    path: None,
                    line: None,
                    column: None,
                    code: "invalid-source",
                    cause: error.to_string(),
                })
            }
        };
        let changes = if apply.is_empty() {
            None
        } else {
            let transaction = apply.into_transaction(command, self.document.revision());
            match self
                .history
                .record_document(&mut self.document, &transaction, None)
            {
                Ok(changes) => Some(changes),
                Err(error) => {
                    return TextExternal::Rejected(SourceDiagnostic {
                        path: None,
                        line: None,
                        column: None,
                        code: "invalid-source",
                        cause: error.to_string(),
                    })
                }
            }
        };
        let library_changed = self.components != project.components
            || self.token_sets != project.token_sets
            || self.images != project.images;
        self.components = project.components;
        self.token_sets = project.token_sets;
        self.images = project.images;
        self.base = now;
        self.saved_revision = self.document.revision();
        TextExternal::Applied {
            changes,
            library_changed,
        }
    }
}
