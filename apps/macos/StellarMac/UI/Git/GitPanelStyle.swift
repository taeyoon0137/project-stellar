import AppKit

/// Git 패널 간격·페이지 상수(디자인 가이드 4.4, 9장).
///
/// 간격 값은 잠정값입니다. 가이드 4.4는 패널 바깥 여백, 행 간격, 컨트롤 묶음 간격을 모든 패널이
/// 공유하는 상수 하나씩으로 두라고 하므로, 통합 시 공통 상수로 옮기고 가이드 12장에 기록해야 합니다.
/// 이 패널은 묶음 간격 대신 구분선과 행 간격으로 구획을 나눕니다.
enum GitPanelMetrics {
    /// 패널 바깥 여백.
    static let outerInset: CGFloat = 12
    /// 패널 안 행 간격.
    static let rowSpacing: CGFloat = 6
    /// 한 줄 안 요소 간격.
    static let inlineSpacing: CGFloat = 4
    /// 목록 행 위아래 여백. 계층 들여쓰기는 NSOutlineView 시스템 값을 씁니다.
    static let rowPadding: CGFloat = 3

    /// 한 번에 요청하는 목록 행 수(XIV.13.1).
    static let pageSize = 256
    /// 메모리에 유지하는 페이지 수 상한. 넘으면 오래 쓰지 않은 페이지부터 버립니다.
    static let pageCapacity = 32
    /// 이벤트 드레인 한 번에서 이벤트 처리에 쓰는 메인 스레드 예산(초, 잠정값). 같은 차례의 UI 반영 몫을 남겨
    /// 아키텍처 §6.3 단일 점유 2ms 안에 둡니다.
    static let eventBudget: TimeInterval = 0.001
    /// 이벤트 처리가 이 시간 안에 끝났을 때만 같은 런 루프 차례에서 UI를 반영합니다. 넘으면 다음 차례로 넘겨
    /// 이벤트 처리와 UI 반영이 한 차례 점유(아키텍처 §6.3 2ms)에 겹치지 않게 합니다.
    static let inlineUIThreshold: TimeInterval = 0.0005
    /// 목록 영역이 최소한 보여 줄 행 수.
    static let minimumVisibleRows: CGFloat = 4
    /// 커밋 메시지 입력의 기본 줄 수.
    static let commitMessageLines: CGFloat = 4
    /// 상태 표시 전환 애니메이션 시간(동작 줄이기에서는 0).
    static let revealDuration: TimeInterval = 0.2
}

/// 패널이 쓰는 SF Symbols 이름. 이미지 asset은 쓰지 않습니다(디자인 가이드 5.4).
enum GitPanelSymbols {
    static let branch = "arrow.triangle.branch"
    static let ahead = "arrow.up"
    static let behind = "arrow.down"
    static let refresh = "arrow.clockwise"
    static let document = "doc.text"
    static let object = "square.on.square.dashed"
    static let restricted = "lock.shield"
    static let warning = "exclamationmark.triangle"
    static let error = "xmark.octagon"
    static let info = "info.circle"
    static let clean = "checkmark.circle"
    static let save = "square.and.arrow.down"
    static let dependency = "link"
    static let reveal = "scope"
    static let rawFile = "doc"

    static func kind(_ kind: GitChangeKind) -> String {
        switch kind {
        case .addObject: return "plus.square"
        case .removeObject: return "minus.square"
        case .moveObject: return "arrow.up.and.down.and.arrow.left.and.right"
        case .reorderObject: return "arrow.up.arrow.down"
        case .setProperty: return "slider.horizontal.3"
        case .addToken, .changeToken: return "tag"
        case .removeToken: return "tag.slash"
        case .componentInterface: return "puzzlepiece"
        case .assetReference: return "photo"
        case .rawFile: return rawFile
        }
    }

    static func conflictKind(_ kind: GitConflictKind) -> String {
        kind == .raw ? rawFile : warning
    }

    /// 검사용 전체 목록.
    static var all: [String] {
        [branch, ahead, behind, refresh, document, object, restricted, warning, error, info, clean,
         save, dependency, reveal, rawFile]
            + GitChangeKind.allCases.map(kind)
            + GitConflictKind.allCases.map(conflictKind)
    }

