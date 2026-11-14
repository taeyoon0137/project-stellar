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
    /// 이벤트 드레인 한 번의 메인 스레드 예산(초). 아키텍처 §6.3 단일 점유 2ms 안에 둡니다.
    static let mainThreadBudget: TimeInterval = 0.0015
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

/// 화면 문구. 현재 앱 UI 문구 관례(영문)를 따르며 한곳에 모아 지역화를 쉽게 합니다.
enum GitPanelStrings {
    static let panelTitle = "Source Control"
    static let changesTab = "Changes"
    static let conflictsTab = "Conflicts"
    static let changesListLabel = "Changes"
    static let conflictsListLabel = "Conflicts"
    static let modeLabel = "Source control view"
    static let refresh = "Refresh Status"
    static let loading = "Loading…"
    static let loadingStatus = "Loading Git status…"
    static let commitMessage = "Commit Message"
    static let commit = "Commit"
    static let push = "Push"
    static let pushRemote = "Push destination"
    static let applyResolutions = "Apply Resolutions"
    static let revealConflict = "Show on Canvas"
    static let details = "Details"
    static let selectChange = "Select a change to see its details."
    static let selectConflict = "Select a conflict to compare Base, Ours, and Theirs."
    static let cancel = "Cancel"
    static let dismiss = "Dismiss"
    static let trustProject = "Trust Project…"
    static let setUpIdentity = "Set Up Git Identity…"
    static let installHelp = "Show Git Installation Help…"
    static let saveNow = "Save Now"
    static let useSavedVersion = "Use Saved Version"
    static let noChangesTitle = "No Changes"
    static let noChangesMessage = "There are no changes since the last commit."
    static let noConflictsTitle = "No Conflicts"
    static let noConflictsMessage = "There are no merge conflicts to resolve."
    static let rawConflictHint = "Resolve this file in Terminal or another tool."
    static let stage = "Stage"
    static let unstage = "Unstage"
    static let base = "Base"
    static let ours = "Ours"
    static let theirs = "Theirs"
    static let kindLabel = "Kind"
    static let objectLabel = "Object"
    static let propertyLabel = "Property"
    static let headLabel = "HEAD"
    static let indexLabel = "Staged"
    static let workingLabel = "Working"
    static let requiresLabel = "Requires"
    static let continueInTerminal = "After applying, continue the merge or rebase in Terminal."

    static let restrictedBanner = "Git is turned off because this project is in Restricted Mode."
    static let gitMissingBanner = "Git isn't installed. Stellar uses the Git installed on your Mac."
    static let notRepositoryBanner = "This project folder isn't a Git repository."
    static let pendingAutosaveBanner = "Some edits aren't saved yet. Git status shows the saved files."

    static let documentLevel = "Document"
    static let objectLevel = "Object"

    /// 변경 행 하나를 VoiceOver가 한 번에 읽는 문구: 제목, 보조 문구, 종류, Stage 상태.
    static func changeRowLabel(title: String, detail: String, kind: String, state: String) -> String {
        [title, detail, kind, state].filter { !$0.isEmpty }.joined(separator: ", ")
    }
    static func resolution(_ title: String) -> String { "Resolution for \(title)" }
    static func conflictsTab(count: Int) -> String { count > 0 ? "Conflicts (\(count))" : conflictsTab }
    static func changeCount(_ count: Int) -> String { count == 1 ? "1 change" : "\(count) changes" }
    static func selectedCount(_ count: Int) -> String { "\(count) items selected" }
    static func aheadBehind(ahead: Int, behind: Int) -> String {
        "\(ahead) to push, \(behind) to pull"
    }
    static func statusSummary(staged: Int, unstaged: Int) -> String {
        "\(staged) staged, \(unstaged) unstaged"
    }
    static func pushInfo(ahead: Int, remote: String, branch: String) -> String {
        ahead == 1 ? "1 commit to push to \(remote)/\(branch)." : "\(ahead) commits to push to \(remote)/\(branch)."
    }
    static func pushNewBranch(remote: String) -> String { "Push this branch to \(remote) for the first time." }
    static func dependency(_ prompt: GitDependencyPrompt) -> String {
        let verb = prompt.isStaging ? "stage" : "unstage"
        let noun = prompt.requiredCount == 1 ? "1 related change" : "\(prompt.requiredCount) related changes"
        let examples = prompt.examples.isEmpty ? "" : " (\(prompt.examples.joined(separator: ", ")))"
        return "To \(verb) this selection, \(noun) must be included\(examples)."
    }
    static func includeRequired(_ prompt: GitDependencyPrompt) -> String {
        prompt.requiredCount == 1 ? "Include 1 Related Change" : "Include \(prompt.requiredCount) Related Changes"
    }

