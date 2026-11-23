//! 창 session의 외부 library: 열기 진단과 적용 전 읽기 전용 사전 검사 (XV.5.5·XV.5.3, ADR-0127 결정 1·3).
//!
//! [`crate::window_session::ProjectWindowSession`]은 문서를 연 뒤 [`WindowLibraries::open`]으로 lock 기준 library를 한 번
//! 로드합니다(`stellar_ops::library::open_libraries_default`, 실패하지 않음). 결과는 두 곳에 씁니다.
//!
//! - **진단**: [`WindowLibraries::diagnostics`]는 CLI `stellar diagnose`·MCP `document.diagnostics`와 같은 기여
//!   (`ProductLibraries::contribution`, origin `libraries`)입니다. 앱 진단 index·UI 연결은 UI 재설계 때 합니다.
//! - **사전 검사**: 창이 transaction을 history에 기록하기 **전에** [`WindowLibraries::check`]가
//!   `stellar_ops::library::check_commands`(= `check_component_edits`)로 판정합니다. 거부되면 문서·history는 그대로입니다.
//!   Undo·Redo와 외부 snapshot 반영은 이미 있던 상태로 되돌리는 것이라 검사하지 않습니다(ADR-0119 결정 4의 `Restore`와
//!   같은 이유).
//!
//! lock·cache가 바뀌어도 열린 창은 다시 로드하지 않습니다. 같은 창에서 다시 읽으려면 [`WindowLibraries::open`]을 다시
//! 부릅니다(`ProjectWindowSession::reload_libraries`).

use std::fmt;
use std::path::Path;

use stellar_commands::command::Command;
use stellar_commands::transaction::CommandTransaction;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::model::DocumentModel;
use stellar_ops::integrity::diagnostic::Diagnostic;
use stellar_ops::library::{
    check_commands, needs_guard, open_libraries, open_libraries_default, LibraryCache,
    LibraryCacheError, LibraryGraphInstance, LibraryGuardRejection, ProductLibraries,
};

/// 창 하나가 연 외부 library.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowLibraries {
    opened: ProductLibraries,
    instance: LibraryGraphInstance,
}

/// 읽기 전용 정책 거부: transaction의 command 위치와 사유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryEditRefused {
    /// 거부된 command 위치(transaction 요청 순서).
    pub index: usize,
    /// 사유.
    pub rejection: LibraryGuardRejection,
}

impl fmt::Display for LibraryEditRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "command #{}: library 원본은 사용 프로젝트에서 바꿀 수 없습니다: {}",
            self.index, self.rejection
        )
    }
}

impl std::error::Error for LibraryEditRefused {}

/// 디스크의 사용 프로젝트 component library(`stellar_ops::component_source::project_library`). 창이 원본 library를 아직
/// 읽지 않았을 때(cache로 빠르게 연 경우·chunk 전용) 사전 검사가 씁니다. 읽지 못하면 빈 library입니다(ops 사전 검사와 같은
/// fail closed 규칙).
pub fn disk_components(project_root: &Path) -> ComponentLibrary {
    stellar_ops::snapshot::open_snapshot(project_root)
        .ok()
        .and_then(|snapshot| stellar_ops::component_source::project_library(&snapshot).ok())
        .map(|(library, _)| library)
        .unwrap_or_default()
}

impl WindowLibraries {
    /// library가 없는 상태.
    pub fn none() -> WindowLibraries {
        WindowLibraries::default()
    }

    /// OS 기본 library cache로 `project_root`의 lock을 로드합니다.
    pub fn open(project_root: &Path) -> WindowLibraries {
        WindowLibraries::from_opened(open_libraries_default(project_root))
    }

    /// 지정한 cache로 로드합니다(시험·cache 위치를 아는 호출자).
    pub fn open_with(
        project_root: &Path,
        cache: Result<&LibraryCache, &LibraryCacheError>,
        now_unix_ms: u64,
    ) -> WindowLibraries {
        WindowLibraries::from_opened(open_libraries(project_root, cache, now_unix_ms))
    }

    fn from_opened(opened: ProductLibraries) -> WindowLibraries {
        let instance = opened.instance();
        WindowLibraries { opened, instance }
    }

    /// 열기 결과.
    pub fn state(&self) -> &ProductLibraries {
        &self.opened
    }

    /// 로드한 graph instance.
    pub fn instance(&self) -> &LibraryGraphInstance {
        &self.instance
    }

    /// 열기 진단(프로젝트 단위, origin `libraries`).
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.opened.contribution().diagnostics
    }

    /// `transaction`을 적용하기 전에 판정합니다. `own`은 검사가 필요할 때만 부르는 사용 프로젝트 component library입니다.
    pub fn check(
        &self,
        own: impl FnOnce() -> ComponentLibrary,
        model: &DocumentModel,
        transaction: &CommandTransaction,
    ) -> Result<(), LibraryEditRefused> {
        if self.instance.snapshots().next().is_none() {
            return Ok(());
        }
        let commands: Vec<Command> = transaction
            .requests
            .iter()
            .map(|request| request.command.clone())
            .collect();
        if !needs_guard(&commands) {
            return Ok(());
        }
        check_commands(&own(), &self.instance, model, &commands)
            .map_err(|(index, rejection)| LibraryEditRefused { index, rejection })
    }
}
