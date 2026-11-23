//! 창 session의 외부 library: 열기 진단, 적용 전 읽기 전용 사전 검사, lock·cache 변경 때 재로드 (XV.5.5·XV.5.3·XV.7.4,
//! ADR-0127 결정 1·3, ADR-0137 결정 5).
//!
//! [`crate::window_session::ProjectWindowSession`]은 문서를 연 뒤 [`WindowLibraries::open`]으로 lock 기준 library를 로드합니다
//! (`stellar_ops::library::open_libraries_default`, 실패하지 않음). 결과는 세 곳에 씁니다.
//!
//! - **진단**: [`WindowLibraries::diagnostics`]는 CLI `stellar diagnose`·MCP `document.diagnostics`와 같은 프로젝트 단위 기여
//!   (`ProductLibraries::contribution`, origin `libraries`), [`WindowLibraries::document_diagnostics`]는 같은 문서 참조 단위 기여
//!   (`ProductLibraries::document_contribution`, origin `library-refs`)입니다. 앱 진단 index·UI 연결은 UI 재설계 때 합니다.
//! - **사전 검사**: 창이 transaction을 history에 기록하기 **전에** [`WindowLibraries::check`]가
//!   `stellar_ops::library::check_commands`(= `check_component_edits`)로 판정합니다. 거부되면 문서·history는 그대로입니다.
//!   Undo·Redo와 외부 snapshot 반영은 이미 있던 상태로 되돌리는 것이라 검사하지 않습니다(ADR-0119 결정 4의 `Restore`와
//!   같은 이유).
//! - **재로드**: 위치를 아는 library([`WindowLibraries::open`]·[`WindowLibraries::open_in_app_data`])는 쓸 때마다(진단·
//!   상태 조회·instance command 사전 검사) lock·`stellar.yaml`·공용 cache 지문([`LibraryStamp`])을 로드 때와 비교하고, 바뀌었으면
//!   그 자리에서 다시 로드합니다. 다른 프로세스가 `stellar library add|update|fetch|import`를 실행하면 열린 창은 다음 사용 때 새
//!   snapshot과 새 저장 참조를 씁니다. instance override는 property StableId로 저장되어 있어 새 snapshot에 같은 StableId로
//!   다시 대어집니다(문서를 다시 쓰지 않음). instance command가 아닌 편집은 지문을 보지 않습니다(편집 경로 비용 없음).
//!
//! 재로드는 `&self`로 합니다(창 session이 `Arc`로 들고 빌려 줌). 로드 결과는 세대 목록에 덧붙이고 지난 세대는 창이 닫힐 때까지
//! 둡니다(빌려 준 참조가 살아 있을 수 있음). lock·cache 변경 횟수만큼만 늘어납니다. `ProjectWindowSession::reload_libraries`는
//! 목록을 새로 시작합니다.

use std::borrow::Cow;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

use stellar_commands::command::Command;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::ids::StableId;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::model::DocumentModel;
use stellar_ops::integrity::diagnostic::Diagnostic;
use stellar_ops::library::{
    check_commands, has_external, needs_guard, now_unix_ms, open_libraries, open_libraries_default,
    LibraryCache, LibraryGraphInstance, LibraryGuardRejection, ProductLibraries,
};
use stellar_ops::library_watch::{CacheLocation, LibraryStamp};

/// 로드 한 번의 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Loaded {
    opened: ProductLibraries,
    instance: LibraryGraphInstance,
    /// 로드 직후 지문(위치를 모르는 library는 `None`).
    stamp: Option<LibraryStamp>,
}

impl Loaded {
    fn new(opened: ProductLibraries, stamp: Option<LibraryStamp>) -> Loaded {
        let instance = opened.instance();
        Loaded {
            opened,
            instance,
            stamp,
        }
    }
}

/// 로드 세대(덧붙이기만 함).
#[derive(Debug, Default)]
struct Generation {
    loaded: Loaded,
    next: OnceLock<Box<Generation>>,
}

/// 다시 로드할 위치.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Source {
    root: PathBuf,
    cache: CacheLocation,
}

impl Source {
    fn load(&self) -> Loaded {
        let opened = match &self.cache {
            CacheLocation::OsDefault => open_libraries_default(&self.root),
            CacheLocation::AppData(dir) => open_libraries(
                &self.root,
                LibraryCache::in_app_data(dir).as_ref(),
                now_unix_ms(),
            ),
        };
        let stamp = self.stamp(opened.is_complete());
        Loaded::new(opened, Some(stamp))
    }

    fn stamp(&self, complete: bool) -> LibraryStamp {
        LibraryStamp::now(&self.root, &self.cache, complete)
    }
}

/// 창 하나가 연 외부 library.
#[derive(Default)]
pub struct WindowLibraries {
    source: Option<Source>,
    first: Generation,
    reload: Mutex<()>,
}

impl fmt::Debug for WindowLibraries {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowLibraries")
            .field("watching", &self.source.is_some())
            .field("generations", &self.generations())
            .finish()
    }
}

impl Clone for WindowLibraries {
    fn clone(&self) -> WindowLibraries {
        WindowLibraries {
            source: self.source.clone(),
            first: Generation {
                loaded: self.latest().loaded.clone(),
                next: OnceLock::new(),
            },
            reload: Mutex::new(()),
        }
    }
}