    static func head(_ head: GitHeadState) -> String {
        switch head {
        case .branch(let name): return name
        case .detached(let hash): return "Detached at \(hash)"
        case .unborn(let branch): return "\(branch) (no commits)"
        case .unknown: return panelTitle
        }
    }

    static func mergeState(_ state: GitMergeState) -> String? {
        switch state {
        case .none: return nil
        case .merging: return "Merging"
        case .rebasing: return "Rebasing"
        case .cherryPicking: return "Cherry-picking"
        }
    }

    static func kind(_ kind: GitChangeKind) -> String {
        switch kind {
        case .addObject: return "Added Object"
        case .removeObject: return "Removed Object"
        case .moveObject: return "Moved Object"
        case .reorderObject: return "Reordered"
        case .setProperty: return "Property Changed"
        case .addToken: return "Token Added"
        case .removeToken: return "Token Removed"
        case .changeToken: return "Token Changed"
        case .componentInterface: return "Component Interface Changed"
        case .assetReference: return "Asset Reference Changed"
        case .rawFile: return "File Changed"
        }
    }

    static func conflictKind(_ kind: GitConflictKind) -> String {
        switch kind {
        case .propertyValue: return "Both Changed"
        case .deleteVersusModify: return "Deleted vs. Modified"
        case .deleteVersusMove: return "Deleted vs. Moved"
        case .typeVersusValue: return "Type vs. Value"
        case .conflictingReorder: return "Conflicting Order"
        case .componentPropertyRemoved: return "Component Property Removed"
        case .bindingProviderRemoved: return "Binding Provider Removed"
        case .inconsistentDefinition: return "Inconsistent Definition"
        case .raw: return "File Conflict"
        }
    }

    static func choice(_ choice: GitConflictChoice) -> String {
        switch choice {
        case .base: return base
        case .ours: return ours
        case .theirs: return theirs
        }
    }

    static func stageState(_ state: GitStageState) -> String {
        switch state {
        case .unstaged: return "Not staged"
        case .staged: return "Staged"
        case .partiallyStaged: return "Partially staged"
        }
    }

    static func commitBlocker(_ blocker: GitCommitBlocker) -> String {
        switch blocker {
        case .statusLoading: return "Loading Git status…"
        case .gitUnavailable: return "Git isn't available for this project."
        case .restricted: return "Trust this project to commit."
        case .operationInProgress: return "Wait for the current Git operation to finish."
        case .identityMissing: return "Set your Git name and email to commit."
        case .unresolvedConflicts(let count):
            return count == 1 ? "Resolve 1 conflict before committing." : "Resolve \(count) conflicts before committing."
        case .nothingStaged: return "Stage at least one change to commit."
        case .stagedSnapshotInvalid(let reason): return reason
        case .emptyMessage: return "Enter a commit message."
        }
    }

    static func pushBlocker(_ blocker: GitPushBlocker) -> String {
        switch blocker {
        case .statusLoading: return "Loading Git status…"
        case .gitUnavailable: return "Git isn't available for this project."
        case .restricted: return "Trust this project to push."
        case .operationInProgress: return "Wait for the current Git operation to finish."
        case .noRemote: return "No remote is set up. Add one in Terminal to push."
        case .detachedHead: return "Check out a branch to push."
        case .nothingToPush: return "No commits to push."
        }
    }

    static func applyBlocker(_ blocker: GitApplyBlocker) -> String {
        switch blocker {
        case .statusLoading: return "Loading Git status…"
        case .gitUnavailable: return "Git isn't available for this project."
        case .restricted: return "Trust this project to resolve conflicts."
        case .operationInProgress: return "Wait for the current Git operation to finish."
        case .noConflicts: return "There are no semantic conflicts to apply."
        case .unresolved(let count):
            return count == 1 ? "Choose a resolution for 1 conflict." : "Choose resolutions for \(count) conflicts."
        }
    }
}
