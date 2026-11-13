import Foundation

// Git 패널이 코어에서 받는 값 snapshot 모음입니다(아키텍처 §8.2, §24.4, §26, §27).
//
// 모든 타입은 Rust 타입을 노출하지 않는 순수 Swift 값 타입입니다. 실제 코어 연결 시에는
// FFI 이벤트 큐(`stellar_runtime_poll_event`)에서 읽은 버전형 메시지를 이 값으로 디코딩합니다.
// 셸은 이 값을 표시만 하고 의미(Semantic Diff, Stage 의존성, Merge 판정)를 계산하지 않습니다.

// MARK: - 저장소 상태

/// Git 패널을 쓸 수 있는 상태인지 나타냅니다.
enum GitAvailability: Equatable {
    /// 신뢰된 프로젝트이고 시스템 Git이 있습니다.
    case ready
    /// 신뢰 전 제한 모드입니다. 저장소 안 시스템 Git 실행이 비활성입니다(아키텍처 §24.3).
    case restricted
    /// 시스템 Git을 찾지 못했습니다. 설치 안내만 하고 자동 설치하지 않습니다(아키텍처 §24.1).
    case gitMissing
    /// 열린 폴더가 Git 저장소가 아닙니다.
    case notRepository
}

/// HEAD 상태(XIV.2.2 RepositoryStatus의 셸 투영).
enum GitHeadState: Equatable {
    case branch(name: String)
    case detached(shortHash: String)
    /// 아직 커밋이 없는 브랜치입니다.
    case unborn(branch: String)
    /// 알 수 없음. 제한 모드에서는 저장소 안 Git을 실행하지 않으므로 HEAD를 읽지 않습니다(아키텍처 §24.3).
    case unknown
}

/// 진행 중인 Git 작업 상태. 셸은 자동 continue하지 않습니다(XIV.12.5).
enum GitMergeState: Equatable {
    case none
    case merging
    case rebasing
    case cherryPicking
}

/// upstream 추적 상태.
struct GitUpstreamInfo: Equatable {
    var remote: String
    var branch: String
    var ahead: Int
    var behind: Int
}

/// 페이지 단위로 받는 목록의 세대와 전체 행 수(XIV.13.1).
///
/// 세대가 바뀌면 이전 세대의 페이지·상세 결과는 버립니다(디자인 가이드 9장).
struct GitListInfo: Equatable {
    var generation: UInt64
    var rowCount: Int

    static let empty = GitListInfo(generation: 0, rowCount: 0)
}

/// 패널 전체 상태 snapshot. 작고 자주 바뀌지 않는 값만 담습니다.
struct GitStatusSnapshot: Equatable {
    var availability: GitAvailability
    var head: GitHeadState
    var upstream: GitUpstreamInfo?
    /// 이미 등록된 remote 이름(표시·선택용). URL은 민감값이므로 담지 않습니다(XIV.9.1).
    var remotes: [String]
    /// Git `user.name`·`user.email` 설정 여부(XIV.8.3).
    var identityConfigured: Bool
    /// Index에 들어간 semantic change 수(완전 staged + 부분 staged).
    var stagedChangeCount: Int
    /// Working Tree에만 있는 semantic change 수(unstaged + 부분 staged).
    var unstagedChangeCount: Int
    var mergeState: GitMergeState
    /// 아직 Git index에 unmerged로 남은 semantic conflict 수(선택 여부와 무관).
    var semanticConflictCount: Int
    /// 선택하지 않은 semantic conflict 수.
    var unresolvedConflictCount: Int
    /// Stellar 파일이 아닌 raw Git conflict 수(XIV.11.4).
    var rawConflictCount: Int
    /// staged snapshot이 유효한 Stellar 프로젝트가 아닐 때의 요약(XIV.8.1).
    var stagedSnapshotIssue: String?
    /// semantic status 계산 실패 요약. raw Git status는 계속 제공됩니다(XIV.4.5).
    var semanticStatusError: String?
    /// 저장 대기 중인 메모리 편집이 있는지(XIV.2.8). Git status는 저장된 파일 기준입니다.
    var pendingAutosave: Bool
    var changes: GitListInfo
    var conflicts: GitListInfo
}

// MARK: - 변경 목록

/// semantic change 종류(XIV.3.2)와 non-Stellar 파일 변경(XIV.4.4).
enum GitChangeKind: Equatable, CaseIterable {
    case addObject
    case removeObject
    case moveObject
    case reorderObject
    case setProperty
    case addToken
    case removeToken
    case changeToken
    case componentInterface
    case assetReference
    case rawFile
}

/// 한 변경 또는 묶음의 Stage 상태(XIV.4.1, XIV.4.2).
enum GitStageState: Equatable {
    /// HEAD == Index != Working Tree.
    case unstaged
    /// HEAD != Index == Working Tree.
    case staged
    /// HEAD != Index != Working Tree. 세 값이 모두 다릅니다(묶음에서는 일부만 staged).
    case partiallyStaged
}

