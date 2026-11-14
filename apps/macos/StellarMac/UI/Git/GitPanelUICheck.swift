import AppKit

/// Git 패널 offscreen 검사(XIV.13.5 UI smoke의 정적 부분).
///
/// Stub data source로 패널을 화면에 표시하지 않고 만든 뒤 일반 크기에서 layout을 강제하고 다음을 확인합니다.
/// - 모든 SF Symbols 이름이 시스템에 있음
/// - 표시 중인 뷰에 모호한 Auto Layout이 없음(Light·Dark, 변경·conflict 화면)
/// - 핵심 컨트롤에 접근성 레이블이 있고, 변경 행은 하나의 접근성 요소(제목·종류·Stage 상태 + custom action)임
/// - 변경 목록이 `NSOutlineView` 계층이며 수준·펼침 상태가 outline에 있음
/// - Commit 비활성 이유(Stage 없음, identity 없음, 신뢰 전 프로젝트)와 Push·해결 적용 이유
/// - Stage 체크박스·필수 동반 변경 확인·키보드(Space, ←/→, B/O/T)·Commit·Push·Conflict 해결 흐름
/// - 코어가 거부한 명령(세대 불일치)에서 미리 보여 준 체크박스가 되돌아가고 같은 페이지를 다시 요청할 수 있음
/// - 구조가 바뀐 뒤(커밋) 펼침 상태가 노드 ID로 복원되고(복원 차례 하나에 구조 변경 하나), 빠진 노드의 펼침 ID는 버려짐
/// - 사용자가 시작한 작업 명령이 거부되면 오류 배너와 높은 우선순위 알림으로 전함
/// - 이벤트 처리가 느리면 UI 반영을 다음 런 루프 차례로 넘김
/// - 100만 change와 자식이 여러 페이지인 묶음에서 보이는 페이지만 요청하고 캐시·행 뷰 수가 제한됨(XIV.13.1)
/// 실제 Git, 코어 FFI, 화면 표시, VoiceOver 읽기 순서, sheet 경로는 검사하지 않습니다.
enum GitPanelUICheck {
    static func run() -> Bool {
        MainActor.assumeIsolated { runOnMain() }
    }

    private struct Failure: Error {
        let message: String
    }

    @MainActor
    private static func runOnMain() -> Bool {
        // 창과 텍스트 시스템을 쓰려면 NSApplication이 필요합니다. Dock 아이콘은 띄우지 않습니다.
        NSApplication.shared.setActivationPolicy(.accessory)
        var report: [String] = []
        do {
            try checkSymbols(&report)
            try checkTypical(&report)
            try checkCommitBlockers(&report)
            try checkRestricted(&report)
            try checkClean(&report)
            try checkConflicts(&report)
            try checkMillion(&report)
            try checkWide(&report)
            try checkExpansionMemory(&report)
            try checkDeferredUI(&report)
            try checkRejectionFeedback(&report)
            try checkKeyboard(&report)
        } catch let failure as Failure {
            FileHandle.standardError.write(Data("git-panel 실패: \(failure.message)\n".utf8))
            return false
        } catch {
            FileHandle.standardError.write(Data("git-panel 실패: \(error)\n".utf8))
            return false
        }
        print("git-panel ok: \(report.joined(separator: "; "))")
        return true
    }

    private static func expect(_ condition: Bool, _ message: @autoclosure () -> String) throws {
        if !condition { throw Failure(message: message()) }
    }

    /// 화살표 같은 기능 키의 문자열.
    private static func functionKey(_ code: Int) -> String {
        UnicodeScalar(UInt32(code)).map { String(Character($0)) } ?? ""
    }

    // MARK: 검사 환경

    @MainActor
    private final class HostRecorder: GitPanelHostActions {
        var trustRequests = 0
        var identityRequests = 0
        var installHelpRequests = 0
        func gitPanelRequestsProjectTrust() { trustRequests += 1 }
        func gitPanelRequestsIdentitySetup() { identityRequests += 1 }
        func gitPanelRequestsGitInstallHelp() { installHelpRequests += 1 }
    }

    @MainActor
    private final class Harness {
        let dataSource: GitPanelStubDataSource
        let host = HostRecorder()
        let controller: GitPanelViewController
        let window: NSWindow
        /// VoiceOver 알림 기록(문구, 우선순위).
        private(set) var announcements: [(String, NSAccessibilityPriorityLevel)] = []

        /// 일반적인 사이드바·인스펙터 폭과 창 높이에서 만듭니다.
        init(_ scenario: GitPanelStubScenario, size: NSSize = NSSize(width: 320, height: 760), appearance: NSAppearance.Name = .aqua) {
            dataSource = GitPanelStubDataSource(scenario: scenario)
            controller = GitPanelViewController(dataSource: dataSource, host: host)
            window = NSWindow(
                contentRect: NSRect(origin: .zero, size: size),
                styleMask: [.titled, .resizable],
                backing: .buffered,
                defer: false
            )
            window.isReleasedWhenClosed = false
            window.appearance = NSAppearance(named: appearance)
            controller.announcementHandler = { [weak self] message, priority in self?.announcements.append((message, priority)) }
            window.contentViewController = controller
            window.setContentSize(size)
            layout()
        }

        var list: GitChangeListController { controller.changeList }
        var outline: GitChangeOutlineView { controller.changeList.outlineView }

        func layout() {
            window.contentView?.layoutSubtreeIfNeeded()
        }

        func announced(_ fragment: String, priority: NSAccessibilityPriorityLevel? = nil) -> Bool {
            announcements.contains { $0.0.contains(fragment) && (priority == nil || $0.1 == priority) }
        }

