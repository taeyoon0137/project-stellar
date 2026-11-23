//! 창 session의 외부 library: 열기 진단 (XV.5.5, ADR-0127 결정 1·2).
//!
//! [`crate::window_session::ProjectWindowSession`]은 문서를 연 뒤 [`WindowLibraries::open`]으로 lock 기준 library를 한 번
//! 로드합니다(`stellar_ops::library::open_libraries_default`, 실패하지 않음).
//!
//! [`WindowLibraries::diagnostics`]는 CLI `stellar diagnose`·MCP `document.diagnostics`와 같은 기여
//! (`ProductLibraries::contribution`, origin `libraries`)입니다. 앱 진단 index·UI 연결은 UI 재설계 때 합니다.
//!
//! lock·cache가 바뀌어도 열린 창은 다시 로드하지 않습니다. 같은 창에서 다시 읽으려면 [`WindowLibraries::open`]을 다시
//! 부릅니다(`ProjectWindowSession::reload_libraries`).

use std::path::Path;

use stellar_ops::integrity::diagnostic::Diagnostic;
use stellar_ops::library::{
    open_libraries, open_libraries_default, LibraryCache, LibraryCacheError, LibraryGraphInstance,
    ProductLibraries,
};

/// 창 하나가 연 외부 library.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowLibraries {
    opened: ProductLibraries,
    instance: LibraryGraphInstance,
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
}
