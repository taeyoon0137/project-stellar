//! 초기 릴리스 종단 시나리오(XVI.11.1~XVI.11.6, ADR-0111) 공용 helper.
//!
//! 앱 UI 없이 제품 경로만 씁니다.
//!
//! - **앱 runtime**: [`App`]은 셸이 C ABI로 부르는 [`AppRuntime`]을 같은 순서(요청 → 완료 event → 상태 snapshot)로
//!   부릅니다. 새 프로젝트 생성, 창 열기·닫기, Frame·Text·Image 배치, 속성 패널 편집, Save Now, 자동 저장, stale
//!   두 선택지, 복구가 모두 셸과 같은 진입점입니다.
//! - **headless**: CLI·MCP가 쓰는 `stellar_ops`(apply·component_source·git·merge 세션)를 in-process로 부릅니다.
//! - **환경**: 테스트 프로세스의 `HOME`을 임시 폴더로 바꿔(한 번만) 저장 잠금·신뢰 저장소·복구본이 사용자 실제
//!   데이터 폴더에 닿지 않게 합니다. Git은 그 `HOME`의 전역 설정 파일(identity·`url.<bare>.insteadOf`)만 읽고 시스템
//!   설정은 읽지 않습니다. 네트워크는 쓰지 않습니다.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use stellar_core_types::ids::StableId;
use stellar_core_types::ids::WindowSessionId;
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::value::{Rect, Size2, Vec2};
use stellar_document::model::DocumentModel;
use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_git_porcelain::runner::GitRunner;
use stellar_git_porcelain::trust::{GatedGitRunner, GitTrust};
use stellar_ops::git::{system_git_runner, GitSession};
use stellar_ops::snapshot::open_snapshot;
use stellar_project::identity::{parse_project_id, ProjectIdentity};
use stellar_runtime::app::text_input::{TextRange, TextTarget};
use stellar_runtime::app::{
    AppRuntime, ImagePlacement, ProjectSessionId, RequestFailure, RequestId, RuntimeEvent,
    SelectionEditTarget, WindowSessionHandle, WindowSessionSummary,
};
use stellar_runtime::window_session::ProjectWindowSession;
use stellar_storage::hash::ContentHash;

/// 완료 event를 기다리는 상한. 다른 작업이 동시에 빌드·시험해도 넘지 않을 만큼 넉넉히 둡니다.
pub const EVENT_TIMEOUT: Duration = Duration::from_secs(30);

/// 자식 프로세스가 부모 테스트 HOME을 이어받는 환경 변수 이름.
pub const INHERITED_HOME: &str = "STELLAR_E2E_INHERITED_HOME";

/// 테스트 프로세스 전용 사용자 데이터 폴더(`HOME`). 처음 부를 때 만들고 환경 변수를 바꿉니다.
///
/// 같은 프로세스의 모든 테스트가 같은 값을 봐야 하므로, 각 테스트는 무엇보다 먼저 이 함수를 부릅니다. Rust 2021에서
/// `set_var`는 안전하지만 다른 thread가 환경을 읽는 중이면 경쟁이므로 `OnceLock` 초기화 안에서 한 번만 바꿉니다.
pub fn test_home() -> &'static Path {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        // 비정상 종료 시나리오의 자식 프로세스는 부모의 HOME(복구본·잠금 위치)을 그대로 이어받습니다.
        if let Some(inherited) = std::env::var_os(INHERITED_HOME) {
            let home = PathBuf::from(inherited);
            std::env::set_var("HOME", &home);
            std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
            std::env::set_var("GIT_CONFIG_GLOBAL", home.join(".gitconfig"));
            std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
            return home;
        }
        let home = std::env::temp_dir().join(format!("stellar-e2e-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("테스트 HOME 생성");
        let home = home.canonicalize().expect("테스트 HOME 실제 경로");
        let config = home.join(".gitconfig");
        std::fs::write(
            &config,
            "[user]\n\tname = Stellar E2E\n\temail = e2e@stellar.invalid\n\
             [core]\n\tautocrlf = false\n\tfsmonitor = false\n\
             [commit]\n\tgpgsign = false\n\
             [init]\n\tdefaultBranch = main\n",
        )
        .expect("전역 gitconfig 기록");
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
        std::env::set_var("GIT_CONFIG_GLOBAL", &config);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        for key in [
            "GIT_AUTHOR_NAME",
            "GIT_AUTHOR_EMAIL",
            "GIT_COMMITTER_NAME",
            "GIT_COMMITTER_EMAIL",
            "EMAIL",
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
        ] {
            std::env::remove_var(key);
        }
        home
    })
}