/// 평탄화된 변경 목록의 행 계층. 문서 → 객체 → 변경 순서로 코어가 정렬합니다(XIV.3.8).
enum GitChangeRowLevel: Equatable {
    case document
    case object
    case change
}

/// 변경 목록 한 행. 묶음 행의 접힘 상태와 집계 Stage 상태도 코어가 계산합니다.
struct GitChangeRow: Equatable {
    var level: GitChangeRowLevel
    var title: String
    /// 보조 문구(값 변화 요약, 하위 변경 수 등). 코어가 만든 표시 문자열입니다.
    var detail: String
    /// 변경 행에서만 값이 있습니다.
    var kind: GitChangeKind?
    var stageState: GitStageState
    /// 묶음 행의 펼침 상태.
    var isExpanded: Bool
    /// 묶음 행의 하위 변경 수(변경 행은 1).
    var changeCount: Int
}

/// 변경 목록 한 페이지.
struct GitChangePage: Equatable {
    var generation: UInt64
    var startRow: Int
    var rows: [GitChangeRow]
}

/// 선택한 변경 한 개의 상세 요약(XIV.4.2: HEAD, Index, Working Tree 세 값 보존).
struct GitChangeDetail: Equatable {
    var generation: UInt64
    var row: Int
    var title: String
    var kind: GitChangeKind?
    /// 문서와 객체 경로(표시용).
    var objectPath: String
    /// `StableId + PropertyPath`의 PropertyPath 표시 문자열.
    var propertyPath: String?
    var headValue: String?
    var indexValue: String?
    var workingValue: String?
    /// 이 변경을 Stage하려면 함께 필요한 변경 요약(XIV.5.3).
    var requiredChanges: [String]
}

// MARK: - Conflict

/// semantic conflict 종류(아키텍처 §27.1, XIV.10.4~XIV.10.7)와 raw Git conflict(XIV.11.4).
enum GitConflictKind: Equatable, CaseIterable {
    case propertyValue
    case deleteVersusModify
    case deleteVersusMove
    case typeVersusValue
    case conflictingReorder
    case componentPropertyRemoved
    case bindingProviderRemoved
    case inconsistentDefinition
    case raw
}

/// 항목별 해결 선택. 허용 선택은 conflict 종류별로 코어가 제한합니다(XIV.12.1).
enum GitConflictChoice: Equatable, CaseIterable {
    case base
    case ours
    case theirs
}

/// conflict 목록 한 행. Base·Ours·Theirs 값 요약을 함께 받습니다.
struct GitConflictRow: Equatable {
    var title: String
    /// 문서·객체·속성 경로 표시 문자열.
    var path: String
    var kind: GitConflictKind
    /// 고를 수 있는 선택. raw conflict는 빈 집합입니다.
    var allowedChoices: Set<GitConflictChoice>
    var choice: GitConflictChoice?
    var baseSummary: String?
    var oursSummary: String?
    var theirsSummary: String?
}

/// conflict 목록 한 페이지.
struct GitConflictPage: Equatable {
    var generation: UInt64
    var startRow: Int
    var rows: [GitConflictRow]
}

// MARK: - 작업 진행과 결과

/// 메인 스레드 밖에서 실행되는 Git 작업 종류(아키텍처 §6.4).
enum GitOperationKind: Equatable {
    case refresh
    case stage
    case unstage
    case commit
    case push
    case applyResolutions
    case saveBeforeStatus
}

/// 진행 이벤트(XIV.13.2).
struct GitOperationProgress: Equatable {
    var id: UInt64
    var kind: GitOperationKind
    /// 0...1. nil이면 진행률을 모릅니다.
    var fraction: Double?
    var message: String
    var isCancellable: Bool
}

/// 작업 결과.
enum GitOperationOutcome: Equatable {
    case succeeded(message: String)
    /// Git이 반환한 결과를 그대로 요약합니다(hook 실패 우회 없음, XIV.8.5). 민감값은 코어가 제거합니다.
    case failed(message: String)
    case cancelled
}

struct GitOperationResult: Equatable {
    var id: UInt64
    var kind: GitOperationKind
    var outcome: GitOperationOutcome
}

/// 선택 변경만으로는 무결성이 깨질 때 코어가 돌려주는 필수 동반 변경(XIV.5.3, XIV.7.2).
/// 셸은 자동으로 포함하지 않고 사용자 승인을 받습니다.
struct GitDependencyPrompt: Equatable {
    var requestID: UInt64
    /// true면 Stage 요청, false면 Unstage 요청.
    var isStaging: Bool
    var requiredCount: Int
    /// 표시용 예시(최대 몇 개).
    var examples: [String]
}