impl PartialEq for WindowLibraries {
    fn eq(&self, other: &WindowLibraries) -> bool {
        self.source == other.source && self.latest().loaded == other.latest().loaded
    }
}

impl Eq for WindowLibraries {}

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
    /// library가 없는 상태(다시 로드하지 않음).
    pub fn none() -> WindowLibraries {
        WindowLibraries::default()
    }

    /// OS 기본 library cache로 `project_root`의 lock을 로드합니다. lock·cache가 바뀌면 다시 로드합니다.
    pub fn open(project_root: &Path) -> WindowLibraries {
        WindowLibraries::watching(Source {
            root: project_root.to_path_buf(),
            cache: CacheLocation::OsDefault,
        })
    }

    /// `app_data_dir` 아래 cache로 로드합니다(시험·cache 위치를 아는 호출자). lock·cache가 바뀌면 다시 로드합니다.
    pub fn open_in_app_data(project_root: &Path, app_data_dir: &Path) -> WindowLibraries {
        WindowLibraries::watching(Source {
            root: project_root.to_path_buf(),
            cache: CacheLocation::AppData(app_data_dir.to_path_buf()),
        })
    }

    /// 이미 연 결과를 그대로 씁니다(다시 로드하지 않음).
    pub fn fixed(opened: ProductLibraries) -> WindowLibraries {
        WindowLibraries {
            source: None,
            first: Generation {
                loaded: Loaded::new(opened, None),
                next: OnceLock::new(),
            },
            reload: Mutex::new(()),
        }
    }

    fn watching(source: Source) -> WindowLibraries {
        let loaded = source.load();
        WindowLibraries {
            source: Some(source),
            first: Generation {
                loaded,
                next: OnceLock::new(),
            },
            reload: Mutex::new(()),
        }
    }

    fn latest(&self) -> &Generation {
        let mut generation = &self.first;
        while let Some(next) = generation.next.get() {
            generation = next;
        }
        generation
    }

    fn generations(&self) -> usize {
        let mut count = 1;
        let mut generation = &self.first;
        while let Some(next) = generation.next.get() {
            generation = next;
            count += 1;
        }
        count
    }

    /// 지금 쓸 로드 결과: 지문이 바뀌었으면 다시 로드합니다(모듈 문서 "재로드").
    fn current(&self) -> &Loaded {
        let latest = self.latest();
        let Some(source) = &self.source else {
            return &latest.loaded;
        };
        let fresh =
            |g: &Generation| g.loaded.stamp == Some(source.stamp(g.loaded.opened.is_complete()));
        if fresh(latest) {
            return &latest.loaded;
        }
        let _guard = self.reload.lock().unwrap_or_else(PoisonError::into_inner);
        // 다른 thread가 먼저 다시 로드했을 수 있습니다.
        let latest = self.latest();
        if fresh(latest) {
            return &latest.loaded;
        }
        let _ = latest.next.set(Box::new(Generation {
            loaded: source.load(),
            next: OnceLock::new(),
        }));
        &latest.next.get().expect("방금 덧붙인 세대").loaded
    }

    /// lock·cache가 바뀌었으면 다시 로드합니다. 다시 로드했으면 `true`입니다.
    pub fn refresh(&self) -> bool {
        let before = self.generations();
        self.current();
        self.generations() != before
    }

    /// 지금까지 다시 로드한 횟수.
    pub fn reloads(&self) -> usize {
        self.generations() - 1
    }

    /// 열기 결과(바뀌었으면 다시 로드한 결과).
    pub fn state(&self) -> &ProductLibraries {
        &self.current().opened
    }

    /// 로드한 graph instance(저장된 외부 component 참조 포함).
    pub fn instance(&self) -> &LibraryGraphInstance {
        &self.current().instance
    }

    /// 열기 진단(프로젝트 단위, origin `libraries`).
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.current().opened.contribution().diagnostics
    }

    /// 문서 하나의 외부 component 참조 진단(문서 단위, origin `library-refs`, ADR-0137 결정 2). `own`은 사용 프로젝트 자신의
    /// component library입니다.
    pub fn document_diagnostics(
        &self,
        document: StableId,
        model: &DocumentModel,
        own: &ComponentLibrary,
    ) -> Vec<Diagnostic> {
        self.current()
            .opened
            .document_contribution(document, model, own)
            .diagnostics
    }

    /// `transaction`을 적용하기 전에 판정합니다. `own`은 검사가 필요할 때만 부르는 사용 프로젝트 component library입니다.
    /// 창이 이미 읽은 library는 빌려 쓰므로 편집마다 복사하지 않습니다. instance command가 없으면 지문도 보지 않습니다.
    pub fn check<'a>(
        &self,
        own: impl FnOnce() -> Cow<'a, ComponentLibrary>,
        model: &DocumentModel,
        transaction: &CommandTransaction,
    ) -> Result<(), LibraryEditRefused> {
        let guarded = transaction
            .requests
            .iter()
            .any(|request| needs_guard(std::slice::from_ref(&request.command)));
        if !guarded {
            return Ok(());
        }
        let instance = &self.current().instance;
        if !has_external(instance) {
            return Ok(());
        }
        let commands: Vec<Command> = transaction
            .requests
            .iter()
            .map(|request| request.command.clone())
            .collect();
        check_commands(&own(), instance, model, &commands)
            .map_err(|(index, rejection)| LibraryEditRefused { index, rejection })
    }
}

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;