/// 테스트 HOME의 전역 gitconfig에 한 줄 설정을 더합니다(`git config --global`과 같은 효과).
pub fn add_global_git_config(section: &str, key: &str, value: &str) {
    let config = test_home().join(".gitconfig");
    let mut text = std::fs::read_to_string(&config).expect("gitconfig 읽기");
    text.push_str(&format!("[{section}]\n\t{key} = {value}\n"));
    std::fs::write(&config, text).expect("gitconfig 쓰기");
}

/// 임시 디렉터리(drop 시 삭제). 경로는 symlink를 해석한 실제 경로입니다(macOS `/var` → `/private/var`).
pub struct TempDir {
    /// 실제 경로.
    pub path: PathBuf,
}

impl TempDir {
    /// `tag`로 시작하는 새 빈 디렉터리.
    pub fn new(tag: &str) -> TempDir {
        test_home();
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("stellar-e2e-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("임시 디렉터리 생성");
        TempDir {
            path: path.canonicalize().expect("임시 디렉터리 실제 경로"),
        }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// `until`이 참이 될 때까지 최대 `timeout` 기다립니다. 마지막 판정을 돌려줍니다.
pub fn wait_until(timeout: Duration, mut until: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if until() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    until()
}

/// 1×1 RGBA PNG(제품 가져오기가 확인하는 signature·IHDR·IEND를 가진 실제 PNG bytes).
pub const TINY_PNG: [u8; 67] = [
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

/// 앱 runtime 하나(셸 하나에 해당). 요청을 보내고 그 요청의 완료 event를 기다립니다.
pub struct App {
    /// runtime.
    pub rt: AppRuntime,
}

/// 요청 실패 완료 event.
#[derive(Debug)]
pub struct Failed {
    /// 실패 코드.
    pub failure: RequestFailure,
    /// 메시지.
    pub message: String,
}

impl App {
    /// runtime을 시작합니다(자동 저장·복구 체크포인트는 켜지 않음).
    pub fn start() -> App {
        test_home();
        App {
            rt: AppRuntime::start().expect("앱 runtime 시작"),
        }
    }

    /// 복구 체크포인트(테스트 HOME의 앱 데이터)를 켠 runtime.
    pub fn start_with_recovery() -> App {
        let mut app = App::start();
        app.rt.enable_recovery();
        app
    }

    /// `request`의 완료 event를 기다립니다. 그 사이의 요청 없는 표시 변경 event(request 0)는 건너뜁니다.
    pub fn complete(&self, request: RequestId) -> Result<RuntimeEvent, Failed> {
        let deadline = Instant::now() + EVENT_TIMEOUT;
        loop {
            if let Some(event) = self.rt.pop_event() {
                if event.request() == RequestId(0) {
                    continue;
                }
                assert_eq!(
                    event.request(),
                    request,
                    "다른 요청의 완료 event: {event:?}"
                );
                return match event {
                    RuntimeEvent::RequestFailed {
                        failure, message, ..
                    } => Err(Failed { failure, message }),
                    other => Ok(other),
                };
            }
            assert!(
                Instant::now() < deadline,
                "요청 {request:?}의 완료 event가 오지 않았습니다"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// 성공해야 하는 요청의 완료 event.
    pub fn ok(&self, request: RequestId, label: &str) -> RuntimeEvent {
        self.complete(request)
            .unwrap_or_else(|f| panic!("{label}: {:?} {}", f.failure, f.message))
    }

    /// 새 프로젝트를 `target`(아직 없는 폴더)에 만듭니다. 반환: Git 초기화 상태(0 = 최초 commit까지 성공).
    pub fn create_project(&self, target: &Path) -> u32 {
        let request = self.rt.create_project(target).expect("생성 요청");
        match self.ok(request, "새 프로젝트") {
            RuntimeEvent::ProjectCreated { git_state, .. } => git_state,
            other => panic!("생성 완료 event가 아님: {other:?}"),
        }
    }

    /// 프로젝트를 엽니다.
    pub fn open_project(&self, root: &Path) -> ProjectSessionId {
        let request = self.rt.open_project(root).expect("열기 요청");
        match self.ok(request, "프로젝트 열기") {
            RuntimeEvent::ProjectOpened { project, .. } => project,
            other => panic!("열기 완료 event가 아님: {other:?}"),
        }
    }

    /// 프로젝트를 닫습니다(그 아래 창도 닫힘).
    pub fn close_project(&self, project: ProjectSessionId) {
        let request = self.rt.close_project(project).expect("프로젝트 닫기 요청");
        self.ok(request, "프로젝트 닫기");
    }

    /// 창을 엽니다. 반환: (handle, 작업 문서 id, loaded generation).
    pub fn open_window(&self, project: ProjectSessionId) -> (WindowSessionHandle, StableId, u64) {
        let request = self.rt.open_window_session(project).expect("창 열기 요청");
        match self.ok(request, "창 열기") {
            RuntimeEvent::WindowSessionOpened {
                session,
                document_id,
                generation,
                ..
            } => (session, document_id, generation),
            other => panic!("창 열기 완료 event가 아님: {other:?}"),
        }
    }

    /// 창을 닫습니다(저장하지 않음).
    pub fn close_window(&self, session: WindowSessionHandle) {
        let request = self.rt.close_window_session(session).expect("창 닫기 요청");
        self.ok(request, "창 닫기");
    }

    /// 창 상태 요약(최신 게시 snapshot).
    pub fn window(&self, session: WindowSessionHandle) -> WindowSessionSummary {
        self.rt
            .state()
            .window(session)
            .cloned()
            .expect("열린 창 요약")
    }

    /// 활성 View와 그 revision·history 위치(셸이 표시한 값과 같은 편집 대상).
    pub fn target(&self, session: WindowSessionHandle) -> SelectionEditTarget {
        let window = self.window(session);
        SelectionEditTarget {
            session,
            view: window.active_view.expect("활성 View"),
            revision: window.revision,
            history_sequence: window.history_sequence,
        }
    }

    /// Save Now. 반환: 성공이면 (generation, written), 실패면 실패 event.
    pub fn try_save(&self, session: WindowSessionHandle) -> Result<(u64, bool), Failed> {
        let request = self.rt.save_now(session).expect("저장 요청");
        self.complete(request).map(|event| match event {
            RuntimeEvent::WindowSessionSaved {
                generation,
                written,
                ..
            } => (generation, written),
            other => panic!("저장 완료 event가 아님: {other:?}"),
        })
    }

    /// 성공해야 하는 Save Now.
    pub fn save(&self, session: WindowSessionHandle) -> (u64, bool) {
        self.try_save(session)
            .unwrap_or_else(|f| panic!("저장 실패: {:?} {}", f.failure, f.message))
    }

    /// 부모 Frame local 영역에 Frame을 그려 넣습니다. 반환: 새 Frame id(삽입 뒤 primary 선택).
    pub fn insert_frame(
        &self,
        session: WindowSessionHandle,
        parent: StableId,
        bounds: Rect,
    ) -> StableId {
        let window = self.window(session);
        let request = self
            .rt
            .insert_canvas_frame(
                session,
                window.active_view.expect("활성 View"),
                window.revision,
                parent,
                bounds,
            )
            .expect("Frame 삽입 요청");
        self.ok(request, "Frame 삽입");
        let after = self.window(session);
        assert_eq!(
            after.revision.0,
            window.revision.0 + 1,
            "Frame 삽입은 revision 1 증가"
        );
        after.primary_selection.expect("새 Frame 선택")
    }

    /// 부모 Frame의 `origin`에 새 Text를 만들고 `content`를 확정 입력합니다(IME 조합 시작 → 확정). 반환: Text id.
    pub fn insert_text(
        &self,
        session: WindowSessionHandle,
        parent: StableId,
        origin: Vec2,
        content: &str,
    ) -> StableId {
        let window = self.window(session);
        let (token, node) = self
            .rt
            .begin_canvas_text(
                session,
                window.active_view.expect("활성 View"),
                window.revision,
                TextTarget::New { parent, origin },
                TextRange::new(0, 0).expect("빈 범위"),
            )
            .expect("Text 조합 시작");
        self.ok(token, "Text 조합 시작");
        let request = self
            .rt
            .commit_canvas_text(session, token.0, content.to_string())
            .expect("Text 확정 요청");
        self.ok(request, "Text 확정");
        node
    }

    /// 사용자가 고른 PNG 파일 `source`를 부모 Frame의 `position`에 배치합니다. 반환: Image node id.
    pub fn place_png(
        &self,
        session: WindowSessionHandle,
        parent: StableId,
        position: Vec2,
        size: Option<Size2>,
        source: &Path,
    ) -> StableId {
        let bytes = std::fs::read(source).expect("PNG 읽기");
        let mut digest = [0u8; 32];
        digest.copy_from_slice(ContentHash::compute(&bytes).digest());
        let target = self.target(session);
        let request = self
            .rt
            .place_png(ImagePlacement {
                target,
                parent,
                position,
                size,
                source: source.to_path_buf(),
                digest,
            })
            .expect("이미지 배치 요청");
        self.ok(request, "이미지 배치");
        self.window(session)
            .primary_selection
            .expect("새 Image 선택")
    }

    /// 창 선택을 바꿉니다.
    pub fn select(&self, session: WindowSessionHandle, ids: &[StableId]) {
        let request = self
            .rt
            .set_selection(session, ids.to_vec())
            .expect("선택 요청");
        self.ok(request, "선택");
    }

    /// 확정 선택에 속성 패널 값 하나를 확정 입력합니다(CSS 값 텍스트는 속성 문법 parser 하나로 해석).
    pub fn edit_property(
        &self,
        session: WindowSessionHandle,
        property: stellar_runtime::app::inspector::InspectorProperty,
        text: &str,
    ) {
        use stellar_runtime::app::inspector::{parse_input, InspectorInput, PropertyEditPhase};
        let value = parse_input(property, text).expect("CSS 값");
        let request = self
            .rt
            .edit_selection_property(
                self.target(session),
                property,
                InspectorInput::Set(value),
                PropertyEditPhase::Commit,
            )
            .expect("속성 편집 요청");
        self.ok(request, "속성 편집");
    }

    /// runtime을 끝까지 기다려 종료합니다.
    pub fn shutdown(self) {
        self.rt.shutdown().expect("runtime 종료");
    }
}

/// 프로젝트 `root`의 매니페스트 project id.
pub fn project_id_of(root: &Path) -> StableId {
    let snapshot = open_snapshot(root).expect("프로젝트 snapshot");
    snapshot.project_id()
}

/// 매니페스트 project id 텍스트를 StableId로.
pub fn parse_id(text: &str) -> StableId {
    parse_project_id(text).expect("UUID")
}

/// 디스크에서 새로 연 창 session(앱이 다음 실행에서 여는 것과 같은 열기 경로).
pub fn reopen(root: &Path) -> ProjectWindowSession {
    let identity = ProjectIdentity::new(project_id_of(root), root).expect("프로젝트 identity");
    let window = WindowSessionId::from_stable_id(StableId::new_v7(7_000_000, 0x0e2e, 1));
    ProjectWindowSession::load(identity, window).expect("창 session 열기")
}

/// [`reopen`]한 작업 문서의 semantic snapshot JSON.
pub fn reopen_json(root: &Path) -> String {
    json(reopen(root).document().expect("작업 문서").model())
}

/// 문서 semantic snapshot JSON(VI.10.4).
pub fn json(model: &DocumentModel) -> String {
    model_snapshot_json(model).expect("semantic snapshot")
}

/// headless(CLI·MCP) 경로로 읽은 저장 문서 하나의 semantic snapshot JSON.
pub fn headless_json(root: &Path, document: StableId) -> String {
    let snapshot = open_snapshot(root).expect("프로젝트 snapshot");
    json(&snapshot.document_model(document).expect("저장 문서"))
}

/// 시스템 Git 실행기(테스트 HOME의 전역 설정만 읽음)를 신뢰 gate로 감쌉니다. 신뢰 판정은 테스트가 정합니다
/// (사용자 신뢰 등록은 테스트 HOME에 없음). 시스템 Git이 없으면 `None`.
pub fn trusted_git() -> Option<GatedGitRunner> {
    test_home();
    let runner: GitRunner = system_git_runner().ok()?;
    Some(GatedGitRunner::new(runner, GitTrust::Trusted))
}

/// Git을 실행하고 성공을 강제합니다. stdout(UTF-8 lossy)을 돌려줍니다.
pub fn git(gate: &GatedGitRunner, dir: &Path, args: &[&str]) -> String {
    let argv = stellar_git_porcelain::runner::argv(args.iter().copied());
    let out = gate.run(&argv, dir).expect("git 실행");
    assert!(
        out.success(),
        "git {args:?} 실패: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `git status --porcelain=v1 -uall` 줄(경로 정렬).
pub fn porcelain(gate: &GatedGitRunner, dir: &Path) -> Vec<String> {
    let mut lines: Vec<String> = git(gate, dir, &["status", "--porcelain=v1", "-uall"])
        .lines()
        .map(str::to_owned)
        .collect();
    lines.sort();
    lines
}

/// Git 엔진 세션(신뢰 gate).
pub fn git_session(gate: &GatedGitRunner, root: &Path) -> GitSession {
    GitSession::open(gate.clone(), root).expect("Git 세션")
}

/// 프로젝트 폴더 전체(상대 경로 → bytes). `.git`은 뺍니다.
pub fn tree(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).expect("디렉터리 읽기") {
            let path = entry.expect("항목").path();
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(base).expect("상대 경로").to_path_buf(),
                    std::fs::read(&path).expect("파일 읽기"),
                );
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// revision 비교 helper.
pub fn rev(n: u64) -> DocumentRevision {
    DocumentRevision(n)
}
