import AppKit

/// Git 패널 offscreen 검사(XIV.13.5 UI smoke의 정적 부분).
///
/// Stub data source로 패널을 화면에 표시하지 않고 만든 뒤 일반 크기에서 layout을 강제하고 다음을 확인합니다.
/// - 모든 SF Symbols 이름이 시스템에 있음
/// - 표시 중인 뷰에 모호한 Auto Layout이 없음(Light·Dark, 변경·conflict 화면)
/// - 핵심 컨트롤과 보이는 목록 행에 접근성 레이블이 있음
/// - Commit 비활성 이유(Stage 없음, identity 없음, 신뢰 전 프로젝트)와 Push·해결 적용 이유
/// - Stage 체크박스·필수 동반 변경 확인·키보드(Space, ←/→, B/O/T)·Commit·Push·Conflict 해결 흐름
/// - 100만 change에서 보이는 페이지만 요청하고 캐시·행 뷰 수가 제한됨(XIV.13.1)
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
            window.contentViewController = controller
            window.setContentSize(size)
            layout()
        }

        func layout() {
            window.contentView?.layoutSubtreeIfNeeded()
        }

        /// 메인 런 루프를 돌려 Stub 이벤트를 처리합니다.
        @discardableResult
        func spin(timeout: TimeInterval = 10, until condition: () -> Bool) -> Bool {
            let deadline = Date().addingTimeInterval(timeout)
            while !condition() {
                if Date() > deadline { return false }
                RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.005))
                layout()
            }
            layout()
            return true
        }

        /// 보이는 변경 행이 모두 fresh 페이지로 채워질 때까지 기다립니다.
        func waitForVisibleChangeRows(timeout: TimeInterval = 10) -> Bool {
            spin(timeout: timeout) {
                let list = controller.changeList
                guard list.rowCount > 0 else { return false }
                let visible = list.tableView.rows(in: list.tableView.visibleRect)
                guard visible.length > 0 else { return false }
                return (visible.location..<(visible.location + visible.length)).allSatisfy { list.row(at: $0) != nil }
            }
        }

        func changeCell(_ row: Int) -> GitChangeCellView? {
            controller.changeList.tableView.scrollRowToVisible(row)
            layout()
            return controller.changeList.tableView.view(atColumn: 0, row: row, makeIfNecessary: true) as? GitChangeCellView
        }

        func conflictCell(_ row: Int) -> GitConflictCellView? {
            controller.conflictList.tableView.scrollRowToVisible(row)
            layout()
            return controller.conflictList.tableView.view(atColumn: 0, row: row, makeIfNecessary: true) as? GitConflictCellView
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
            ("changes list", panel.changeList.tableView),
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

        var missing: [String] = []
        panel.changeList.tableView.enumerateAvailableRowViews { rowView, row in
            guard let cell = rowView.view(atColumn: 0) as? GitChangeCellView else { return }
            if !cell.stageCheckbox.isHidden, (cell.stageCheckbox.accessibilityLabel() ?? "").isEmpty {
                missing.append("change \(row) checkbox")
            }
            if !cell.disclosureButton.isHidden, (cell.disclosureButton.accessibilityLabel() ?? "").isEmpty {
                missing.append("change \(row) disclosure")
            }
            if cell.kindImageView.image != nil, (cell.kindImageView.accessibilityLabel() ?? "").isEmpty {
                missing.append("change \(row) icon")
            }
        }
        panel.conflictList.tableView.enumerateAvailableRowViews { rowView, row in
            guard let cell = rowView.view(atColumn: 0) as? GitConflictCellView else { return }
            if !cell.choiceControl.isHidden, (cell.choiceControl.accessibilityLabel() ?? "").isEmpty {
                missing.append("conflict \(row) choice")
            }
        }
        try expect(missing.isEmpty, "\(context): 접근성 레이블 누락 \(missing.prefix(5))")
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
        try expect(harness.waitForVisibleChangeRows(), "typical: 변경 목록 페이지가 오지 않음")
        guard let initial = panel.status else { throw Failure(message: "typical: status 없음") }

        // 구조: 표준 AppKit 목록·텍스트 컨트롤.
        try expect(type(of: panel.changeList.tableView).isSubclass(of: NSTableView.self), "typical: 변경 목록이 NSTableView가 아님")
        try expect(panel.commitScrollView.documentView === panel.commitTextView, "typical: 커밋 메시지가 NSTextView가 아님")
        try expect(!panel.commitTextView.isRichText && panel.commitTextView.allowsUndo, "typical: 커밋 메시지 plain text·undo 아님")
        try expect(panel.commitButton.keyEquivalent == "\r" && panel.commitButton.keyEquivalentModifierMask == [.command],
                   "typical: Commit 키 등가가 Command-Return이 아님")

        try expectNoAmbiguity(harness, "typical light")
        try expectAccessibility(harness, "typical")

        // 저장 대기 편집 안내(XIV.2.8).
        try expect(initial.pendingAutosave && isShown(panel.autosaveBanner), "typical: 저장 대기 배너가 보이지 않음")
        panel.saveNowButton.performClick(nil)
        try expect(harness.spin { panel.status?.pendingAutosave == false && !isShown(panel.autosaveBanner) },
                   "typical: Save Now 후 배너가 사라지지 않음")

        // 메시지가 비어 있으면 Commit 비활성.
        try expect(panel.commitBlockers == [.emptyMessage] && !panel.commitButton.isEnabled,
                   "typical: 빈 메시지 비활성 이유 불일치 \(panel.commitBlockers)")
        harness.typeCommitMessage("Update button colors")
        try expect(panel.commitBlockers.isEmpty && panel.commitButton.isEnabled, "typical: 메시지 입력 후 Commit 비활성")

        // 필수 동반 변경: 새 객체의 속성만 Stage하면 AddObject 포함 확인을 받습니다(XIV.5.3).
        let list = panel.changeList
        var target: Int?
        var row = 0
        while row < list.rowCount, target == nil {
            if list.row(at: row) == nil {
                list.tableView.scrollRowToVisible(row)
                _ = harness.waitForVisibleChangeRows()
            }
            if let header = list.row(at: row), header.level == .object, header.detail.hasPrefix("New"),
               let add = list.row(at: row + 1), add.kind == .addObject, add.stageState == .unstaged,
               let property = list.row(at: row + 2), property.level == .change, property.stageState == .unstaged {
                target = row + 2
            }
            row += 1
        }
        guard let dependentRow = target else { throw Failure(message: "typical: 의존 변경 fixture 행을 찾지 못함") }
        let stagedBefore = panel.status?.stagedChangeCount ?? 0
        guard let cell = harness.changeCell(dependentRow) else { throw Failure(message: "typical: 셀 없음") }
        cell.stageCheckbox.performClick(nil)
        try expect(harness.spin { panel.pendingPrompt != nil }, "typical: 필수 동반 변경 확인이 오지 않음")
        try expect(isShown(panel.dependencyBanner) && !panel.changeList.isInteractive,
                   "typical: 확인 대기 중 배너·목록 잠금 불일치")
        panel.includeRequiredButton.performClick(nil)
        try expect(harness.spin { (panel.status?.stagedChangeCount ?? 0) == stagedBefore + 2 },
                   "typical: 동반 변경 포함 Stage 결과 불일치")

        // 상세 요약: 변경 행 선택 → HEAD/Index/Working 값.
        _ = harness.waitForVisibleChangeRows()
        list.tableView.selectRowIndexes(IndexSet(integer: dependentRow), byExtendingSelection: false)
        try expect(harness.spin { panel.currentDetail?.row == dependentRow }, "typical: 변경 상세가 오지 않음")
        try expect(panel.detailRows[.kind].map(isShown) == true && panel.detailRows[.object].map(isShown) == true,
                   "typical: 상세 행이 보이지 않음")

        // 키보드 Space: 선택 행 Stage 토글.
        let beforeSpace = panel.status?.stagedChangeCount ?? 0
        harness.window.makeFirstResponder(list.tableView)
        harness.key(" ", keyCode: 49, to: list.tableView)
        try expect(harness.spin { (panel.status?.stagedChangeCount ?? 0) == beforeSpace - 1 },
                   "typical: Space Unstage 결과 불일치")
        _ = harness.waitForVisibleChangeRows()
        harness.key(" ", keyCode: 49, to: list.tableView)
        try expect(harness.spin { (panel.status?.stagedChangeCount ?? 0) == beforeSpace },
                   "typical: Space Stage 결과 불일치")

        // 키보드 ←/→: 문서 묶음 접기·펼치기.
        let rowsBefore = list.rowCount
        list.tableView.scrollRowToVisible(0)
        list.tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        _ = harness.waitForVisibleChangeRows()
        harness.key(functionKey(NSLeftArrowFunctionKey), keyCode: 123, to: list.tableView)
        try expect(harness.spin { list.rowCount < rowsBefore }, "typical: ← 접기 실패")
        _ = harness.waitForVisibleChangeRows()
        harness.key(functionKey(NSRightArrowFunctionKey), keyCode: 124, to: list.tableView)
        try expect(harness.spin { list.rowCount == rowsBefore }, "typical: → 펼치기 실패")

        // Commit → 메시지 비움, staged 0, ahead 증가.
        let aheadBefore = panel.status?.upstream?.ahead ?? 0
        try expect(panel.commitButton.isEnabled, "typical: Commit 직전 비활성 \(panel.commitBlockers)")
        panel.commitButton.performClick(nil)
        try expect(harness.spin { if case .succeeded = panel.lastResult?.outcome, panel.lastResult?.kind == .commit { return true }; return false },
                   "typical: Commit 결과가 오지 않음")
        try expect(panel.commitTextView.string.isEmpty, "typical: Commit 후 메시지가 남음")
        // 비운 메시지는 텍스트 시스템 Undo로 되살릴 수 있어야 합니다.
        panel.commitTextView.undoManager?.undo()
        try expect(panel.commitTextView.string == "Update button colors", "typical: Commit 후 메시지 Undo 실패 '\(panel.commitTextView.string)'")
        try expect(panel.status?.stagedChangeCount == 0 && panel.status?.upstream?.ahead == aheadBefore + 1,
                   "typical: Commit 후 상태 불일치")
        try expect(panel.commitBlockers.first == .nothingStaged, "typical: Commit 후 비활성 이유 불일치 \(panel.commitBlockers)")

        // Push(강제 없음) → ahead 0.
        try expect(panel.pushBlockers.isEmpty && panel.pushButton.isEnabled, "typical: Push 비활성 \(panel.pushBlockers)")
        panel.pushButton.performClick(nil)
        try expect(harness.spin { panel.isBusy }, "typical: Push 진행이 시작되지 않음")
        try expect(isShown(panel.progressBanner) && isShown(panel.cancelOperationButton),
                   "typical: Push 진행 배너·취소 버튼 없음")
        try expect(harness.spin { panel.lastResult?.kind == .push }, "typical: Push 결과가 오지 않음")
        try expect(panel.pushBlockers == [.nothingToPush], "typical: Push 후 이유 불일치 \(panel.pushBlockers)")

        // Dark 외형.
        let dark = Harness(.typical, appearance: .darkAqua)
        defer { dark.close() }
        try expect(dark.waitForVisibleChangeRows(), "typical dark: 페이지 없음")
        try expect(dark.controller.view.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua,
                   "typical dark: Dark 외형 아님")
        try expectNoAmbiguity(dark, "typical dark")

        let typicalDrain = String(format: "%.2f", panel.diagnostics.maxDrainSeconds * 1000)
        report.append("typical stage/deps/detail/space/arrows/commit/push ok, rows \(list.rowCount), max drain \(typicalDrain)ms")
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
        guard let card = harness.conflictCell(1) else { throw Failure(message: "conflicts: 셀 없음") }
        try expect(!card.choiceControl.isEnabled(forSegment: 0) && card.choiceControl.isEnabled(forSegment: 1),
                   "conflicts: 허용 선택 제한 불일치")
        guard let raw = harness.conflictCell(5) else { throw Failure(message: "conflicts: raw 셀 없음") }
        try expect(raw.choiceControl.isHidden && !raw.rawHintField.isHidden, "conflicts: raw conflict 표시 불일치")

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
            guard let value = list.row(at: index), let cell = harness.conflictCell(index),
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
        report.append("conflicts choose/keys/apply ok")
    }

    @MainActor
    private static func checkMillion(_ report: inout [String]) throws {
        let harness = Harness(.million)
        defer { harness.close() }
        let panel = harness.controller
        let list = panel.changeList
        let expectedRows = 1_000 + 100_000 + 1_000_000
        try expect(harness.spin(timeout: 120) { panel.status != nil }, "million: status 없음")
        try expect(panel.status?.changes.rowCount == expectedRows && list.tableView.numberOfRows == expectedRows,
                   "million: 행 수 불일치 \(panel.status?.changes.rowCount ?? -1)")
        try expect(harness.waitForVisibleChangeRows(timeout: 30), "million: 첫 페이지 없음")

        for target in [expectedRows - 1, expectedRows / 2, 123_457, 0] {
            list.tableView.scrollRowToVisible(target)
            harness.layout()
            try expect(harness.waitForVisibleChangeRows(timeout: 30) && list.row(at: target) != nil,
                       "million: \(target) 행 페이지 없음")
        }
        let lastCell = harness.changeCell(expectedRows - 1)
        try expect(lastCell.map { $0.titleField.stringValue != GitPanelStrings.loading } == true, "million: 마지막 행이 placeholder")

        var rowViews = 0
        list.tableView.enumerateAvailableRowViews { _, _ in rowViews += 1 }
        let requested = list.cache.requestCount
        let cached = list.cache.cachedPageCount
        try expect(requested <= 16, "million: 페이지 요청이 너무 많음 \(requested)")
        try expect(cached <= GitPanelMetrics.pageCapacity, "million: 캐시 페이지 상한 초과 \(cached)")
        try expect(rowViews <= 200, "million: 행 뷰가 너무 많음 \(rowViews)")
        try expectNoAmbiguity(harness, "million")

        // 큰 목록에서 문서 묶음 접기·펼치기(코어 평탄화, 새 세대).
        list.tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        harness.window.makeFirstResponder(list.tableView)
        harness.key(functionKey(NSLeftArrowFunctionKey), keyCode: 123, to: list.tableView)
        try expect(harness.spin(timeout: 30) { list.rowCount == expectedRows - 1_100 }, "million: 접기 결과 불일치 \(list.rowCount)")
        try expect(harness.waitForVisibleChangeRows(timeout: 30), "million: 접은 뒤 페이지 없음")
        list.tableView.scrollRowToVisible(0)
        try expect(harness.waitForVisibleChangeRows(timeout: 30) && list.row(at: 0)?.isExpanded == false,
                   "million: 접은 묶음 행 페이지 없음")
        harness.key(functionKey(NSRightArrowFunctionKey), keyCode: 124, to: list.tableView)
        try expect(harness.spin(timeout: 30) { list.rowCount == expectedRows }, "million: 펼치기 결과 불일치 \(list.rowCount)")
        try expect(harness.waitForVisibleChangeRows(timeout: 30), "million: 펼친 뒤 페이지 없음")

        // 전체 선택 후 Stage: 100만 행 선택도 범위 하나로 전달합니다.
        list.tableView.selectAll(nil)
        list.toggleSelection()
        try expect(harness.spin(timeout: 120) { panel.status?.stagedChangeCount == 1_000_000 },
                   "million: 전체 Stage 결과 불일치 \(panel.status?.stagedChangeCount ?? -1)")

        // 회귀 방지용 느슨한 상한(한 프레임). 실제 계약은 아키텍처 §6.3의 단일 점유 2ms이며 값은 보고에 남깁니다.
        try expect(panel.diagnostics.maxDrainSeconds < 1.0 / 60.0,
                   "million: 이벤트 반영 한 번이 한 프레임을 넘음 \(panel.diagnostics.maxDrainSeconds * 1000)ms")
        let maxDrain = String(format: "%.2f", panel.diagnostics.maxDrainSeconds * 1000)
        report.append("million rows \(expectedRows), pages requested \(requested), cached \(cached), row views \(rowViews), max drain \(maxDrain)ms")
    }
}