    @MainActor private static var imageCache: [String: NSImage] = [:]

    /// 시스템 심볼 이미지. 접근성 설명을 함께 붙이고, 목록 행 재구성 비용을 줄이려고 캐시합니다.
    @MainActor
    static func image(_ name: String, label: String) -> NSImage? {
        let key = "\(name)\u{1F}\(label)"
        if let cached = imageCache[key] { return cached }
        guard let image = NSImage(systemSymbolName: name, accessibilityDescription: label) else { return nil }
        imageCache[key] = image
        return image
    }
}

/// 화면 문구. 현재 앱 UI 문구 관례(영문)를 따르고, 사이드바·시작 창과 같이 `String(localized:comment:)`로
/// 지역화 키를 둡니다. 문장 조각을 이어 붙이지 않고 문장 단위로 둡니다. 수에 따른 단·복수는 지금은 1과 그 밖의
/// 두 문장으로 나누며, 언어별 복수 규칙은 문자열 카탈로그를 들일 때 정합니다(11장).
enum GitPanelStrings {
    static let panelTitle = String(localized: "Source Control", comment: "Git 패널 제목·접근성 그룹 이름")
    static let changesTab = String(localized: "Changes", comment: "Git 패널 화면 전환: 변경 화면")
    static let conflictsTab = String(localized: "Conflicts", comment: "Git 패널 화면 전환: conflict 화면")
    static let changesListLabel = String(localized: "Changes", comment: "변경 outline 접근성 레이블")
    static let conflictsListLabel = String(localized: "Conflicts", comment: "conflict 목록 접근성 레이블")
    static let modeLabel = String(localized: "Source control view", comment: "화면 전환 컨트롤 접근성 레이블")
    static let refresh = String(localized: "Refresh Status", comment: "Git 상태 새로고침 버튼")
    static let loading = String(localized: "Loading…", comment: "값을 읽는 중 자리 표시")
    static let loadingStatus = String(localized: "Loading Git status…", comment: "첫 상태를 읽는 중 안내")
    static let commitMessage = String(localized: "Commit Message", comment: "커밋 메시지 입력 레이블")
    static let commit = String(localized: "Commit", comment: "커밋 버튼")
    static let push = String(localized: "Push", comment: "Push 버튼")
    static let pushRemote = String(localized: "Push destination", comment: "Push 대상 remote 팝업 레이블")
    static let applyResolutions = String(localized: "Apply Resolutions", comment: "conflict 해결 적용 버튼")
    static let revealConflict = String(localized: "Show on Canvas", comment: "conflict 대상을 캔버스에서 보기")
    static let details = String(localized: "Details", comment: "상세 구획 제목")
    static let selectChange = String(localized: "Select a change to see its details.", comment: "변경 상세 자리 표시")
    static let selectConflict = String(
        localized: "Select a conflict to compare Base, Ours, and Theirs.", comment: "conflict 상세 자리 표시")
    static let cancel = String(localized: "Cancel", comment: "취소 버튼")
    static let dismiss = String(localized: "Dismiss", comment: "결과 배너 닫기 버튼")
    static let trustProject = String(localized: "Trust Project…", comment: "제한 모드 프로젝트 신뢰 부여 버튼")
    static let setUpIdentity = String(localized: "Set Up Git Identity…", comment: "Git 이름·이메일 설정 안내 버튼")
    static let installHelp = String(localized: "Show Git Installation Help…", comment: "Git 설치 안내 버튼")
    static let saveNow = String(localized: "Save Now", comment: "저장 대기 편집을 먼저 저장")
    static let useSavedVersion = String(localized: "Use Saved Version", comment: "저장된 파일 기준으로 계속")
    static let noChangesTitle = String(localized: "No Changes", comment: "변경 없음 빈 상태 제목")
    static let noChangesMessage = String(localized: "There are no changes since the last commit.", comment: "변경 없음 빈 상태 설명")
    static let noConflictsTitle = String(localized: "No Conflicts", comment: "conflict 없음 빈 상태 제목")
    static let noConflictsMessage = String(localized: "There are no merge conflicts to resolve.", comment: "conflict 없음 빈 상태 설명")
    static let restrictedTitle = String(localized: "Restricted Mode", comment: "제한 모드 안내 제목")
    static let gitMissingTitle = String(localized: "Git Not Found", comment: "시스템 Git 없음 안내 제목")
    static let noRepositoryTitle = String(localized: "No Repository", comment: "Git 저장소 아님 안내 제목")
    static let rawConflictHint = String(
        localized: "Resolve this file in Terminal or another tool.", comment: "Stellar 파일이 아닌 conflict 안내")
    static let stage = String(localized: "Stage", comment: "Stage 동작(문맥 메뉴·접근성 동작)")
    static let unstage = String(localized: "Unstage", comment: "Unstage 동작(문맥 메뉴·접근성 동작)")
    static let base = String(localized: "Base", comment: "conflict 공통 조상 쪽")
    static let ours = String(localized: "Ours", comment: "conflict 현재 브랜치 쪽")
    static let theirs = String(localized: "Theirs", comment: "conflict 병합 대상 쪽")
    static let kindLabel = String(localized: "Kind", comment: "상세: 변경·conflict 종류")
    static let objectLabel = String(localized: "Object", comment: "상세: 문서·객체 경로")
    static let propertyLabel = String(localized: "Property", comment: "상세: 속성 경로")
    static let headLabel = String(localized: "HEAD", comment: "상세: HEAD 값")
    static let indexLabel = String(localized: "Staged", comment: "상세: Index 값")
    static let workingLabel = String(localized: "Working", comment: "상세: Working Tree 값")
    static let requiresLabel = String(localized: "Requires", comment: "상세: 함께 필요한 변경")
    static let statusLabel = String(localized: "Status", comment: "상세: Stage 상태")
    static let pathLabel = String(localized: "Path", comment: "conflict 상세: 경로")
    static let choiceLabel = String(localized: "Choice", comment: "conflict 상세: 고른 쪽")
    static let notChosen = String(localized: "Not chosen", comment: "conflict 상세: 아직 고르지 않음")
    static let continueInTerminal = String(
        localized: "After applying, continue the merge or rebase in Terminal.", comment: "해결 적용 뒤 continue 안내")

