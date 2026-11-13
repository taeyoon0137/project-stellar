import Foundation

// 시작 창(Welcome) 영역의 데이터 경계.
//
// 셸은 코어에서 plain value snapshot을 받아 그리기만 하고, 사용자 동작은 `WelcomeCommand`로
// 내보냅니다. Rust 타입을 노출하지 않으므로, 이후 어댑터가 FFI pull 이벤트(`stellar_runtime_poll_event`)를
// 읽어 이 snapshot으로 바꾸고 명령을 C ABI 호출로 옮길 수 있습니다(아키텍처 §8.2, ADR-0009).
// 경로는 C 문자열로 옮기기 쉽도록 `String`으로 둡니다. 화면 표시 전용이며 진단·텔레메트리에 넣지 않습니다(§39).

/// 시작 창이 그리는 상태 전체(코어 → 셸).
struct WelcomeSnapshot: Equatable {
    /// snapshot 세대 번호. 셸은 이미 그린 세대보다 오래된 snapshot을 무시합니다(아키텍처 §9.2).
    var generation: UInt64
    /// 최근 프로젝트 목록 상태.
    var recentProjects: WelcomeRecentProjectsState
    /// 시스템 Git 탐색 결과(아키텍처 §24.1).
    var git: WelcomeGitState
    /// 진행 중인 생성·열기·Clone 작업. 없으면 nil.
    var activity: WelcomeActivity?
    /// 한 번만 보여 줄 안내·오류. 셸이 표시한 뒤 `dismissNotice`로 확인합니다.
    var notice: WelcomeNotice?
    /// 새 프로젝트·Clone 대상 폴더의 기본 상위 디렉터리. nil이면 셸이 사용자 문서 폴더를 씁니다.
    var defaultParentDirectory: String?
}

/// 최근 프로젝트 목록의 상태.
enum WelcomeRecentProjectsState: Equatable {
    /// 코어가 목록을 읽는 중입니다.
    case loading
    /// 목록을 읽었습니다. 빈 배열이면 빈 상태를 보여 줍니다. 순서는 코어가 정합니다(최근 사용 순).
    case loaded([WelcomeRecentProject])
    /// 목록을 읽지 못했습니다. `detail`은 보조 설명으로만 표시합니다.
    case failed(detail: String)
}

/// 최근 프로젝트 한 행.
struct WelcomeRecentProject: Equatable {
    /// 코어가 발급한 불투명 식별자. 명령에 그대로 돌려줍니다.
    var id: String
    /// 표시 이름(보통 프로젝트 폴더 이름).
    var displayName: String
    /// 프로젝트 폴더 절대 경로. 표시와 Finder 보기에만 씁니다.
    var path: String
    /// 마지막 확인 시 폴더가 없었으면 true.
    var isMissing: Bool
}

/// 시스템 Git 상태(아키텍처 §24.1). 셸은 설치를 실행하지 않고 안내만 합니다.
enum WelcomeGitState: Equatable {
    /// 탐색 중입니다. 안내를 띄우지 않습니다.
    case checking
    /// 사용할 수 있는 Git을 찾았습니다.
    case available(version: String)
    /// 사용자 지정 경로, 표준 설치 경로, PATH 어디에서도 Git을 찾지 못했습니다.
    case notFound
    /// Git을 찾았지만 버전·기능 검사를 통과하지 못했습니다.
    case unusable(detail: String)

    /// 새 프로젝트(git init)와 Clone에 Git을 쓸 수 있는지. 탐색 중에는 막지 않습니다.
    var allowsGitOperations: Bool {
        switch self {
        case .checking, .available: return true
        case .notFound, .unusable: return false
        }
    }
}

/// 진행 중 작업.
struct WelcomeActivity: Equatable {
    enum Kind: Equatable {
        case creating
        case opening
        case cloning
    }

    var kind: Kind
    /// 대상 프로젝트 표시 이름. Clone URL은 넣지 않습니다(§39 Git URL은 민감값).
    var projectName: String
    /// 0...1 진행률. 모르면 nil(불확정 표시).
    var fractionCompleted: Double?
    /// 취소할 수 있으면 true(Git 명령 취소, 계획 XIV.1.4).
    var isCancellable: Bool
}

/// 한 번만 표시할 안내·오류. 문구는 셸이 종류 코드에서 만듭니다(지역화 대비).
struct WelcomeNotice: Equatable {
    enum Kind: Equatable {
        /// `stellar.yaml`이 없는 폴더·Git 저장소는 전환하지 않고 안내만 합니다(아키텍처 §25.2).
        case notStellarProject(path: String)
        /// 최근 목록의 폴더가 없어졌습니다.
        case projectMissing(path: String)
        /// 새 프로젝트·Clone 대상 폴더가 이미 있습니다.
        case folderAlreadyExists(path: String)
        /// 최초 커밋 전에 Git `user.name`·`user.email`이 필요합니다(아키텍처 §25.1).
        case gitIdentityRequired
        /// 그 밖의 실패. `detail`은 보조 설명입니다.
        case operationFailed(detail: String)
    }

    /// 같은 안내를 두 번 띄우지 않기 위한 식별자.
    var id: UInt64
    var kind: Kind
}

/// 시작 창에서 나가는 명령(셸 → 코어). 결과는 다음 snapshot으로 돌아옵니다.
enum WelcomeCommand: Equatable {
    /// 상위 디렉터리와 새 폴더 이름으로 프로젝트를 만듭니다(아키텍처 §25.1, 계획 XII.6.1).
    case createProject(parentDirectory: String, folderName: String)
    /// 기존 프로젝트 폴더를 엽니다(계획 XII.6.2).
    case openProject(directory: String)
    /// 원격 저장소를 Clone한 뒤 엽니다(계획 XII.6.3).
    case cloneProject(remoteURL: String, parentDirectory: String, folderName: String)
    /// 최근 목록의 프로젝트를 엽니다.
    case openRecent(id: String)
    /// 최근 목록에서 뺍니다. 프로젝트 폴더는 건드리지 않습니다.
    case removeRecent(id: String)
    /// 최근 목록을 다시 읽습니다.
    case reloadRecentProjects
    /// Git 탐색을 다시 실행합니다.
    case recheckGit
    /// 사용자가 고른 Git 실행 파일을 탐색 1순위로 지정합니다(아키텍처 §24.1 탐색 순서 1).
    case useGitExecutable(path: String)
    /// 진행 중 작업을 취소합니다.
    case cancelActivity
    /// 표시한 안내를 확인했습니다.
    case dismissNotice(id: UInt64)
}

/// 시작 창 데이터 공급자.
///
/// 모든 호출은 메인 스레드에서 합니다. 구현은 코어 이벤트를 메인 스레드 밖에서 pull해 snapshot을
/// 만들어 두고, 바뀌면 `snapshotDidChange`를 메인 스레드에서 호출합니다. `currentSnapshot()`은
/// 미리 만든 값을 돌려주기만 해야 합니다(UI 스레드 단일 점유 2ms, 아키텍처 §6.3).
protocol WelcomeDataSource: AnyObject {
    /// 최신 snapshot.
    func currentSnapshot() -> WelcomeSnapshot
    /// snapshot이 바뀌었음을 알립니다. 셸은 다음 런 루프에서 한 번만 다시 그립니다.
    var snapshotDidChange: (() -> Void)? { get set }
    /// 명령을 보냅니다. 즉시 반환해야 하며 결과는 snapshot으로 돌아옵니다.
    func send(_ command: WelcomeCommand)
}
