import Foundation

// Commit, Push, 해결 적용 버튼의 비활성 이유를 계산하는 순수 함수입니다.
//
// 이유는 우선순위 순서로 반환하고 패널은 첫 번째 이유를 버튼 아래에 표시합니다.
// 코어가 최종 판정을 다시 합니다(예: XIV.8.1 commit 전 재검증). 셸 규칙은 안내용입니다.

/// Commit 비활성 이유.
enum GitCommitBlocker: Equatable {
    case statusLoading
    case gitUnavailable
    /// 신뢰 전 제한 모드(아키텍처 §24.3, XIV.8.5).
    case restricted
    case operationInProgress
    /// Git identity 미설정(아키텍처 §25.1, XIV.8.3).
    case identityMissing
    /// unmerged 항목이 남아 있습니다.
    case unresolvedConflicts(count: Int)
    /// Stage한 변경이 없습니다.
    case nothingStaged
    /// staged snapshot이 유효하지 않습니다(XIV.8.1).
    case stagedSnapshotInvalid(reason: String)
    case emptyMessage
}

/// Push 비활성 이유.
enum GitPushBlocker: Equatable {
    case statusLoading
    case gitUnavailable
    case restricted
    case operationInProgress
    /// 등록된 remote가 없습니다. Remote 관리 UI는 초기 빌드에 없으므로 터미널을 안내합니다(XIV.13.7).
    case noRemote
    case detachedHead
    case nothingToPush
}

/// 해결 적용 비활성 이유.
enum GitApplyBlocker: Equatable {
    case statusLoading
    case gitUnavailable
    case restricted
    case operationInProgress
    case noConflicts
    case unresolved(count: Int)
}

enum GitPanelRules {
    static func commitBlockers(
        status: GitStatusSnapshot?,
        message: String,
        operationInProgress: Bool
    ) -> [GitCommitBlocker] {
        guard let status else { return [.statusLoading] }
        var blockers: [GitCommitBlocker] = []
        switch status.availability {
        case .restricted: blockers.append(.restricted)
        case .gitMissing, .notRepository: blockers.append(.gitUnavailable)
        case .ready: break
        }
        if operationInProgress { blockers.append(.operationInProgress) }
        if status.availability == .ready {
            if !status.identityConfigured { blockers.append(.identityMissing) }
            let unmerged = status.semanticConflictCount + status.rawConflictCount
            if unmerged > 0 { blockers.append(.unresolvedConflicts(count: unmerged)) }
            if status.stagedChangeCount == 0 { blockers.append(.nothingStaged) }
            if let issue = status.stagedSnapshotIssue { blockers.append(.stagedSnapshotInvalid(reason: issue)) }
        }
        if message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            blockers.append(.emptyMessage)
        }
        return blockers
    }

    static func pushBlockers(status: GitStatusSnapshot?, operationInProgress: Bool) -> [GitPushBlocker] {
        guard let status else { return [.statusLoading] }
        var blockers: [GitPushBlocker] = []
        switch status.availability {
        case .restricted: blockers.append(.restricted)
        case .gitMissing, .notRepository: blockers.append(.gitUnavailable)
        case .ready: break
        }
        if operationInProgress { blockers.append(.operationInProgress) }
        guard status.availability == .ready else { return blockers }
        if status.remotes.isEmpty { blockers.append(.noRemote) }
        switch status.head {
        case .detached: blockers.append(.detachedHead)
        case .unborn, .unknown: blockers.append(.nothingToPush)
        case .branch:
            // upstream이 없으면 선택한 remote에 같은 이름으로 처음 Push할 수 있습니다.
            if let upstream = status.upstream, upstream.ahead == 0 { blockers.append(.nothingToPush) }
        }
        return blockers
    }

    static func applyBlockers(status: GitStatusSnapshot?, operationInProgress: Bool) -> [GitApplyBlocker] {
        guard let status else { return [.statusLoading] }
        var blockers: [GitApplyBlocker] = []
        switch status.availability {
        case .restricted: blockers.append(.restricted)
        case .gitMissing, .notRepository: blockers.append(.gitUnavailable)
        case .ready: break
        }
        if operationInProgress { blockers.append(.operationInProgress) }
        guard status.availability == .ready else { return blockers }
        if status.semanticConflictCount == 0 {
            blockers.append(.noConflicts)
        } else if status.unresolvedConflictCount > 0 {
            blockers.append(.unresolved(count: status.unresolvedConflictCount))
        }
        return blockers
    }

    /// 묶음 행 체크박스를 눌렀을 때의 목표 Stage 값. 부분 staged는 전체 Stage로 갑니다.
    static func stagedTarget(afterToggling state: GitStageState) -> Bool {
        state != .staged
    }

    /// 시작을 VoiceOver에 알리는 작업. 짧은 Stage·Unstage·새로고침은 목록 변화로 충분하므로 알리지 않습니다.
    static func announcesProgress(_ kind: GitOperationKind) -> Bool {
        switch kind {
        case .commit, .push, .applyResolutions, .saveBeforeStatus: return true
        case .refresh, .stage, .unstage: return false
        }
    }
}