    static let restrictedBanner = String(
        localized: "Git is turned off because this project is in Restricted Mode.", comment: "제한 모드 안내 설명")
    static let gitMissingBanner = String(
        localized: "Git isn't installed. Stellar uses the Git installed on your Mac.", comment: "시스템 Git 없음 설명")
    static let notRepositoryBanner = String(
        localized: "This project folder isn't a Git repository.", comment: "Git 저장소 아님 설명")
    static let pendingAutosaveBanner = String(
        localized: "Some edits aren't saved yet. Git status shows the saved files.", comment: "저장 대기 편집 배너")

    static let documentLevel = String(localized: "Document", comment: "변경 계층: 문서 묶음")
    static let objectLevel = String(localized: "Object", comment: "변경 계층: 객체 묶음")

    // 아이콘 접근성 설명.
    static let branchSymbol = String(localized: "Branch", comment: "브랜치 아이콘 설명")
    static let aheadSymbol = String(localized: "To push", comment: "Push 대기 아이콘 설명")
    static let behindSymbol = String(localized: "To pull", comment: "Pull 대기 아이콘 설명")
    static let unsavedEditsSymbol = String(localized: "Unsaved edits", comment: "저장 대기 배너 아이콘 설명")
    static let statusErrorSymbol = String(localized: "Status error", comment: "semantic status 오류 배너 아이콘 설명")
    static let relatedChangesSymbol = String(localized: "Related changes", comment: "필수 동반 변경 배너 아이콘 설명")
    static let inProgressSymbol = String(localized: "In progress", comment: "진행 배너 아이콘 설명")
    static let resultSymbol = String(localized: "Result", comment: "결과 배너 아이콘 설명")
    static let errorSymbol = String(localized: "Error", comment: "실패 결과 아이콘 설명")
    static let doneSymbol = String(localized: "Done", comment: "성공 결과 아이콘 설명")

