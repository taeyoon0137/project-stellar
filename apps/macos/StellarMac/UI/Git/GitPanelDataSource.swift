import Foundation

// Git 패널과 코어 사이의 경계입니다(아키텍처 §8.2, ADR-0009).
//
// - 명령(command)은 셸 → 코어 단방향입니다. 호출은 즉시 반환하고 실제 Git 작업은 메인 스레드 밖에서
//   실행합니다(아키텍처 §6.4, 디자인 가이드 6.4).
// - 결과는 pull 방식 이벤트 큐로 받습니다. 셸은 `pollEvent()`로 하나씩 꺼냅니다. callback으로 코어가
//   셸을 재진입하지 않습니다(ADR-0009 "이벤트").
// - 큰 목록은 세대(generation)와 범위로 페이지 단위 요청합니다(XIV.13.1). 변경 목록은 부모 노드별 자식
//   범위로, conflict 목록은 행 범위로 요청합니다. 부모·순번 → 변경 ID 해석은 코어가 같은 세대 기준으로 합니다.
// - 세대가 다르거나 지금 실행할 수 없는 명령은 코어가 버리고 `.commandRejected`로 알립니다. 셸은 미리 보여 준
//   값을 되돌리고 요청 중 표시를 지웁니다. 조용히 버리지 않습니다.

/// 셸 → 코어 명령.
///
/// 강제 Push 명령은 의도적으로 없습니다(아키텍처 §25.3, XIV.9.5).
/// Branch, Pull, Fetch, Merge, Rebase, Stash, Worktree, Remote 관리 명령도 없습니다(XIV.13.7).
enum GitPanelCommand: Equatable {
    /// 저장소 상태를 다시 계산합니다. 결과는 `.status` 이벤트입니다.
    case refreshStatus
    /// 한 부모(nil이면 최상위 문서 목록)의 자식 페이지를 요청합니다. 결과는 `.changePage` 이벤트입니다.
    case loadChangePage(generation: UInt64, parent: GitChangeNodeID?, children: Range<Int>)
    /// 변경 노드 하나의 상세를 요청합니다. 결과는 `.changeDetail` 이벤트입니다.
    case loadChangeDetail(generation: UInt64, node: GitChangeNodeID)
    /// 범위(묶음 노드는 하위 전체)를 Stage 또는 Unstage합니다(XIV.13.2).
    case setStaged(generation: UInt64, targets: [GitChangeRange], staged: Bool)
    /// `.dependencyPrompt`에 대한 사용자 응답입니다.
    case answerDependencyPrompt(requestID: UInt64, includeRequired: Bool)
    /// staged snapshot을 커밋합니다. 메시지는 Git에 stdin·임시 파일로 전달됩니다(XIV.8.2).
    case commit(message: String)
    /// 이미 등록된 remote로 Push합니다. 강제 옵션은 없습니다.
    case push(remote: String)
    /// conflict 목록 페이지를 요청합니다. 결과는 `.conflictPage` 이벤트입니다.
    case loadConflictPage(generation: UInt64, rows: Range<Int>)
    /// conflict 행의 해결 선택을 candidate snapshot에만 기록합니다(XIV.12.2).
    case chooseResolution(generation: UInt64, rows: IndexSet, choice: GitConflictChoice)
    /// 모든 선택을 검증하고 파일 저장 후 `git add`로 resolved 처리합니다(XIV.12.3, XIV.12.4).
    /// merge·rebase continue와 commit은 실행하지 않습니다(XIV.12.5).
    case applyResolutions
    /// conflict 대상 객체를 캔버스에서 보여 줍니다(아키텍처 §27.2).
    case revealConflict(generation: UInt64, row: Int)
    /// 저장 대기 중인 편집을 먼저 저장할지, 디스크 버전으로 계속할지 답합니다(XIV.2.8).
    case resolvePendingAutosave(saveFirst: Bool)
    /// 취소 가능한 작업을 취소합니다.
    case cancelOperation(id: UInt64)
}

/// 코어 → 셸 이벤트.
enum GitPanelEvent: Equatable {
    case status(GitStatusSnapshot)
    case changePage(GitChangePage)
    case changeDetail(GitChangeDetail)
    case conflictPage(GitConflictPage)
    case dependencyPrompt(GitDependencyPrompt)
    case operationProgress(GitOperationProgress)
    case operationFinished(GitOperationResult)
    /// 코어가 실행하지 않고 버린 명령(세대 불일치 등).
    case commandRejected(GitCommandRejection)
}

/// Git 패널이 필요로 하는 데이터·동작 경계.
///
/// 모든 메서드는 메인 스레드에서 호출되며 즉시 반환해야 합니다(아키텍처 §6.3 단일 점유 2ms).
/// 구현은 새 이벤트가 생기면 메인 스레드에서 `onEventsAvailable`을 호출해 패널을 깨웁니다.
/// 실제 코어 구현에서는 셸의 runtime 이벤트 펌프가 이 hook을 호출합니다.
@MainActor
protocol GitPanelDataSource: AnyObject {
    /// 새 이벤트가 큐에 들어왔음을 알리는 hook. 패널이 설정합니다.
    var onEventsAvailable: (() -> Void)? { get set }
    /// 명령을 보냅니다. 블로킹하지 않습니다.
    func send(_ command: GitPanelCommand)
    /// 이벤트 큐에서 다음 이벤트를 FIFO로 꺼냅니다. 비었으면 nil입니다.
    func pollEvent() -> GitPanelEvent?
}

/// 코어가 아니라 셸의 다른 화면이 처리하는 동작입니다.
///
/// 신뢰 부여 UI, Git identity 설정 안내, Git 설치 안내는 각각 별도 셸 화면의 몫이며 이 패널은
/// 진입점만 제공합니다(디자인 가이드 6.4, 7.2).
@MainActor
protocol GitPanelHostActions: AnyObject {
    /// 제한 모드 프로젝트의 신뢰 부여 UI를 엽니다(아키텍처 §24.3, XVI.7.2).
    func gitPanelRequestsProjectTrust()
    /// Git `user.name`·`user.email` 설정 안내를 엽니다. 전역 config를 자동 수정하지 않습니다(XIV.8.3).
    func gitPanelRequestsIdentitySetup()
    /// 시스템 Git 설치 안내를 엽니다. 자동 설치하지 않습니다(아키텍처 §24.1).
    func gitPanelRequestsGitInstallHelp()
}
