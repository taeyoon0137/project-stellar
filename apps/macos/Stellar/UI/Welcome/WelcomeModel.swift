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
    /// 목록을 읽지 못했습니다. 문구는 셸이 이유 코드에서 만듭니다.
    case failed(WelcomeFailureReason)
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
    /// Git을 찾았지만 버전·기능 검사를 통과하지 못했습니다. 문구는 셸이 문제 코드에서 만듭니다.
    case unusable(WelcomeGitProblem)

    /// 새 프로젝트(git init)와 Clone에 Git을 쓸 수 있는지. 탐색 중에는 막지 않습니다.
    var allowsGitOperations: Bool {
        switch self {
        case .checking, .available: return true
        case .notFound, .unusable: return false
        }
    }
}

/// Git을 쓸 수 없는 이유(코어 → 셸 코드, 아키텍처 §24.1).
enum WelcomeGitProblem: Equatable {
    /// 찾은 Git이 Stellar가 요구하는 최소 버전보다 낮습니다. 버전 문자열은 값이므로 그대로 표시합니다.
    case versionTooOld(found: String, minimum: String)
    /// 지정한 경로가 실행 파일이 아닙니다.
    case notExecutable(path: String)
    /// Git을 실행했지만 버전 확인에 실패했습니다.
    case launchFailed
}

/// 작업 실패 이유 코드. 코어 오류는 어댑터가 이 코드로 바꾸고, 문구는 셸이 만듭니다(지역화 대비).
enum WelcomeFailureReason: Equatable {
    /// 폴더를 읽거나 쓸 권한이 없습니다.
    case permissionDenied(path: String)
    /// 디스크 공간이 부족합니다.
    case notEnoughSpace
    /// 원격 저장소에 연결하지 못했습니다.
    case networkUnavailable
    /// 원격 저장소 인증이 필요하거나 실패했습니다(자격 증명은 시스템 Git이 다룸, 아키텍처 §37.3).
    case authenticationFailed
    /// 원격 저장소를 찾지 못했습니다.
    case remoteNotFound
    /// Git 명령이 실패했습니다. 종료 상태는 진단용 숫자입니다.
    case gitCommandFailed(exitStatus: Int32)
    /// 그 밖의 코어 오류. `code`는 사람이 읽는 문장이 아닌 안정 식별자이며 진단 표시용입니다.
    case internalError(code: String)
    /// 프로젝트 열기 실패(XII.6.2). 원인은 셸 어댑터가 코어 실패 코드에서 고른 `ShellProjectOpenFailure`이고, 문구는
    /// 그 원인의 제목과 해결 방법입니다. `path`는 고른 폴더입니다(문구에는 폴더 이름만 씀).
    case projectOpenFailed(ShellProjectOpenFailure, path: String)
    case projectCreationFailed(message: String)
    /// Git이 checkout을 만들지 못했거나 대상 경로에 쓸 수 없습니다.
    case projectCloneFailed
}

/// 진행 중 작업.
struct WelcomeActivity: Equatable {
    enum Kind: Equatable {
        case creating
        case opening
        case cloning
    }

    /// 코어가 발급한 작업 식별자. 취소 명령에 그대로 돌려주어, 끝난 작업 대신 새 작업을 취소하는 일을 막습니다.
    var id: UInt64
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
        /// 그 밖의 실패. 문구는 셸이 이유 코드에서 만듭니다.
        case operationFailed(WelcomeFailureReason)
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
    /// 진행 중 작업을 취소합니다. 식별자가 현재 작업과 다르면 코어가 무시합니다.
    case cancelActivity(id: UInt64)
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