    static let stageRelatedTitle = String(localized: "Stage Related Changes Too?", comment: "필수 동반 변경 Stage 확인 제목")
    static let unstageRelatedTitle = String(
        localized: "Unstage Related Changes Too?", comment: "필수 동반 변경 Unstage 확인 제목")

    /// 변경 행 하나를 VoiceOver가 한 번에 읽는 문구: 제목, 보조 문구, 종류, Stage 상태.
    static func changeRowLabel(title: String, detail: String, kind: String, state: String) -> String {
        [title, detail, kind, state].filter { !$0.isEmpty }.joined(separator: ", ")
    }
    static func resolution(_ title: String) -> String {
        String(localized: "Resolution for \(title)", comment: "conflict 행 해결 선택 컨트롤 접근성 레이블")
    }
    static func conflictsTab(count: Int) -> String {
        count > 0 ? String(localized: "Conflicts (\(count))", comment: "conflict 수가 붙은 화면 전환 제목") : conflictsTab
    }
    static func changeCount(_ count: Int) -> String {
        count == 1
            ? String(localized: "1 change", comment: "semantic change 1개")
            : String(localized: "\(count) changes", comment: "semantic change 여러 개")
    }
    static func selectedCount(_ count: Int) -> String {
        String(localized: "\(count) items selected", comment: "여러 행 선택 시 상세 자리 표시")
    }
    static func aheadBehind(ahead: Int, behind: Int) -> String {
        String(localized: "\(ahead) to push, \(behind) to pull", comment: "Push·Pull 대기 수 도움말 태그")
    }
    static func aheadCount(_ count: Int) -> String {
        String(localized: "\(count) to push", comment: "Push 대기 수 접근성 레이블")
    }
    static func behindCount(_ count: Int) -> String {
        String(localized: "\(count) to pull", comment: "Pull 대기 수 접근성 레이블")
    }
    static func statusSummary(staged: Int, unstaged: Int) -> String {
        String(localized: "\(staged) staged, \(unstaged) unstaged", comment: "Stage 요약")
    }
    static func commitReady(changes: Int, branch: String) -> String {
        changes == 1
            ? String(localized: "Commit 1 change to \(branch).", comment: "커밋할 수 있을 때 안내(변경 1개)")
            : String(localized: "Commit \(changes) changes to \(branch).", comment: "커밋할 수 있을 때 안내")
    }
    static func applyReady(_ count: Int) -> String {
        count == 1
            ? String(localized: "Apply 1 resolution and mark it resolved.", comment: "해결 적용할 수 있을 때 안내(1개)")
            : String(localized: "Apply \(count) resolutions and mark them resolved.", comment: "해결 적용할 수 있을 때 안내")
    }
    static func pushInfo(ahead: Int, remote: String, branch: String) -> String {
        ahead == 1
            ? String(localized: "1 commit to push to \(remote)/\(branch).", comment: "Push 대기 커밋 안내(1개)")
            : String(localized: "\(ahead) commits to push to \(remote)/\(branch).", comment: "Push 대기 커밋 안내")
    }
    static func pushNewBranch(remote: String) -> String {
        String(localized: "Push this branch to \(remote) for the first time.", comment: "upstream 없는 첫 Push 안내")
    }
    static func dependency(_ prompt: GitDependencyPrompt) -> String {
        let examples = prompt.examples.joined(separator: ", ")
        let count = prompt.requiredCount
        switch (prompt.isStaging, count == 1, examples.isEmpty) {
        case (true, true, true):
            return String(localized: "To stage this selection, 1 related change must be included.", comment: "필수 동반 변경(Stage, 1개)")
        case (true, true, false):
            return String(
                localized: "To stage this selection, 1 related change must be included (\(examples)).",
                comment: "필수 동반 변경(Stage, 1개, 예시)")
        case (true, false, true):
            return String(
                localized: "To stage this selection, \(count) related changes must be included.", comment: "필수 동반 변경(Stage)")
        case (true, false, false):
            return String(
                localized: "To stage this selection, \(count) related changes must be included (\(examples)).",
                comment: "필수 동반 변경(Stage, 예시)")
        case (false, true, true):
            return String(
                localized: "To unstage this selection, 1 related change must be included.", comment: "필수 동반 변경(Unstage, 1개)")
        case (false, true, false):
            return String(
                localized: "To unstage this selection, 1 related change must be included (\(examples)).",
                comment: "필수 동반 변경(Unstage, 1개, 예시)")
        case (false, false, true):
            return String(
                localized: "To unstage this selection, \(count) related changes must be included.", comment: "필수 동반 변경(Unstage)")
        case (false, false, false):
            return String(
                localized: "To unstage this selection, \(count) related changes must be included (\(examples)).",
                comment: "필수 동반 변경(Unstage, 예시)")
        }
    }
    static func includeRequired(_ prompt: GitDependencyPrompt) -> String {
        prompt.requiredCount == 1
            ? String(localized: "Include 1 Related Change", comment: "필수 동반 변경 포함 버튼(1개)")
            : String(localized: "Include \(prompt.requiredCount) Related Changes", comment: "필수 동반 변경 포함 버튼")
    }