        /// 메인 런 루프를 돌려 Stub 이벤트를 처리합니다. 조건이 맞은 뒤에는 미뤄 둔 UI·상세·복원 반영(다음 런 루프
        /// 차례로 넘긴 반영)을 마저 끝내 화면 상태를 확인할 수 있게 합니다. 그 사이 조건이 바뀌면 멈춥니다.
        @discardableResult
        func spin(timeout: TimeInterval = 10, until condition: () -> Bool) -> Bool {
            let deadline = Date().addingTimeInterval(timeout)
            while !condition() {
                if Date() > deadline { return false }
                RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.005))
                layout()
            }
            while controller.hasPendingWork, condition(), Date() <= deadline {
                RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.005))
                layout()
            }
            layout()
            return true
        }

        /// 보이는 변경 행이 모두 fresh 페이지로 채워질 때까지 기다립니다.
        func waitForVisibleChangeRows(timeout: TimeInterval = 10) -> Bool {
            spin(timeout: timeout) {
                guard list.documentCount > 0 else { return false }
                let visible = outline.rows(in: outline.visibleRect)
                guard visible.length > 0 else { return false }
                return (visible.location..<(visible.location + visible.length)).allSatisfy { row in
                    list.item(atRow: row).flatMap { list.row(of: $0) } != nil
                }
            }
        }

        func changeCell(_ item: GitChangeOutlineItem) -> GitChangeCellView? {
            let row = outline.row(forItem: item)
            guard row >= 0 else { return nil }
            outline.scrollRowToVisible(row)
            layout()
            return outline.view(atColumn: 0, row: row, makeIfNecessary: true) as? GitChangeCellView
        }

        /// 항목을 선택하고 outline에 키를 보냅니다(포커스 포함).
        func select(_ item: GitChangeOutlineItem) {
            let row = outline.row(forItem: item)
            guard row >= 0 else { return }
            outline.scrollRowToVisible(row)
            outline.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
            window.makeFirstResponder(outline)
            layout()
        }

        /// 사용자 펼침 한 번(`expandItem` + 그 뒤 layout)의 최대 시간.
        private(set) var maxUserExpandSeconds: TimeInterval = 0

        /// 항목을 펼치고(사용자 펼침과 같은 AppKit 경로) 자식 행 페이지가 올 때까지 기다립니다.
        func expand(_ item: GitChangeOutlineItem, timeout: TimeInterval = 10) -> Bool {
            let start = ProcessInfo.processInfo.systemUptime
            outline.expandItem(item)
            layout()
            maxUserExpandSeconds = max(maxUserExpandSeconds, ProcessInfo.processInfo.systemUptime - start)
            return outline.isItemExpanded(item) && waitForVisibleChangeRows(timeout: timeout)
        }

        /// 부모의 n번째 자식 항목(펼쳐져 있어야 함).
        func child(_ index: Int, of parent: GitChangeOutlineItem?) -> GitChangeOutlineItem? {
            outline.child(index, ofItem: parent) as? GitChangeOutlineItem
        }

        /// 항목 행이 fresh로 올 때까지 스크롤해 기다립니다.
        func loadedRow(_ item: GitChangeOutlineItem, timeout: TimeInterval = 10) -> GitChangeRow? {
            let row = outline.row(forItem: item)
            guard row >= 0 else { return nil }
            outline.scrollRowToVisible(row)
            layout()
            spin(timeout: timeout) { list.row(of: item) != nil }
            return list.row(of: item)
        }

        /// 펼친 문서 안에서 새 객체(AddObject가 unstaged)의 unstaged 속성 변경을 찾아 그 객체를 펼쳐 둡니다.
        /// 이 속성만 Stage하면 코어가 필수 동반 변경 확인을 보냅니다.
        func dependentProperty(in document: GitChangeOutlineItem) -> GitChangeOutlineItem? {
            for index in 0..<outline.numberOfChildren(ofItem: document) {
                guard let object = child(index, of: document), let header = loadedRow(object),
                      header.detail.hasPrefix("New")
                else { continue }
                guard expand(object), let add = child(0, of: object), let property = child(1, of: object),
                      loadedRow(add)?.kind == .addObject, loadedRow(add)?.stageState == .unstaged,
                      loadedRow(property)?.stageState == .unstaged
                else { continue }
                return property
            }
            return nil
        }

        func key(_ characters: String, keyCode: UInt16, to view: NSView) {
            guard let event = NSEvent.keyEvent(
                with: .keyDown,
                location: .zero,
                modifierFlags: [],
                timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber,
                context: nil,
                characters: characters,
                charactersIgnoringModifiers: characters,
                isARepeat: false,
                keyCode: keyCode
            ) else { return }
            view.keyDown(with: event)
        }

        func typeCommitMessage(_ text: String) {
            window.makeFirstResponder(controller.commitTextView)
            controller.commitTextView.insertText(text, replacementRange: NSRange(location: NSNotFound, length: 0))
        }

        func close() {
            window.close()
        }
    }

    /// 표시 중인 뷰 중 모호한 Auto Layout을 가진 뷰를 찾습니다.
    ///
    /// `NSTextView` 자체는 검사하지만 그 안의 비공개 TextKit 2 하위 뷰(`_NSTextContentView` 등)는
    /// 내려가지 않습니다. 이 하위 뷰는 제약으로 배치한 scroll view 안의 기본 `NSTextView`에서도
    /// 항상 모호하다고 보고되며(셸 코드와 무관, 2026-11-14 macOS 26 SDK 확인) 시스템이 frame으로 관리합니다.
    @MainActor
    private static func ambiguousViews(in root: NSView) -> [String] {
        var result: [String] = []
        func visit(_ view: NSView) {
            guard !view.isHidden else { return }
            if view.hasAmbiguousLayout {
                result.append("\(type(of: view))(\(view.accessibilityLabel() ?? ""))")
            }
            if view is NSTextView { return }
            view.subviews.forEach(visit)
        }
        visit(root)
        return result
    }

    @MainActor
    private static func expectNoAmbiguity(_ harness: Harness, _ context: String) throws {
        guard let root = harness.window.contentView else { throw Failure(message: "\(context): contentView 없음") }
        let ambiguous = ambiguousViews(in: root)
        try expect(ambiguous.isEmpty, "\(context): 모호한 Auto Layout \(ambiguous.prefix(5))")
    }

    /// 창 안에 있고 자신·조상이 숨겨지지 않은 뷰.
    @MainActor
    private static func isShown(_ view: NSView) -> Bool {
        view.window != nil && !view.isHiddenOrHasHiddenAncestor
    }

    @MainActor
    private static func hasLabel(_ view: NSView) -> Bool {
        let label = view.accessibilityLabel() ?? ""
        let title = (view as? NSButton)?.title ?? ""
        return !label.isEmpty || !title.isEmpty
    }

    @MainActor
    private static func expectAccessibility(_ harness: Harness, _ context: String) throws {
        let panel = harness.controller
        let controls: [(String, NSView)] = [
            ("refresh", panel.refreshButton),
            ("mode", panel.modeControl),
            ("changes list", panel.changeList.outlineView),
            ("conflicts list", panel.conflictList.tableView),
            ("commit message", panel.commitTextView),
            ("commit", panel.commitButton),
            ("push", panel.pushButton),
            ("push destination", panel.remotePopup),
            ("apply", panel.applyButton),
            ("reveal", panel.revealButton),
            ("panel", panel.view),
        ]
        for (name, control) in controls {
            try expect(hasLabel(control), "\(context): \(name) 접근성 레이블 없음")
        }
        try expect(!(panel.refreshButton.toolTip ?? "").isEmpty, "\(context): 아이콘 버튼 도움말 태그 없음")

        // 변경 행: 셀 하나가 요소이고(레이블에 제목과 Stage 상태), 하위 뷰는 요소가 아니며, Stage 동작이 custom action.
        var problems: [String] = []
        let outline = panel.changeList.outlineView
        outline.enumerateAvailableRowViews { rowView, row in
            guard let cell = rowView.view(atColumn: 0) as? GitChangeCellView,
                  let item = panel.changeList.item(atRow: row), let value = panel.changeList.row(of: item)
            else { return }
            let label = cell.accessibilityLabel() ?? ""
            if !label.contains(value.title)
                || !label.contains(GitPanelStrings.stageState(value.stageState)) {
                problems.append("row \(row) label '\(label)'")
            }
            // NSControl의 접근성은 cell이 노출하므로 view와 cell을 함께 확인합니다.
            let parts: [NSControl] = [cell.stageCheckbox, cell.kindImageView, cell.titleField, cell.detailField]
            if parts.contains(where: { $0.isAccessibilityElement() || $0.cell?.isAccessibilityElement() == true }) {
                problems.append("row \(row) 하위 요소 노출")
            }
            if panel.changeList.isInteractive, (cell.accessibilityCustomActions() ?? []).isEmpty {
                problems.append("row \(row) custom action 없음")
            }
            if cell.stageCheckbox.acceptsFirstResponder {
                problems.append("row \(row) 체크박스가 Tab 순서에 있음")
            }
        }
        panel.conflictList.tableView.enumerateAvailableRowViews { rowView, row in
            guard let cell = rowView.view(atColumn: 0) as? GitConflictCellView else { return }
            if !cell.choiceControl.isHidden, (cell.choiceControl.accessibilityLabel() ?? "").isEmpty {
                problems.append("conflict \(row) choice")
            }
        }
        try expect(problems.isEmpty, "\(context): 접근성 \(problems.prefix(5))")
    }

    /// 패널이 런 루프 차례로 나눈 작업이 메인 스레드를 한 차례에 점유한 최대 시간(이벤트 드레인, UI 반영, 상세,
    /// 미룬 outline 다시 읽기, 펼침·선택 복원). 사용자 펼침(검사가 `expandItem`을 직접 부름)은 들어 있지 않고
    /// `Harness.maxUserExpandSeconds`로 따로 잽니다.
    @MainActor
    private static func maxTurn(_ panel: GitPanelViewController) -> TimeInterval {
        max(panel.diagnostics.maxTurnSeconds, panel.changeList.maxStructureReloadSeconds, panel.changeList.maxRestoreSeconds)
    }

    /// Debug 빌드 회귀 방지 상한. 계약(아키텍처 §6.3 단일 점유 2ms, Release·기준 하드웨어)의 두 배입니다.
    private static let debugTurnLimit: TimeInterval = 0.004

    /// 런 루프 차례별 최대 점유(ms) 보고 문구.
    @MainActor
    private static func occupancy(_ panel: GitPanelViewController) -> String {
        let d = panel.diagnostics
        func ms(_ seconds: TimeInterval) -> String { String(format: "%.2f", seconds * 1000) }
        let list = panel.changeList
        return "max drain \(ms(d.maxDrainSeconds))ms / UI \(ms(d.maxUIUpdateSeconds))ms / detail \(ms(d.maxDetailSeconds))ms / outline reload \(ms(list.maxStructureReloadSeconds))ms / restore \(ms(list.maxRestoreSeconds))ms (restored \(list.restoredStructureChanges), deferred UI \(d.deferredUIUpdates))"
    }

    // MARK: 개별 검사

    @MainActor
    private static func checkSymbols(_ report: inout [String]) throws {
        let missing = GitPanelSymbols.all.filter { NSImage(systemSymbolName: $0, accessibilityDescription: nil) == nil }
        try expect(missing.isEmpty, "없는 SF Symbols: \(missing)")
        report.append("symbols \(Set(GitPanelSymbols.all).count)")
    }

    @MainActor
    private static func checkTypical(_ report: inout [String]) throws {
        let harness = Harness(.typical)
        defer { harness.close() }
        let panel = harness.controller
        let list = harness.list
        let outline = harness.outline
        try expect(harness.waitForVisibleChangeRows(), "typical: 변경 목록 페이지가 오지 않음")
        guard let initial = panel.status else { throw Failure(message: "typical: status 없음") }

        // 구조: 표준 AppKit outline·텍스트 컨트롤. 처음에는 문서 행만 보입니다(펼침은 보기 상태).
        try expect(outline.outlineTableColumn != nil && outline.accessibilityRole() == .outline,
                   "typical: 변경 목록이 NSOutlineView 계층이 아님")
        try expect(outline.numberOfRows == initial.changes.documentCount && outline.numberOfRows == 6,
                   "typical: 최상위 행 수 불일치 \(outline.numberOfRows)")
        try expect(panel.commitScrollView.documentView === panel.commitTextView, "typical: 커밋 메시지가 NSTextView가 아님")
        try expect(!panel.commitTextView.isRichText && panel.commitTextView.allowsUndo, "typical: 커밋 메시지 plain text·undo 아님")
        // ⌘Return은 패널 루트 뷰가 포커스를 확인해 처리합니다. 버튼 키 등가는 창 전체에서 발동하므로 두지 않습니다.
        try expect(panel.commitButton.keyEquivalent.isEmpty && panel.view is GitPanelRootView,
                   "typical: Commit 버튼에 창 전체 키 등가가 있음")

        try expectNoAmbiguity(harness, "typical light")
        try expectAccessibility(harness, "typical")

        // 저장 대기 편집 안내(XIV.2.8). 배너가 나타나면 VoiceOver에도 같은 문구를 알립니다.
        try expect(initial.pendingAutosave && isShown(panel.autosaveBanner), "typical: 저장 대기 배너가 보이지 않음")
        try expect(harness.announced(GitPanelStrings.pendingAutosaveBanner), "typical: 저장 대기 알림 없음")

        // Push·Pull 대기 수: 두 숫자를 각각 읽고(같은 문장 두 번 아님), 도움말 태그는 전체 문장입니다.
        let aheadLabel = panel.aheadLabel.accessibilityLabel() ?? ""
        let behindLabel = panel.behindLabel.accessibilityLabel() ?? ""
        try expect(aheadLabel == GitPanelStrings.aheadCount(2) && behindLabel == GitPanelStrings.behindCount(0)
                   && panel.aheadLabel.toolTip == GitPanelStrings.aheadBehind(ahead: 2, behind: 0),
                   "typical: Push·Pull 레이블 '\(aheadLabel)' '\(behindLabel)'")

        // 대비 높이기: 보조 문구·보조 색 아이콘(머리 줄, 진행 배너, 목록 행)을 label 색으로 올리고 되돌립니다.
        let normal = panel.displayOptions
        panel.apply(displayOptions: GitDisplayOptions(reduceMotion: true, increaseContrast: true, reduceTransparency: false))
        let firstCell = list.item(atRow: 0).flatMap { harness.changeCell($0) }
        try expect(panel.summaryLabel.textColor == .labelColor && panel.branchIcon.contentTintColor == .labelColor
                   && panel.progressBanner.iconView.contentTintColor == .labelColor
                   && panel.resultBanner.iconView.contentTintColor == .labelColor
                   && panel.semanticErrorBanner.iconView.contentTintColor == .systemOrange
                   && firstCell?.kindImageView.contentTintColor == .labelColor,
                   "typical: 대비 높이기 색이 반영되지 않음")
        try expect(panel.displayOptions.revealDuration == 0, "typical: 동작 줄이기에서 전환 시간이 0이 아님")
        panel.apply(displayOptions: GitDisplayOptions(reduceMotion: false, increaseContrast: false, reduceTransparency: false))
        try expect(panel.summaryLabel.textColor == .secondaryLabelColor && panel.progressBanner.iconView.contentTintColor == .secondaryLabelColor
                   && firstCell?.kindImageView.contentTintColor == .secondaryLabelColor,
                   "typical: 대비 높이기를 끈 뒤 색이 돌아가지 않음")
        panel.apply(displayOptions: normal)

        // 선택 강조 행: 종류 아이콘 tint를 비워 시스템 대비 색으로 그립니다.
        if let cell = firstCell {
            cell.backgroundStyle = .emphasized
            try expect(cell.kindImageView.contentTintColor == nil, "typical: 강조 행 아이콘 색이 고정됨")
            cell.backgroundStyle = .normal
            try expect(cell.kindImageView.contentTintColor == panel.displayOptions.secondaryTextColor,
                       "typical: 강조 해제 뒤 아이콘 색이 돌아가지 않음")
        }

        panel.saveNowButton.performClick(nil)
        try expect(harness.spin { panel.status?.pendingAutosave == false && !isShown(panel.autosaveBanner) },
                   "typical: Save Now 후 배너가 사라지지 않음")

        // 메시지가 비어 있으면 Commit 비활성.
        try expect(panel.commitBlockers == [.emptyMessage] && !panel.commitButton.isEnabled,
                   "typical: 빈 메시지 비활성 이유 불일치 \(panel.commitBlockers)")
        harness.typeCommitMessage("Update button colors")
        try expect(panel.commitBlockers.isEmpty && panel.commitButton.isEnabled, "typical: 메시지 입력 후 Commit 비활성")

        // 키보드 →: 첫 문서 펼치기(NSOutlineView 기본 동작). 객체 4개가 수준 1로 나타납니다.
        guard let document = list.item(atRow: 0) else { throw Failure(message: "typical: 문서 항목 없음") }
        harness.select(document)
        harness.key(functionKey(NSRightArrowFunctionKey), keyCode: 124, to: outline)
        try expect(harness.spin { outline.isItemExpanded(document) && outline.numberOfRows == 10 },
                   "typical: → 펼치기 실패 (행 \(outline.numberOfRows))")
        try expect(outline.level(forRow: 1) == 1 && outline.level(forItem: document) == 0, "typical: 계층 수준 불일치")
        try expect(harness.waitForVisibleChangeRows(), "typical: 객체 행 페이지가 오지 않음")

        // 필수 동반 변경: 새 객체의 속성만 Stage하면 AddObject 포함 확인을 받습니다(XIV.5.3).
        guard let dependent = harness.dependentProperty(in: document), let dependentObject = dependent.parent else {
            throw Failure(message: "typical: 의존 변경 fixture 행을 찾지 못함")
        }
        try expect(outline.level(forItem: dependent) == 2, "typical: 변경 행 수준이 2가 아님")
        let stagedBefore = panel.status?.stagedChangeCount ?? 0
        guard let cell = harness.changeCell(dependent) else { throw Failure(message: "typical: 셀 없음") }
        cell.stageCheckbox.performClick(nil)
        try expect(harness.spin { panel.pendingPrompt != nil }, "typical: 필수 동반 변경 확인이 오지 않음")
        try expect(isShown(panel.dependencyBanner) && !panel.changeList.isInteractive,
                   "typical: 확인 대기 중 배너·목록 잠금 불일치")
        panel.includeRequiredButton.performClick(nil)
        try expect(harness.spin { (panel.status?.stagedChangeCount ?? 0) == stagedBefore + 2 },
                   "typical: 동반 변경 포함 Stage 결과 불일치")
        // 구조는 그대로이므로 펼침이 유지됩니다.
        try expect(outline.isItemExpanded(document) && outline.isItemExpanded(dependentObject),
                   "typical: Stage 후 펼침이 풀림")

        // 상세 요약: 변경 행 선택 → HEAD/Index/Working 값.
        _ = harness.waitForVisibleChangeRows()
        harness.select(dependent)
        try expect(harness.spin { panel.currentDetail?.node == dependent.nodeID }, "typical: 변경 상세가 오지 않음")
        try expect(panel.detailRows[.kind].map(isShown) == true && panel.detailRows[.object].map(isShown) == true,
                   "typical: 상세 행이 보이지 않음")

        // 키보드 Space: 선택 행 Stage 토글.
        let beforeSpace = panel.status?.stagedChangeCount ?? 0
        harness.key(" ", keyCode: 49, to: outline)
        try expect(harness.spin { (panel.status?.stagedChangeCount ?? 0) == beforeSpace - 1 },
                   "typical: Space Unstage 결과 불일치")
        _ = harness.waitForVisibleChangeRows()
        harness.key(" ", keyCode: 49, to: outline)
        try expect(harness.spin { (panel.status?.stagedChangeCount ?? 0) == beforeSpace },
                   "typical: Space Stage 결과 불일치")

        // 거부: 세대가 지난 Stage 요청은 실행되지 않고, 미리 보여 준 체크박스가 코어 값으로 돌아갑니다.
        _ = harness.waitForVisibleChangeRows()
        guard let shownCell = harness.changeCell(dependent), let shownRow = list.row(of: dependent) else {
            throw Failure(message: "typical: 거부 검사 셀 없음")
        }
        let truth: NSControl.StateValue = shownRow.stageState == .staged ? .on : (shownRow.stageState == .unstaged ? .off : .mixed)
        shownCell.stageCheckbox.state = truth == .on ? .off : .on
        let rejectionsBefore = panel.diagnostics.rejections
        let stagedBeforeStale = panel.status?.stagedChangeCount ?? 0
        harness.dataSource.send(.setStaged(
            generation: list.generation &- 1,
            targets: [GitChangeRange(parent: nil, children: IndexSet(integer: 0))],
            staged: true))
        // 이미 받은 페이지를 지난 세대로 다시 요청하면 거부되고, 요청 중 표시가 남지 않습니다.
        harness.dataSource.send(.loadChangePage(generation: list.generation &- 1, parent: nil, children: 0..<6))
        try expect(harness.spin { panel.diagnostics.rejections == rejectionsBefore + 2 }, "typical: 거부 이벤트가 오지 않음")
        try expect(shownCell.stageCheckbox.state == truth, "typical: 거부 뒤 체크박스가 코어 값으로 돌아가지 않음")
        try expect(panel.status?.stagedChangeCount == stagedBeforeStale, "typical: 거부된 Stage가 실행됨")

        // 키보드 ←: 선택한 변경 → 부모로 이동, 다시 ←: 객체 접기(기본 outline 동작).
        harness.select(dependent)
        harness.key(functionKey(NSLeftArrowFunctionKey), keyCode: 123, to: outline)
        harness.key(functionKey(NSLeftArrowFunctionKey), keyCode: 123, to: outline)
        try expect(harness.spin { !outline.isItemExpanded(dependentObject) }, "typical: ← 접기 실패")
        harness.key(functionKey(NSRightArrowFunctionKey), keyCode: 124, to: outline)
        try expect(harness.spin { outline.isItemExpanded(dependentObject) }, "typical: → 다시 펼치기 실패")

        // Commit → 메시지 비움, staged 0, ahead 증가. 구조가 바뀌어 다시 읽은 뒤 펼침을 노드 ID로 복원합니다.
        guard let documentID = document.nodeID else { throw Failure(message: "typical: 문서 ID 없음") }
        let aheadBefore = panel.status?.upstream?.ahead ?? 0
        try expect(panel.commitButton.isEnabled, "typical: Commit 직전 비활성 \(panel.commitBlockers)")
        panel.commitButton.performClick(nil)
        // 보낸 뒤 결과가 올 때까지 메시지를 편집할 수 없고 다시 커밋할 수 없습니다(보낸 문장 = 지울 문장).
        try expect(panel.isCommitInFlight && !panel.commitTextView.isEditable && panel.commitBlockers.contains(.operationInProgress),
                   "typical: 커밋 중 메시지가 잠기지 않음 \(panel.commitBlockers)")
        panel.commitTextView.insertText(" (late)", replacementRange: NSRange(location: NSNotFound, length: 0))
        try expect(panel.commitTextView.string == "Update button colors", "typical: 커밋 중 메시지가 바뀜 '\(panel.commitTextView.string)'")
        try expect(harness.spin { if case .succeeded = panel.lastResult?.outcome, panel.lastResult?.kind == .commit { return true }; return false },
                   "typical: Commit 결과가 오지 않음")
        try expect(!panel.isCommitInFlight && panel.commitTextView.isEditable, "typical: 커밋 후 메시지 잠금이 풀리지 않음")
        try expect(harness.announced("Committing") && harness.announced("Committed", priority: .medium),
                   "typical: 커밋 진행·결과 알림 없음")
        try expect(panel.commitTextView.string.isEmpty, "typical: Commit 후 메시지가 남음")
        // 비운 메시지는 텍스트 시스템 Undo로 되살릴 수 있어야 합니다.
        panel.commitTextView.undoManager?.undo()
        try expect(panel.commitTextView.string == "Update button colors", "typical: Commit 후 메시지 Undo 실패 '\(panel.commitTextView.string)'")
        try expect(panel.status?.stagedChangeCount == 0 && panel.status?.upstream?.ahead == aheadBefore + 1,
                   "typical: Commit 후 상태 불일치")
        try expect(panel.commitBlockers.first == .nothingStaged, "typical: Commit 후 비활성 이유 불일치 \(panel.commitBlockers)")
        try expect(harness.spin { list.item(for: documentID).map(outline.isItemExpanded) == true },
                   "typical: 구조 변경 뒤 문서 펼침이 복원되지 않음")
        try expect(list.item(for: documentID) !== document, "typical: 구조 변경 뒤 항목을 다시 만들지 않음")
        // 복원 펼침 차례가 실제로 실행되어 점유 측정(maxTurn)에 들어갔습니다.
        try expect(list.restoredStructureChanges > 0 && list.maxRestoreSeconds > 0, "typical: 복원 펼침 차례가 측정되지 않음")

        // Push(강제 없음) → ahead 0.
        try expect(panel.pushBlockers.isEmpty && panel.pushButton.isEnabled, "typical: Push 비활성 \(panel.pushBlockers)")
        panel.pushButton.performClick(nil)
        try expect(harness.spin { panel.isBusy }, "typical: Push 진행이 시작되지 않음")
        try expect(isShown(panel.progressBanner) && isShown(panel.cancelOperationButton),
                   "typical: Push 진행 배너·취소 버튼 없음")
        try expect(harness.spin { panel.lastResult?.kind == .push }, "typical: Push 결과가 오지 않음")
        try expect(harness.announced("Pushing to origin") && harness.announced("Pushed to origin", priority: .medium),
                   "typical: Push 진행·결과 알림 없음")
        try expect(!harness.announced("Staging") && !harness.announced("Unstaging"), "typical: 짧은 Stage 작업까지 알림")
        try expect(panel.pushBlockers == [.nothingToPush], "typical: Push 후 이유 불일치 \(panel.pushBlockers)")

        // Dark 외형.
        let dark = Harness(.typical, appearance: .darkAqua)
        defer { dark.close() }
        try expect(dark.waitForVisibleChangeRows(), "typical dark: 페이지 없음")
        if let first = dark.list.item(atRow: 0) { _ = dark.expand(first) }
        try expect(dark.controller.view.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua,
                   "typical dark: Dark 외형 아님")
        try expectNoAmbiguity(dark, "typical dark")
        try expectAccessibility(dark, "typical dark")

        // 프로세스 첫 상태 반영(첫 셀·글꼴·심볼 생성)이 들어 있어 느슨한 상한(한 프레임)만 둡니다. 값은 보고에 남깁니다.
        try expect(maxTurn(panel) < 1.0 / 60.0, "typical: 한 차례 점유가 한 프레임을 넘음 \(occupancy(panel))")
        report.append("typical outline/stage/deps/detail/space/arrows/reject/commit/restore/push/announce/contrast ok, rows \(outline.numberOfRows), \(occupancy(panel))")
    }

    @MainActor
    private static func checkCommitBlockers(_ report: inout [String]) throws {
        let nothing = Harness(.nothingStaged)
        defer { nothing.close() }
        try expect(nothing.waitForVisibleChangeRows(), "nothing-staged: 페이지 없음")
        nothing.typeCommitMessage("Message")
        try expect(nothing.controller.commitBlockers.first == .nothingStaged && !nothing.controller.commitButton.isEnabled,
                   "nothing-staged: 이유 불일치 \(nothing.controller.commitBlockers)")
        try expect(nothing.controller.commitReasonLabel.stringValue == GitPanelStrings.commitBlocker(.nothingStaged),
                   "nothing-staged: 이유 문구 불일치")

        let identity = Harness(.identityMissing)
        defer { identity.close() }
        try expect(identity.waitForVisibleChangeRows(), "identity: 페이지 없음")
        identity.typeCommitMessage("Message")
        try expect(identity.controller.commitBlockers.first == .identityMissing && !identity.controller.commitButton.isEnabled,
                   "identity: 이유 불일치 \(identity.controller.commitBlockers)")
        try expect(isShown(identity.controller.identityRow), "identity: 설정 안내 버튼이 보이지 않음")
        identity.controller.identityButton.performClick(nil)
        try expect(identity.host.identityRequests == 1, "identity: host 동작이 호출되지 않음")
        // Git이 거절한 커밋(실패 결과)은 오류 배너와 함께 높은 우선순위로 알립니다.
        identity.dataSource.send(.commit(message: "Message"))
        try expect(identity.spin { identity.announced("Git needs your name", priority: .high) }, "identity: 커밋 실패 알림 없음")
        try expectNoAmbiguity(identity, "identity")
        report.append("commit blockers nothing-staged/identity ok")
    }

    @MainActor
    private static func checkRestricted(_ report: inout [String]) throws {
        let harness = Harness(.restricted)
        defer { harness.close() }
        let panel = harness.controller
        try expect(harness.spin { panel.status != nil }, "restricted: status 없음")
        harness.typeCommitMessage("Message")
        try expect(panel.commitBlockers.first == .restricted && !panel.commitButton.isEnabled,
                   "restricted: Commit 이유 불일치 \(panel.commitBlockers)")
        try expect(panel.pushBlockers.first == .restricted && !panel.pushButton.isEnabled,
                   "restricted: Push 이유 불일치 \(panel.pushBlockers)")
        try expect(!panel.changesOverlay.isHidden && panel.changesOverlay.actionButton.title == GitPanelStrings.trustProject,
                   "restricted: 신뢰 부여 동작이 보이지 않음")
        panel.changesOverlay.actionButton.performClick(nil)
        try expect(harness.host.trustRequests == 1, "restricted: 신뢰 부여 host 동작이 호출되지 않음")
        try expectNoAmbiguity(harness, "restricted")
        report.append("restricted ok")
    }

    @MainActor
    private static func checkClean(_ report: inout [String]) throws {
        let harness = Harness(.clean)
        defer { harness.close() }
        let panel = harness.controller
        try expect(harness.spin { panel.status != nil }, "clean: status 없음")
        try expect(!panel.changesOverlay.isHidden && panel.changeList.scrollView.isHidden
                   && panel.changesOverlay.titleField.stringValue == GitPanelStrings.noChangesTitle
                   && !panel.changesOverlay.actionButton.isHidden,
                   "clean: 빈 상태와 다음 동작이 보이지 않음")
        try expectNoAmbiguity(harness, "clean")
        report.append("empty state ok")
    }

    @MainActor
    private static func checkConflicts(_ report: inout [String]) throws {
        let harness = Harness(.conflicts)
        defer { harness.close() }
        let panel = harness.controller
        try expect(harness.spin { panel.status != nil }, "conflicts: status 없음")
        try expect(panel.commitBlockers.contains(.unresolvedConflicts(count: 6)), "conflicts: Commit이 conflict로 막히지 않음 \(panel.commitBlockers)")

        panel.modeControl.selectedSegment = GitPanelViewController.Mode.conflicts.rawValue
        if let action = panel.modeControl.action { NSApp.sendAction(action, to: panel.modeControl.target, from: panel.modeControl) }
        try expect(panel.mode == .conflicts && isShown(panel.conflictsColumn) && !isShown(panel.changesColumn),
                   "conflicts: 화면 전환 실패")
        let list = panel.conflictList
        try expect(harness.spin { list.rowCount == 6 && (0..<6).allSatisfy { list.row(at: $0) != nil } },
                   "conflicts: conflict 페이지가 오지 않음")
        try expect(panel.applyBlockers == [.unresolved(count: 5)] && !panel.applyButton.isEnabled,
                   "conflicts: 적용 이유 불일치 \(panel.applyBlockers)")
        try expectNoAmbiguity(harness, "conflicts")
        try expectAccessibility(harness, "conflicts")

        // 종류별 허용 선택: 삭제 대 수정은 Base를 고를 수 없습니다(XIV.12.1).
        guard let card = conflictCell(harness, 1) else { throw Failure(message: "conflicts: 셀 없음") }
        try expect(!card.choiceControl.isEnabled(forSegment: 0) && card.choiceControl.isEnabled(forSegment: 1),
                   "conflicts: 허용 선택 제한 불일치")
        guard let raw = conflictCell(harness, 5) else { throw Failure(message: "conflicts: raw 셀 없음") }
        try expect(raw.choiceControl.isHidden && !raw.rawHintField.isHidden, "conflicts: raw conflict 표시 불일치")
        // 선택 강조 행에서는 주황 경고 아이콘도 시스템 대비 색으로 그립니다.
        card.backgroundStyle = .emphasized
        try expect(card.kindImageView.contentTintColor == nil, "conflicts: 강조 행 아이콘 색이 고정됨")
        card.backgroundStyle = .normal
        try expect(card.kindImageView.contentTintColor == .systemOrange, "conflicts: 강조 해제 뒤 아이콘 색이 돌아가지 않음")

        // 거부: 세대가 지난 해결 선택은 기록되지 않고, 미리 바꾼 segment가 코어 값으로 돌아갑니다.
        guard let first = conflictCell(harness, 0) else { throw Failure(message: "conflicts: 첫 셀 없음") }
        first.choiceControl.selectedSegment = 2
        let rejectionsBefore = panel.diagnostics.rejections
        harness.dataSource.send(.chooseResolution(generation: list.generation &- 1, rows: IndexSet(integer: 0), choice: .theirs))
        try expect(harness.spin { panel.diagnostics.rejections == rejectionsBefore + 1 }, "conflicts: 거부 이벤트가 오지 않음")
        try expect(first.choiceControl.selectedSegment == -1 && list.row(at: 0)?.choice == nil,
                   "conflicts: 거부 뒤 선택이 되돌아가지 않음")

        // 키보드 O: 첫 conflict에 Ours.
        list.tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        harness.window.makeFirstResponder(list.tableView)
        harness.key("o", keyCode: 31, to: list.tableView)
        try expect(harness.spin { panel.status?.unresolvedConflictCount == 4 && list.row(at: 0)?.choice == .ours },
                   "conflicts: O 키 선택 실패")
        try expect(harness.spin { panel.conflictRows[.choice].map(isShown) == true }, "conflicts: 상세가 보이지 않음")

        // 나머지는 행의 segmented control로 고릅니다.
        for index in 1..<5 {
            harness.spin { list.row(at: index) != nil }
            guard let value = list.row(at: index), let cell = conflictCell(harness, index),
                  let choice = GitConflictChoice.allCases.first(where: value.allowedChoices.contains),
                  let segment = GitConflictChoice.allCases.firstIndex(of: choice)
            else { throw Failure(message: "conflicts: \(index) 행 준비 실패") }
            cell.choiceControl.selectedSegment = segment
            if let action = cell.choiceControl.action {
                NSApp.sendAction(action, to: cell.choiceControl.target, from: cell.choiceControl)
            }
            let expected = 4 - index
            try expect(harness.spin { panel.status?.unresolvedConflictCount == expected },
                       "conflicts: \(index) 행 선택 반영 실패")
        }
        try expect(panel.applyBlockers.isEmpty && panel.applyButton.isEnabled, "conflicts: 적용 비활성 \(panel.applyBlockers)")
        panel.applyButton.performClick(nil)
        try expect(harness.spin { panel.status?.semanticConflictCount == 0 && panel.lastResult?.kind == .applyResolutions },
                   "conflicts: 해결 적용 결과가 오지 않음")
        try expect(panel.status?.rawConflictCount == 1 && isShown(panel.continueHintLabel),
                   "conflicts: raw conflict·continue 안내 불일치")
        try expect(panel.commitBlockers.contains(.unresolvedConflicts(count: 1)), "conflicts: raw conflict가 Commit을 막지 않음")
        report.append("conflicts choose/keys/reject/apply ok")
    }

    @MainActor
    private static func conflictCell(_ harness: Harness, _ row: Int) -> GitConflictCellView? {
        let table = harness.controller.conflictList.tableView
        table.scrollRowToVisible(row)
        harness.layout()
        return table.view(atColumn: 0, row: row, makeIfNecessary: true) as? GitConflictCellView
    }

    @MainActor
    private static func checkMillion(_ report: inout [String]) throws {
        let harness = Harness(.million)
        defer { harness.close() }
        let panel = harness.controller
        let list = harness.list
        let outline = harness.outline
        try expect(harness.spin(timeout: 120) { panel.status != nil }, "million: status 없음")
        let total = (panel.status?.stagedChangeCount ?? 0) + (panel.status?.unstagedChangeCount ?? 0)
        try expect(total == 1_000_000 && panel.status?.changes.documentCount == 1_000 && outline.numberOfRows == 1_000,
                   "million: 규모 불일치 changes \(total), rows \(outline.numberOfRows)")
        try expect(harness.waitForVisibleChangeRows(timeout: 30), "million: 첫 페이지 없음")

        // 끝 문서와 가운데 문서를 펼치고 그 안의 객체를 펼칩니다. 펼친 묶음마다 보이는 페이지만 요청합니다.
        for documentIndex in [999, 500] {
            guard let document = harness.child(documentIndex, of: nil) else { throw Failure(message: "million: 문서 \(documentIndex) 없음") }
            outline.scrollRowToVisible(outline.row(forItem: document))
            harness.layout()
            try expect(harness.waitForVisibleChangeRows(timeout: 30), "million: 문서 \(documentIndex) 페이지 없음")
            try expect(harness.expand(document, timeout: 30), "million: 문서 \(documentIndex) 펼치기 실패")
            guard let object = harness.child(99, of: document), harness.loadedRow(object) != nil,
                  harness.expand(object, timeout: 30), let change = harness.child(9, of: object),
                  harness.loadedRow(change)?.level == .change
            else { throw Failure(message: "million: 문서 \(documentIndex) 객체·변경 행 없음") }
            try expect(outline.level(forItem: change) == 2, "million: 변경 행 수준 불일치")
        }
        try expect(outline.numberOfRows == 1_000 + 2 * 100 + 2 * 10, "million: 펼친 뒤 행 수 불일치 \(outline.numberOfRows)")
        for target in [outline.numberOfRows - 1, 0] {
            outline.scrollRowToVisible(target)
            harness.layout()
            try expect(harness.waitForVisibleChangeRows(timeout: 30), "million: \(target) 행 페이지 없음")
        }

        var rowViews = 0
        outline.enumerateAvailableRowViews { _, _ in rowViews += 1 }
        let requested = list.cache.requestCount
        let cached = list.cache.cachedPageCount
        try expect(requested <= 16, "million: 페이지 요청이 너무 많음 \(requested)")
        try expect(cached <= GitPanelMetrics.pageCapacity, "million: 캐시 페이지 상한 초과 \(cached)")
        try expect(rowViews <= 200, "million: 행 뷰가 너무 많음 \(rowViews)")
        try expectNoAmbiguity(harness, "million")

        // 전체 선택 후 Stage: 100만 change를 최상위 범위 하나로 보냅니다.
        outline.selectAll(nil)
        list.toggleSelection()
        guard case .setStaged(_, let targets, _)? = harness.dataSource.sentCommands.last(where: {
            if case .setStaged = $0 { return true }
            return false
        }) else { throw Failure(message: "million: Stage 명령 없음") }
        try expect(targets == [GitChangeRange(parent: nil, children: IndexSet(integersIn: 0..<1_000))],
                   "million: 전체 선택 대상이 최상위 범위 하나가 아님 \(targets.count)")
        try expect(harness.spin(timeout: 120) { panel.status?.stagedChangeCount == 1_000_000 },
                   "million: 전체 Stage 결과 불일치 \(panel.status?.stagedChangeCount ?? -1)")

        // 패널이 나눈 차례(드레인·UI 반영·상세·outline 다시 읽기·복원)가 각각 Debug 상한(계약 2ms의 두 배) 안이어야
        // 합니다. 값은 보고에 남깁니다.
        try expect(maxTurn(panel) < debugTurnLimit, "million: 한 차례 점유가 상한을 넘음 \(occupancy(panel))")
        // 사용자 펼침(검사가 expandItem을 직접 부름)은 차례 점유가 아니라 인터랙션입니다. 화면에 새로 보이는 행의 셀 생성과
        // AppKit 펼침 작업이 들어 있어 §6.3 인터랙션 10ms를 넘는 알려진 위험(명세 8장·11장)이므로 값만 보고합니다.
        let expandMs = String(format: "%.2f", harness.maxUserExpandSeconds * 1000)
        report.append("million changes 1000000 (documents 1000), pages requested \(requested), cached \(cached), row views \(rowViews), \(occupancy(panel)), user expand max \(expandMs)ms")
    }

    /// 한 부모 아래 자식이 페이지 여러 개(2,000 객체)인 묶음: 그 부모의 보이는 페이지만 요청합니다.
    @MainActor
    private static func checkWide(_ report: inout [String]) throws {
        let harness = Harness(.wide)
        defer { harness.close() }
        let list = harness.list
        let outline = harness.outline
        try expect(harness.waitForVisibleChangeRows(), "wide: 첫 페이지 없음")
        guard let document = harness.child(0, of: nil), harness.expand(document) else {
            throw Failure(message: "wide: 문서 펼치기 실패")
        }
        try expect(outline.numberOfRows == 2_001, "wide: 펼친 뒤 행 수 불일치 \(outline.numberOfRows)")
        let before = list.cache.requestCount
        guard let last = harness.child(1_999, of: document), let lastRow = harness.loadedRow(last) else {
            throw Failure(message: "wide: 마지막 객체 행 없음")
        }
        try expect(lastRow.level == .object, "wide: 마지막 행 종류 불일치")
        let requested = list.cache.requestCount - before
        try expect(requested <= 2, "wide: 끝으로 이동할 때 페이지를 너무 많이 요청함 \(requested)")
        try expect(harness.dataSource.sentCommands.contains {
            if case .loadChangePage(_, let parent, let children) = $0 { return parent == document.nodeID && children.lowerBound == 1_792 }
            return false
        }, "wide: 마지막 페이지를 부모 기준으로 요청하지 않음")
        // 사용자 펼침은 화면에 새로 보이는 행의 셀 생성과 AppKit 펼침 작업으로 §6.3 인터랙션 10ms를 넘는 알려진 위험
        // (명세 8장·11장)이라 상한 없이 값만 보고합니다.
        report.append("wide parent pages ok (2000 children, \(list.cache.requestCount) requests, user expand \(String(format: "%.2f", harness.maxUserExpandSeconds * 1000))ms)")
    }

    /// 펼침 기억: 구조가 바뀐 뒤 남은 노드는 다시 펼치고(같은 페이지의 두 문서도 복원 차례 하나에 펼침 하나씩), 커밋으로
    /// 빠진 노드의 ID는 이후 구조 변경에서 버립니다.
    @MainActor
    private static func checkExpansionMemory(_ report: inout [String]) throws {
        let harness = Harness(.typical)
        defer { harness.close() }
        let panel = harness.controller
        let list = harness.list
        let outline = harness.outline
        try expect(harness.waitForVisibleChangeRows(), "expansion: 페이지 없음")
        guard let first = harness.child(1, of: nil), let second = harness.child(2, of: nil), let removed = harness.child(5, of: nil),
              let firstID = first.nodeID, let secondID = second.nodeID, let removedID = removed.nodeID,
              harness.expand(first), harness.expand(second), harness.expand(removed)
        else { throw Failure(message: "expansion: 문서 펼치기 실패") }
        let keptIDs: Set<GitChangeNodeID> = [firstID, secondID]
        try expect(list.expandedIDs == keptIDs.union([removedID]), "expansion: 펼침 기억 불일치 \(list.expandedIDs.count)")

        /// 최상위 문서 하나를 전부 Stage하고 커밋합니다. 그 문서는 목록에서 빠지고 구조 번호가 올라갑니다.
        func commitDocument(_ index: Int) throws {
            guard let status = panel.status else { throw Failure(message: "expansion: status 없음") }
            harness.dataSource.send(.setStaged(
                generation: list.generation,
                targets: [GitChangeRange(parent: nil, children: IndexSet(integer: index))],
                staged: true))
            try expect(harness.spin { panel.status?.changes.generation != status.changes.generation && !panel.isBusy },
                       "expansion: 문서 \(index) Stage 결과가 오지 않음")
            harness.dataSource.send(.commit(message: "Commit document \(index)"))
            // 상태의 구조 번호는 미룬 outline 다시 읽기보다 한 차례 먼저 오므로, 목록이 새 구조를 반영하고 복원을 마친
            // 뒤의 항목으로 확인합니다(그 전에는 이전 구조 항목이 펼쳐진 채 남아 있음).
            try expect(harness.spin {
                guard let current = panel.status?.changes.structure, current > status.changes.structure else { return false }
                return !panel.isBusy && list.cache.structure == current && !list.isRestoring
                    && keptIDs.allSatisfy { list.item(for: $0).map(outline.isItemExpanded) == true }
            }, "expansion: 문서 \(index) 커밋 뒤 남은 문서 펼침이 복원되지 않음")
        }

        // 1) 펼친 문서 하나를 커밋으로 뺍니다. 2·3) 펼치지 않은 다른 문서를 커밋해 구조를 두 번 더 바꿉니다.
        try commitDocument(5)
        try expect(list.item(for: removedID) == nil, "expansion: 커밋한 문서가 목록에 남음")
        // 최상위 페이지 하나로 온 두 문서도 복원 차례 하나에 펼침 하나씩만 합니다(명세 8장).
        try expect(list.maxRestoreChangesPerTurn == 1 && list.restoredStructureChanges >= 2,
                   "expansion: 복원 차례 하나에서 outline 구조를 \(list.maxRestoreChangesPerTurn)번 바꿈")
        try commitDocument(3)
        try commitDocument(3)
        try expect(!list.expandedIDs.contains(removedID), "expansion: 빠진 노드의 펼침 ID가 남음 \(list.expandedIDs.count)")
        try expect(list.expandedIDs == keptIDs, "expansion: 남은 노드의 펼침 ID 불일치 \(list.expandedIDs.count)")
        report.append("expansion memory restore/prune ok (\(list.expandedIDs.count) ids after 3 commits, restores \(list.restoredStructureChanges), max per turn \(list.maxRestoreChangesPerTurn))")
    }

    /// 이벤트 처리가 `inlineUIThreshold`를 넘으면 UI 반영을 다음 런 루프 차례로 넘깁니다. Stub이 이벤트마다 메인 스레드를
    /// 점유하게 해(느린 디코딩 모사) 이 경로를 실제로 실행합니다.
    @MainActor
    private static func checkDeferredUI(_ report: inout [String]) throws {
        let harness = Harness(.typical)
        defer { harness.close() }
        let panel = harness.controller
        let dataSource = harness.dataSource
        try expect(harness.waitForVisibleChangeRows(), "deferred UI: 페이지 없음")
        guard let before = panel.status else { throw Failure(message: "deferred UI: status 없음") }
        let summaryBefore = panel.summaryLabel.stringValue

        // 패널이 스스로 깨어나지 않게 hook을 잠시 바꾸고, Stage 이벤트(진행·상태·결과)가 큐에 모두 쌓인 뒤 드레인 한
        // 차례를 직접 실행합니다.
        let wake = dataSource.onEventsAvailable
        var woke = false
        dataSource.onEventsAvailable = { woke = true }
        dataSource.send(.setStaged(
            generation: harness.list.generation,
            targets: [GitChangeRange(parent: nil, children: IndexSet(integer: 0))],
            staged: true))
        let queued = harness.spin { woke && dataSource.queuedEventCount >= 3 }
        dataSource.onEventsAvailable = wake
        try expect(queued, "deferred UI: Stage 이벤트가 큐에 오지 않음")

        let deferredBefore = panel.diagnostics.deferredUIUpdates
        dataSource.pollDelay = GitPanelMetrics.inlineUIThreshold * 1.5
        panel.drainEvents()
        dataSource.pollDelay = 0
        guard let after = panel.status, after != before else { throw Failure(message: "deferred UI: 드레인에서 상태를 받지 못함") }
        try expect(panel.diagnostics.deferredUIUpdates == deferredBefore + 1 && panel.hasPendingWork,
                   "deferred UI: 느린 드레인에서 UI 반영을 다음 차례로 넘기지 않음")
        try expect(panel.summaryLabel.stringValue == summaryBefore, "deferred UI: 드레인 차례에서 UI를 반영함")
        let expected = GitPanelStrings.statusSummary(staged: after.stagedChangeCount, unstaged: after.unstagedChangeCount)
        try expect(harness.spin { panel.summaryLabel.stringValue == expected && !panel.hasPendingWork },
                   "deferred UI: 다음 차례에 UI가 반영되지 않음 '\(panel.summaryLabel.stringValue)'")
        report.append("deferred UI path ok (deferred \(panel.diagnostics.deferredUIUpdates - deferredBefore))")
    }

    /// 사용자가 시작한 작업 명령을 코어가 거부하면 오류 배너와 높은 우선순위 알림으로 알립니다.
    @MainActor
    private static func checkRejectionFeedback(_ report: inout [String]) throws {
        let harness = Harness(.typical)
        defer { harness.close() }
        let panel = harness.controller
        try expect(harness.waitForVisibleChangeRows(), "rejection: 페이지 없음")
        harness.typeCommitMessage("Racing commit")
        try expect(panel.commitButton.isEnabled, "rejection: Commit 비활성 \(panel.commitBlockers)")

        // 1) 다른 곳에서 시작한 Push가 코어에 먼저 들어갔고, 패널은 그 진행 이벤트를 아직 받지 못한 채 Commit을 보냅니다.
        //    코어는 busy로 거부하고, 패널은 메시지 잠금을 풀고 실행하지 않았다는 문장과 이유를 배너·알림으로 전합니다.
        harness.dataSource.send(.push(remote: "origin"))
        panel.commitButton.performClick(nil)
        try expect(panel.isCommitInFlight, "rejection: Commit을 보내지 않음")
        try expect(harness.spin { !panel.isCommitInFlight }, "rejection: 거부된 Commit 뒤 메시지 잠금이 풀리지 않음")
        try expect(panel.commitTextView.isEditable && panel.commitTextView.string == "Racing commit",
                   "rejection: 거부 뒤 메시지가 바뀜 '\(panel.commitTextView.string)'")
        guard let commitNotice = GitPanelStrings.rejectionNotice(command: .commit(message: "Racing commit"), reason: .busy) else {
            throw Failure(message: "rejection: Commit 거부 문구 없음")
        }
        try expect(panel.rejectionNotice == commitNotice && isShown(panel.resultBanner)
                   && panel.resultBanner.messageField.stringValue == commitNotice,
                   "rejection: 거부된 Commit을 배너로 알리지 않음 '\(panel.resultBanner.messageField.stringValue)'")
        try expect(harness.announced(commitNotice, priority: .high), "rejection: 거부된 Commit을 VoiceOver에 알리지 않음")
        try expectNoAmbiguity(harness, "rejection")
        // 새 작업 결과(Push 성공)가 오면 그 결과 배너로 바뀝니다.
        try expect(harness.spin { panel.lastResult?.kind == .push && !panel.isBusy }, "rejection: Push 결과가 오지 않음")
        try expect(panel.rejectionNotice == nil && panel.resultBanner.messageField.stringValue != commitNotice,
                   "rejection: 새 결과 뒤에도 거부 알림이 남음")

        // 2) 필수 동반 변경 확인에 다른 응답이 먼저 처리되어 코어가 요청을 지운 뒤 패널이 답하면, 거부를 알리고 목록을
        //    다시 조작할 수 있습니다. 취소 응답의 거부는 알리지 않습니다.
        guard let document = harness.child(0, of: nil), harness.expand(document),
              let property = harness.dependentProperty(in: document), let cell = harness.changeCell(property)
        else { throw Failure(message: "rejection: 의존 변경 fixture 행을 찾지 못함") }
        cell.stageCheckbox.performClick(nil)
        try expect(harness.spin { panel.pendingPrompt != nil }, "rejection: 필수 동반 변경 확인이 오지 않음")
        guard let prompt = panel.pendingPrompt else { throw Failure(message: "rejection: 확인 없음") }
        harness.dataSource.send(.answerDependencyPrompt(requestID: prompt.requestID, includeRequired: true))
        panel.includeRequiredButton.performClick(nil)
        let answer = GitPanelCommand.answerDependencyPrompt(requestID: prompt.requestID, includeRequired: true)
        guard let answerNotice = GitPanelStrings.rejectionNotice(command: answer, reason: .unknownTarget) else {
            throw Failure(message: "rejection: 확인 응답 거부 문구 없음")
        }
        try expect(harness.spin { panel.rejectionNotice == answerNotice && !panel.isBusy },
                   "rejection: 거부된 확인 응답을 알리지 않음 '\(panel.rejectionNotice ?? "")'")
        try expect(isShown(panel.resultBanner) && harness.announced(answerNotice, priority: .high) && panel.changeList.isInteractive,
                   "rejection: 확인 응답 거부 배너·알림·목록 잠금 해제 불일치")
        try expect(GitPanelStrings.rejectionNotice(
            command: .answerDependencyPrompt(requestID: prompt.requestID, includeRequired: false), reason: .unknownTarget) == nil,
                   "rejection: 취소 응답의 거부까지 알림")
        // Dismiss는 거부 알림도 닫습니다.
        panel.dismissResultButton.performClick(nil)
        try expect(panel.rejectionNotice == nil && !isShown(panel.resultBanner), "rejection: Dismiss 뒤 거부 배너가 남음")
        report.append("rejection feedback commit-busy/answer-unknown/dismiss ok")
    }

    // MARK: 키보드 순서·단축키 범위

    /// 셸 창처럼 패널 위·아래에 다른 컨트롤이 있는 창.
    @MainActor
    private final class HostedPanel {
        let dataSource: GitPanelStubDataSource
        let host = HostRecorder()
        let controller: GitPanelViewController
        let window: NSWindow
        let above = NSTextField(string: "")
        let below = NSTextField(string: "")

        init(_ scenario: GitPanelStubScenario) {
            dataSource = GitPanelStubDataSource(scenario: scenario)
            controller = GitPanelViewController(dataSource: dataSource, host: host)
            let size = NSSize(width: 320, height: 860)
            window = NSWindow(
                contentRect: NSRect(origin: .zero, size: size),
                styleMask: [.titled, .resizable],
                backing: .buffered,
                defer: false
            )
            window.isReleasedWhenClosed = false
            // 셸 창(ShellWindowController)과 같이 시스템이 위치 순서로 key view loop를 계산하게 합니다.
            window.autorecalculatesKeyViewLoop = true
            let container = NSView()
            window.contentView = container
            let panel = controller.view
            for view in [above, panel, below] {
                view.translatesAutoresizingMaskIntoConstraints = false
                container.addSubview(view)
            }
            NSLayoutConstraint.activate([
                above.topAnchor.constraint(equalTo: container.topAnchor, constant: 8),
                above.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 8),
                above.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -8),
                panel.topAnchor.constraint(equalTo: above.bottomAnchor, constant: 8),
                panel.leadingAnchor.constraint(equalTo: container.leadingAnchor),
                panel.trailingAnchor.constraint(equalTo: container.trailingAnchor),
                below.topAnchor.constraint(equalTo: panel.bottomAnchor, constant: 8),
                below.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 8),
                below.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -8),
                below.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -8),
            ])
            // 시스템 key view loop는 창이 화면 목록에 있어야 계산됩니다. 화면 밖 좌표에 두어 보이지 않게 합니다.
            window.setFrameOrigin(NSPoint(x: -20_000, y: -20_000))
            window.orderFrontRegardless()
            container.layoutSubtreeIfNeeded()
        }

        @discardableResult
        func spin(timeout: TimeInterval = 10, until condition: () -> Bool) -> Bool {
            let deadline = Date().addingTimeInterval(timeout)
            while !condition() {
                if Date() > deadline { return false }
                RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.005))
                window.contentView?.layoutSubtreeIfNeeded()
            }
            while controller.hasPendingWork, condition(), Date() <= deadline {
                RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.005))
                window.contentView?.layoutSubtreeIfNeeded()
            }
            window.contentView?.layoutSubtreeIfNeeded()
            return true
        }

        /// 위 컨트롤에서 Tab 순서(nextKeyView 고리)를 따라 아래 컨트롤까지 갑니다. 보이는 패널 뷰를 순서대로 돌려주고,
        /// 아래 컨트롤에 닿지 못하면(패널 안에서 맴돌면) nil입니다.
        ///
        /// 실제 Tab 정지 여부는 시스템 "키보드 탐색" 설정(Full Keyboard Access)에 따라 버튼을 건너뛰므로,
        /// 설정과 무관한 고리 소속을 확인합니다.
        func walkToBelow() -> [NSView]? {
            // 실제 Tab처럼 창에 다음 key view로 옮겨 달라고 해, 필요하면(뷰가 추가·제거됨) 시스템이 스스로 고리를 다시
            // 계산하게 합니다. 강제로 다시 계산하지 않으므로 패널이 직접 이은 고리(nextKeyView)가 있으면 그대로 드러납니다.
            window.makeFirstResponder(above)
            window.selectKeyView(following: above)
            var visited: [NSView] = []
            var seen = Set<ObjectIdentifier>()
            var current: NSView? = above.nextKeyView
            while let view = current, seen.insert(ObjectIdentifier(view)).inserted {
                if view === below { return visited }
                if view.isDescendant(of: controller.view), !view.isHiddenOrHasHiddenAncestor { visited.append(view) }
                current = view.nextKeyView
            }
            return nil
        }

        func commandKey(_ characters: String, keyCode: UInt16) -> NSEvent? {
            NSEvent.keyEvent(
                with: .keyDown, location: .zero, modifierFlags: [.command],
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber, context: nil,
                characters: characters, charactersIgnoringModifiers: characters, isARepeat: false, keyCode: keyCode)
        }

        func close() {
            window.close()
        }
    }

    /// 순서대로 들어 있는지(사이에 다른 뷰가 있어도 됨).
    @MainActor
    private static func expectOrder(_ visited: [NSView], _ expected: [(String, NSView)], _ context: String) throws {
        var index = 0
        for (name, view) in expected {
            guard let found = visited[index...].firstIndex(where: { $0 === view }) else {
                let present = visited.contains { $0 === view }
                throw Failure(message: "\(context): Tab 순서에 \(name) \(present ? "순서 어긋남" : "없음")")
            }
            index = found + 1
        }
    }

    @MainActor
    private static func checkKeyboard(_ report: inout [String]) throws {
        // 0) 패널은 key view를 직접 잇지 않습니다(닫힌 고리 금지). 창에 넣기 전, 두 화면을 오가도 연결이 없어야 합니다.
        let bare = GitPanelViewController(dataSource: GitPanelStubDataSource(scenario: .typical))
        bare.loadViewIfNeeded()
        bare.setMode(.conflicts)
        bare.setMode(.changes)
        let controls: [NSView] = [
            bare.refreshButton, bare.modeControl, bare.saveNowButton, bare.useSavedButton, bare.retryButton,
            bare.includeRequiredButton, bare.declineRequiredButton, bare.cancelOperationButton, bare.dismissResultButton,
            bare.changesOverlay.actionButton, bare.changeList.outlineView, bare.commitTextView, bare.commitButton,
            bare.identityButton, bare.pushButton, bare.remotePopup, bare.conflictsOverlay.actionButton,
            bare.conflictList.tableView, bare.revealButton, bare.applyButton,
        ]
        let linked = controls.filter { $0.nextKeyView != nil }
        try expect(linked.isEmpty, "keyboard: 패널이 key view를 직접 이음 \(linked.map { String(describing: type(of: $0)) })")

        // 1) 변경 화면 + 저장 대기 배너: 배너 버튼까지 위치 순서로 닿고, 패널 밖(아래 컨트롤)으로 나갑니다.
        let typical = HostedPanel(.typical)
        defer { typical.close() }
        let panel = typical.controller
        // 이 fixture는 배너 애니메이션을 검사합니다. 시스템 접근성 설정과 독립적으로 재생 조건을 만듭니다.
        var displayOptions = panel.displayOptions
        displayOptions.reduceMotion = false
        panel.apply(displayOptions: displayOptions)
        try expect(typical.spin { panel.status != nil && isShown(panel.autosaveBanner) }, "keyboard: 상태·배너가 오지 않음")
        guard let changesOrder = typical.walkToBelow() else { throw Failure(message: "keyboard: Tab이 패널 밖으로 나가지 못함(변경 화면)") }
        try expectOrder(changesOrder, [
            ("refresh", panel.refreshButton), ("mode", panel.modeControl), ("Save Now", panel.saveNowButton),
            ("Use Saved Version", panel.useSavedButton), ("changes list", panel.changeList.outlineView),
            ("commit message", panel.commitTextView), ("commit", panel.commitButton), ("push", panel.pushButton),
            ("push destination", panel.remotePopup),
        ], "keyboard 변경 화면")
        try expect(!changesOrder.contains { $0 === panel.conflictList.tableView }, "keyboard: 숨긴 conflict 화면이 Tab 순서에 보임")

        // 2) 진행 중 작업의 취소 버튼(Push)도 순서에 들어갑니다.
        panel.pushButton.performClick(nil)
        try expect(typical.spin { isShown(panel.cancelOperationButton) }, "keyboard: Push 취소 버튼이 보이지 않음")
        guard let progressOrder = typical.walkToBelow() else { throw Failure(message: "keyboard: 진행 중 Tab이 패널 밖으로 나가지 못함") }
        try expectOrder(progressOrder, [("mode", panel.modeControl), ("Cancel", panel.cancelOperationButton),
                                        ("changes list", panel.changeList.outlineView)], "keyboard 진행 배너")
        try expect(typical.spin { panel.lastResult?.kind == .push && isShown(panel.dismissResultButton) }, "keyboard: Push 결과 배너 없음")
        guard let resultOrder = typical.walkToBelow() else { throw Failure(message: "keyboard: 결과 배너 뒤 Tab이 나가지 못함") }
        try expectOrder(resultOrder, [("Dismiss", panel.dismissResultButton), ("changes list", panel.changeList.outlineView)],
                        "keyboard 결과 배너")

        // 3) ⌘Return: 패널 밖 포커스에서는 처리하지 않고(다른 패널로 넘김), 커밋 메시지 포커스에서는 커밋합니다.
        guard let commitKey = typical.commandKey("\r", keyCode: 36) else { throw Failure(message: "keyboard: 키 이벤트 생성 실패") }
        try expect(typical.spin { panel.status?.stagedChangeCount ?? 0 > 0 && !panel.isBusy }, "keyboard: Stage된 변경 없음")
        typical.window.makeFirstResponder(panel.commitTextView)
        panel.commitTextView.insertText("Scoped commit", replacementRange: NSRange(location: NSNotFound, length: 0))
        typical.window.makeFirstResponder(typical.above)
        let commitsBefore = typical.dataSource.sentCommands.filter { if case .commit = $0 { return true }; return false }.count
        try expect(!typical.window.performKeyEquivalent(with: commitKey), "keyboard: 패널 밖 포커스에서 ⌘Return이 처리됨")
        let commitsOutside = typical.dataSource.sentCommands.filter { if case .commit = $0 { return true }; return false }.count
        try expect(commitsOutside == commitsBefore, "keyboard: 패널 밖 포커스에서 커밋 명령이 나감")
        typical.window.makeFirstResponder(panel.commitTextView)
        try expect(typical.window.performKeyEquivalent(with: commitKey), "keyboard: 커밋 메시지 포커스에서 ⌘Return 미처리")
        try expect(typical.dataSource.sentCommands.last == .commit(message: "Scoped commit"), "keyboard: ⌘Return 커밋 명령 없음")

        // 4) conflict 화면: 목록·적용 버튼에 닿고 나갑니다. 변경 화면 컨트롤은 건너뜁니다.
        let conflicts = HostedPanel(.conflicts)
        defer { conflicts.close() }
        let conflictPanel = conflicts.controller
        try expect(conflicts.spin { conflictPanel.status != nil }, "keyboard conflicts: status 없음")
        conflictPanel.setMode(.conflicts)
        guard let conflictOrder = conflicts.walkToBelow() else { throw Failure(message: "keyboard: Tab이 패널 밖으로 나가지 못함(conflict 화면)") }
        try expectOrder(conflictOrder, [("mode", conflictPanel.modeControl), ("conflicts list", conflictPanel.conflictList.tableView),
                                        ("apply", conflictPanel.applyButton)], "keyboard conflict 화면")
        try expect(!conflictOrder.contains { $0 === conflictPanel.commitTextView }, "keyboard: 숨긴 변경 화면이 Tab 순서에 보임")
        // conflict 화면에서는 ⌘Return이 커밋하지 않습니다.
        conflicts.window.makeFirstResponder(conflictPanel.conflictList.tableView)
        guard let conflictKey = conflicts.commandKey("\r", keyCode: 36) else { throw Failure(message: "keyboard: 키 이벤트 생성 실패") }
        try expect(!conflicts.window.performKeyEquivalent(with: conflictKey), "keyboard: conflict 화면에서 ⌘Return이 처리됨")

        // 5) 제한 모드: 유일한 동작인 "Trust Project…"에 키보드로 닿습니다.
        let restricted = HostedPanel(.restricted)
        defer { restricted.close() }
        let restrictedPanel = restricted.controller
        try expect(restricted.spin { restrictedPanel.status != nil && !restrictedPanel.changesOverlay.isHidden },
                   "keyboard restricted: 상태 없음")
        guard let restrictedOrder = restricted.walkToBelow() else { throw Failure(message: "keyboard: Tab이 패널 밖으로 나가지 못함(제한 모드)") }
        try expectOrder(restrictedOrder, [("mode", restrictedPanel.modeControl), ("Trust Project", restrictedPanel.changesOverlay.actionButton)],
                        "keyboard 제한 모드")

        // 화면에 보이는 창(배너 페이드 경로 실행)에서도 한 차례 점유가 Debug 상한 안이어야 합니다.
        try expect(panel.diagnostics.animatedUIUpdates > 0, "keyboard: 배너 페이드가 실행되지 않음 (visible=\(typical.window.isVisible), duration=\(panel.displayOptions.revealDuration))")
        try expect(maxTurn(panel) < debugTurnLimit, "keyboard: 보이는 창 한 차례 점유가 상한을 넘음 \(occupancy(panel))")
        report.append("keyboard loop changes \(changesOrder.count)/conflicts \(conflictOrder.count)/restricted \(restrictedOrder.count) views, exits panel; ⌘Return scoped; visible window animated UI \(panel.diagnostics.animatedUIUpdates), \(occupancy(panel))")
    }
}