    static func head(_ head: GitHeadState) -> String {
        switch head {
        case .branch(let name): return name
        case .detached(let hash): return String(localized: "Detached at \(hash)", comment: "detached HEAD 표시")
        case .unborn(let branch): return String(localized: "\(branch) (no commits)", comment: "커밋 없는 브랜치 표시")
        case .unknown: return panelTitle
        }
    }

    static func mergeState(_ state: GitMergeState) -> String? {
        switch state {
        case .none: return nil
        case .merging: return String(localized: "Merging", comment: "진행 중인 Git 작업: merge")
        case .rebasing: return String(localized: "Rebasing", comment: "진행 중인 Git 작업: rebase")
        case .cherryPicking: return String(localized: "Cherry-picking", comment: "진행 중인 Git 작업: cherry-pick")
        }
    }

    static func kind(_ kind: GitChangeKind) -> String {
        switch kind {
        case .addObject: return String(localized: "Added Object", comment: "변경 종류")
        case .removeObject: return String(localized: "Removed Object", comment: "변경 종류")
        case .moveObject: return String(localized: "Moved Object", comment: "변경 종류")
        case .reorderObject: return String(localized: "Reordered", comment: "변경 종류")
        case .setProperty: return String(localized: "Property Changed", comment: "변경 종류")
        case .addToken: return String(localized: "Token Added", comment: "변경 종류")
        case .removeToken: return String(localized: "Token Removed", comment: "변경 종류")
        case .changeToken: return String(localized: "Token Changed", comment: "변경 종류")
        case .componentInterface: return String(localized: "Component Interface Changed", comment: "변경 종류")
        case .assetReference: return String(localized: "Asset Reference Changed", comment: "변경 종류")
        case .rawFile: return String(localized: "File Changed", comment: "변경 종류: Stellar 파일이 아닌 파일")
        }
    }

    static func conflictKind(_ kind: GitConflictKind) -> String {
        switch kind {
        case .propertyValue: return String(localized: "Both Changed", comment: "conflict 종류")
        case .deleteVersusModify: return String(localized: "Deleted vs. Modified", comment: "conflict 종류")
        case .deleteVersusMove: return String(localized: "Deleted vs. Moved", comment: "conflict 종류")
        case .typeVersusValue: return String(localized: "Type vs. Value", comment: "conflict 종류")
        case .conflictingReorder: return String(localized: "Conflicting Order", comment: "conflict 종류")
        case .componentPropertyRemoved: return String(localized: "Component Property Removed", comment: "conflict 종류")
        case .bindingProviderRemoved: return String(localized: "Binding Provider Removed", comment: "conflict 종류")
        case .inconsistentDefinition: return String(localized: "Inconsistent Definition", comment: "conflict 종류")
        case .raw: return String(localized: "File Conflict", comment: "conflict 종류: Stellar 파일이 아닌 파일")
        }
    }

    static func choice(_ choice: GitConflictChoice) -> String {
        switch choice {
        case .base: return base
        case .ours: return ours
        case .theirs: return theirs
        }
    }

    static func useChoice(_ choice: GitConflictChoice) -> String {
        switch choice {
        case .base: return String(localized: "Use Base", comment: "conflict 문맥 메뉴")
        case .ours: return String(localized: "Use Ours", comment: "conflict 문맥 메뉴")
        case .theirs: return String(localized: "Use Theirs", comment: "conflict 문맥 메뉴")
        }
    }

    static func stageState(_ state: GitStageState) -> String {
        switch state {
        case .unstaged: return String(localized: "Not staged", comment: "Stage 상태")
        case .staged: return String(localized: "Staged", comment: "Stage 상태")
        case .partiallyStaged: return String(localized: "Partially staged", comment: "Stage 상태")
        }
    }

    private static let statusLoadingReason = String(localized: "Loading Git status…", comment: "비활성 이유: 상태 읽는 중")
    private static let gitUnavailableReason = String(
        localized: "Git isn't available for this project.", comment: "비활성 이유: Git 사용 불가")
    private static let operationInProgressReason = String(
        localized: "Wait for the current Git operation to finish.", comment: "비활성 이유: 작업 진행 중")

    static func commitBlocker(_ blocker: GitCommitBlocker) -> String {
        switch blocker {
        case .statusLoading: return statusLoadingReason
        case .gitUnavailable: return gitUnavailableReason
        case .restricted: return String(localized: "Trust this project to commit.", comment: "Commit 비활성 이유: 제한 모드")
        case .operationInProgress: return operationInProgressReason
        case .identityMissing:
            return String(localized: "Set your Git name and email to commit.", comment: "Commit 비활성 이유: identity 없음")
        case .unresolvedConflicts(let count):
            return count == 1
                ? String(localized: "Resolve 1 conflict before committing.", comment: "Commit 비활성 이유: conflict 1개")
                : String(localized: "Resolve \(count) conflicts before committing.", comment: "Commit 비활성 이유: conflict")
        case .nothingStaged:
            return String(localized: "Stage at least one change to commit.", comment: "Commit 비활성 이유: Stage 없음")
        case .stagedSnapshotInvalid(let reason): return reason
        case .emptyMessage: return String(localized: "Enter a commit message.", comment: "Commit 비활성 이유: 메시지 없음")
        }
    }

    static func pushBlocker(_ blocker: GitPushBlocker) -> String {
        switch blocker {
        case .statusLoading: return statusLoadingReason
        case .gitUnavailable: return gitUnavailableReason
        case .restricted: return String(localized: "Trust this project to push.", comment: "Push 비활성 이유: 제한 모드")
        case .operationInProgress: return operationInProgressReason
        case .noRemote:
            return String(localized: "No remote is set up. Add one in Terminal to push.", comment: "Push 비활성 이유: remote 없음")
        case .detachedHead: return String(localized: "Check out a branch to push.", comment: "Push 비활성 이유: detached HEAD")
        case .nothingToPush: return String(localized: "No commits to push.", comment: "Push 비활성 이유: 커밋 없음")
        }
    }

    static func applyBlocker(_ blocker: GitApplyBlocker) -> String {
        switch blocker {
        case .statusLoading: return statusLoadingReason
        case .gitUnavailable: return gitUnavailableReason
        case .restricted:
            return String(localized: "Trust this project to resolve conflicts.", comment: "해결 적용 비활성 이유: 제한 모드")
        case .operationInProgress: return operationInProgressReason
        case .noConflicts:
            return String(localized: "There are no semantic conflicts to apply.", comment: "해결 적용 비활성 이유: conflict 없음")
        case .unresolved(let count):
            return count == 1
                ? String(localized: "Choose a resolution for 1 conflict.", comment: "해결 적용 비활성 이유: 미선택 1개")
                : String(localized: "Choose resolutions for \(count) conflicts.", comment: "해결 적용 비활성 이유: 미선택")
        }
    }
}
