import AppKit

/// 에이전트 패널 UI 정적 검사.
///
/// 화면에 표시하지 않은 창에 Stub 데이터로 패널을 만들고 일반적인 인스펙터 크기에서 레이아웃을 강제한 뒤
/// - 모호한 Auto Layout이 없는지, 여러 줄 레이블이 잘리지 않는지
/// - 핵심 컨트롤의 접근성 레이블·도움말 태그가 있는지
/// - 채팅 보기가 기본이고 원문 텍스트 보기로 전환되는지
/// - 승인 카드의 주 내용과 에이전트 설명(출처 표시)이 분리되는지, 민감 확인이 키보드 기본 동작만으로 승인되지 않는지
/// - 개별·세션 되돌리기, 중지(자동 롤백 없음·대기 승인 만료), 보내기(IME 조합 중 Return 제외)가 명령으로 나가는지
/// - 보낸 뒤 입력창 Undo가 종료 없이 보낸 문장을 되살리는지, 안내 문구 클릭이 입력창으로 가는지, Tab이 포커스를 옮기는지
/// - Command-Return·Command-Period가 패널 안 포커스에서만 동작하는지(창 전체 키 등가 없음)
/// - 같은 승인 카드에 응답이 한 번만 나가는지, reset이 더 새로운 세대에서만 적용되는지
/// - 빈·진행·오류 상태가 다음 동작과 함께 보이는지
/// - 알림 문구에 프로젝트 데이터가 없는지
/// 를 확인합니다. Light·Dark 두 외형에서 그리기 경로도 한 번씩 실행합니다.
/// 통합 시 `scripts/macos/smoke.sh`와 `main.swift` 실행 인자에 연결합니다.
enum AgentPanelUICheck {
    /// 인스펙터 영역에 들어가는 일반적인 크기(point).
    static let typicalSize = NSSize(width: 360, height: 760)

    private struct CheckFailure: Error {
        let message: String
    }

    private static func require(_ condition: Bool, _ message: @autoclosure () -> String) throws {
        if !condition { throw CheckFailure(message: message()) }
    }

    static func run() -> Bool {
        let application = NSApplication.shared
        application.setActivationPolicy(.accessory)
        var report: [String] = []
        do {
            try checkSymbols()
            try checkNotificationText()
            try checkConfirmationAlerts()
            report.append(try checkActiveSession(appearance: .aqua))
            report.append(try checkActiveSession(appearance: .darkAqua))
            report.append(try checkInteractions())
            report.append(try checkStopExpiresPendingApprovals())
            report.append(try checkShortcutScope())
            report.append(try checkResetGeneration())
            report.append(try checkEmptyStates())
        } catch let failure as CheckFailure {
            FileHandle.standardError.write(Data("agent-panel-ui 실패: \(failure.message)\n".utf8))
            return false
        } catch {
            FileHandle.standardError.write(Data("agent-panel-ui 실패: \(error)\n".utf8))
            return false
        }
        print("agent-panel-ui ok: \(report.joined(separator: "; "))")
        return true
    }

    // MARK: - 검사 대상 창

    private final class RecordingPresenter: AgentConfirmationPresenting {
        var answer = true
        private(set) var presented: [AgentConfirmation] = []

        func present(_ confirmation: AgentConfirmation, for window: NSWindow?, completion: @escaping (Bool) -> Void) {
            presented.append(confirmation)
            completion(answer)
        }
    }

    private final class Harness {
        let stub: AgentSessionStub
        let controller: AgentPanelViewController
        let window: NSWindow
        let presenter = RecordingPresenter()
        private(set) var attentions: [AgentAttentionKind] = []

        init(scenario: AgentSessionStub.Scenario, appearance: NSAppearance.Name = .aqua) {
            stub = AgentSessionStub(scenario: scenario)
            controller = AgentPanelViewController(source: stub)
            controller.confirmationPresenter = presenter
            window = NSWindow(
                contentRect: NSRect(origin: .zero, size: AgentPanelUICheck.typicalSize),
                styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.appearance = NSAppearance(named: appearance)
            window.contentViewController = controller
            window.setContentSize(AgentPanelUICheck.typicalSize)
            controller.attentionHandler = { [weak self] kind in self?.attentions.append(kind) }
            settle()
        }

        /// 레이아웃을 강제하고, 다음 런 루프로 미룬 갱신(열 폭 변경 재측정, 이벤트 pull)을 처리합니다.
        func settle() {
            for _ in 0..<3 {
                window.contentView?.layoutSubtreeIfNeeded()
                RunLoop.current.run(until: Date().addingTimeInterval(0.02))
                controller.pumpEvents()
            }
            window.contentView?.layoutSubtreeIfNeeded()
        }

        var commands: [AgentPanelCommand] { stub.performedCommands }

        /// 행을 화면에 들여온 뒤 그 행에 배치된 셀을 돌려줍니다. 화면 밖에 셀을 따로 만들지 않습니다.
        func realizedCell(_ row: Int) throws -> AgentConversationCellView {
            let table = controller.tableView
            table.scrollRowToVisible(row)
            settle()
            let visible = table.rows(in: table.visibleRect)
            guard NSLocationInRange(row, visible),
                let cell = table.view(atColumn: 0, row: row, makeIfNecessary: false) as? AgentConversationCellView
            else {
                throw CheckFailure(message: "행 \(row)이 화면에 배치되지 않음(보이는 행 \(visible))")
            }
            return cell
        }

        func cell<Cell: AgentConversationCellView>(_ type: Cell.Type, where matches: (AgentConversationItem) -> Bool) -> Cell? {
            guard let row = controller.model.items.firstIndex(where: matches) else { return nil }
            return (try? realizedCell(row)) as? Cell
        }

        func approvalCell(_ id: UInt64) -> AgentApprovalCellView? {
            cell(AgentApprovalCellView.self) {
                if case .approval(let request) = $0.content { return request.approvalID.rawValue == id }
                return false
            }
        }

        func changeCell(_ id: UInt64) -> AgentChangeCellView? {
            cell(AgentChangeCellView.self) {
                if case .appliedChange(let change) = $0.content { return change.changeID.rawValue == id }
                return false
            }
        }

        func close() {
            window.close()
        }
    }

    /// 단축키 이벤트(⌘Return, ⌘.).
    private static func commandKey(_ characters: String, keyCode: UInt16, window: NSWindow) -> NSEvent? {
        NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [.command],
            timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber, context: nil,
            characters: characters, charactersIgnoringModifiers: characters, isARepeat: false, keyCode: keyCode)
    }

    /// 순서대로 꺼내 주는 시험용 원천(세대 규칙 검사용).
    private final class ScriptedSource: AgentPanelSource {
        let snapshot: AgentPanelSnapshot
        var events: [AgentPanelEvent] = []

        init(snapshot: AgentPanelSnapshot) {
            self.snapshot = snapshot
        }

        func currentSnapshot() -> AgentPanelSnapshot { snapshot }
        func pollEvent() -> AgentPanelEvent? { events.isEmpty ? nil : events.removeFirst() }
        func perform(_ command: AgentPanelCommand) {}
    }

    // MARK: - 공통 검사 도구

    private static func ambiguousViews(in root: NSView) -> [String] {
        var found: [String] = []
        func visit(_ view: NSView) {
            let name = String(describing: type(of: view))
            // AppKit 내부 비공개 뷰(이름이 '_'로 시작, 예: TextKit 2 렌더링 뷰)는 이 패널의 제약 대상이 아닙니다.
            if name.hasPrefix("_") { return }
            if view.hasAmbiguousLayout { found.append(name) }
            view.subviews.forEach(visit)
        }
        visit(root)
        return found
    }

    private static func requireNoAmbiguity(_ root: NSView, _ context: String) throws {
        let ambiguous = ambiguousViews(in: root)
        try require(ambiguous.isEmpty, "\(context): 모호한 Auto Layout \(ambiguous)")
    }

    private static func requireLabel(_ element: NSView, _ name: String) throws {
        let label = element.accessibilityLabel() ?? ""
        try require(!label.isEmpty, "\(name) 접근성 레이블 없음")
    }

    /// 여러 줄 레이블이 충분한 폭을 받고(지나친 줄바꿈 없음), 내용보다 낮게 배치되지 않았는지(잘림 없음) 확인합니다.
    /// 검사한 레이블 수를 반환합니다.
    @discardableResult
    private static func requireNoClipping(_ cell: AgentConversationCellView, row: Int, columnWidth: CGFloat) throws -> Int {
        var checked = 0
        for label in cell.wrappingLabels {
            guard let textCell = label.cell else { continue }
            checked += 1
            try require(
                label.frame.width >= columnWidth * 0.5,
                "행 \(row) 레이블 폭이 좁음: \(label.frame.width) / 열 \(columnWidth) ('\(label.stringValue.prefix(30))')")
            let needed = textCell.cellSize(
                forBounds: NSRect(x: 0, y: 0, width: label.frame.width, height: .greatestFiniteMagnitude)).height
            try require(
                label.frame.height + 1 >= needed,
                "행 \(row) 레이블 잘림: 높이 \(label.frame.height) < 필요 \(needed) ('\(label.stringValue.prefix(30))')")
            // 행 높이가 내용보다 지나치게 크지 않은지(재측정 누락) 확인합니다.
            try require(
                label.frame.height <= needed + 2,
                "행 \(row) 레이블 높이 과다: \(label.frame.height) > 필요 \(needed) ('\(label.stringValue.prefix(30))')")
        }
        // 행 높이가 내용보다 지나치게 크지 않은지(폭 변경 뒤 재측정 누락) 확인합니다.
        var used = NSRect.null
        func collect(_ view: NSView) {
            for subview in view.subviews where !subview.isHidden {
                if subview is NSControl || subview is NSImageView {
                    used = used.union(subview.convert(subview.bounds, to: cell))
                } else {
                    collect(subview)
                }
            }
        }
        collect(cell)
        let allowance = 2 * (AgentPanelMetrics.rowVerticalPadding + AgentPanelMetrics.cardContentInset) + 6
        try require(
            !used.isNull && cell.bounds.height - used.height <= allowance,
            "행 \(row) 여백 과다: 셀 \(cell.bounds.height), 내용 \(used.height)")
        return checked
    }

    /// 창 콘텐츠를 비트맵에 그려 draw 경로를 실행합니다(외형별 동적 색 해석 포함).
    private static func render(_ harness: Harness) throws {
        guard let content = harness.window.contentView,
            let bitmap = content.bitmapImageRepForCachingDisplay(in: content.bounds)
        else { throw CheckFailure(message: "렌더링용 bitmap 생성 실패") }
        content.cacheDisplay(in: content.bounds, to: bitmap)
    }

    // MARK: - 개별 검사

    private static func checkSymbols() throws {
        for name in AgentPanelSymbol.all {
            try require(NSImage(systemSymbolName: name, accessibilityDescription: nil) != nil, "SF Symbol 없음: \(name)")
        }
    }

    /// 알림 문구는 고정 문구이며 샘플 프로젝트 데이터(프로젝트·레이어·CLI 이름)를 포함하지 않습니다.
    private static func checkNotificationText() throws {
        let forbidden = [
            AgentSessionStub.sampleProjectName, "Sign In", AgentSessionStub.primaryCLI.displayName, "stub-model",
        ]
        for kind in AgentAttentionKind.allCases {
            let text = AgentPanelText.notificationTitle(kind) + " " + AgentPanelText.notificationBody(kind)
            try require(!text.isEmpty, "알림 문구 없음: \(kind)")
            for word in forbidden {
                try require(!text.contains(word), "알림 문구에 프로젝트 데이터 포함: \(kind) '\(word)'")
            }
        }
    }

    private static func checkConfirmationAlerts() throws {
        for kind in [AgentApprovalKind.gitCommit, .gitPush] {
            let (alert, response) = AgentAlertConfirmationPresenter.makeAlert(for: .sensitiveApproval(kind))
            try require(alert.buttons.count == 2, "민감 확인 버튼 수")
            try require(
                alert.buttons[0].title == AgentConfirmation.cancelTitle && alert.buttons[0].keyEquivalent == "\r",
                "민감 확인의 Return 기본 버튼이 Cancel이 아님")
            try require(
                alert.buttons[1].hasDestructiveAction && alert.buttons[1].keyEquivalent.isEmpty,
                "민감 확인 버튼이 파괴적 동작 표시가 없거나 키 등가가 있음")
            try require(response == .alertSecondButtonReturn, "민감 확인 응답 매핑")
        }
        let start = AgentConfirmation.startSession(
            projectName: AgentSessionStub.sampleProjectName, cli: AgentSessionStub.primaryCLI,
            mode: .reviewBeforeApplying)
        try require(
            start.informativeText.contains(AgentSessionStub.sampleProjectName)
                && start.informativeText.contains(AgentSessionStub.primaryCLI.displayName)
                && start.informativeText.contains(AgentPanelText.modeTitle(.reviewBeforeApplying)),
            "세션 시작 확인에 대상 프로젝트·CLI·모드가 없음")
        let (startAlert, startResponse) = AgentAlertConfirmationPresenter.makeAlert(for: start)
        try require(
            startAlert.buttons.first?.title == start.confirmTitle && startResponse == .alertFirstButtonReturn,
            "세션 시작 확인 버튼 구성")
    }

    private static func checkActiveSession(appearance: NSAppearance.Name) throws -> String {
        let harness = Harness(scenario: .activeSession, appearance: appearance)
        defer { harness.close() }
        let controller = harness.controller
        let header = controller.headerView!
        let context = "활성 세션(\(appearance.rawValue))"

        // 채팅 보기가 기본입니다.
        try require(controller.displayMode == .chat, "\(context): 기본 보기가 채팅이 아님")
        try require(
            !controller.chatScrollView.isHidden && controller.rawScrollView.isHidden && controller.stateView.isHidden,
            "\(context): 채팅 보기 표시 상태 불일치")
        try require(header.displayModeControl.selectedSegment == AgentConversationDisplayMode.chat.rawValue, "보기 전환 컨트롤 선택")
        // 처음 열면 최신 항목(마지막 행)이 보입니다.
        let visibleRows = controller.tableView.rows(in: controller.tableView.visibleRect)
        try require(
            NSMaxRange(visibleRows) == controller.model.items.count,
            "\(context): 처음 표시에서 마지막 항목이 보이지 않음 \(visibleRows)")

        // 헤더: 사용 중인 CLI와 연결 상태, CLI가 보고한 모델.
        try require(
            header.cliPopUp.titleOfSelectedItem?.contains(AgentSessionStub.primaryCLI.displayName) == true,
            "\(context): 헤더에 사용 중인 CLI가 없음")
        try require(header.statusLabel.stringValue == "Connected", "\(context): 연결 상태 '\(header.statusLabel.stringValue)'")
        try require(!header.modelLabel.isHidden && header.modelLabel.stringValue.contains("stub-model"), "모델 표시")
        try require(!header.cliPopUp.isEnabled, "세션 중 CLI 선택이 활성화됨")
        try require(
            header.modePopUp.titleOfSelectedItem == AgentPanelText.modeTitle(.reviewBeforeApplying), "모드 표시")

        // 접근성 레이블·도움말 태그.
        let composer = controller.composerView
        let labelled: [(NSView, String)] = [
            (header.cliPopUp, "CLI 선택"), (header.moreButton, "더 보기"), (header.displayModeControl, "보기 전환"),
            (header.modePopUp, "모드"), (controller.tableView, "대화 기록"), (controller.rawTextView, "원문 텍스트"),
            (composer.textView, "입력창"), (composer.sendButton, "보내기"), (composer.stopButton, "중지"),
            (controller.view, "패널"),
        ]
        for (view, name) in labelled { try requireLabel(view, name) }
        for button in [composer.sendButton!, composer.stopButton!, ] {
            try require(!(button.toolTip ?? "").isEmpty, "아이콘 버튼 도움말 태그 없음")
        }
        try require(!(header.moreButton.toolTip ?? "").isEmpty, "더 보기 도움말 태그 없음")
        // ⌘Return·⌘.은 패널 루트 뷰가 포커스를 확인해 처리합니다. 버튼 키 등가는 창 전체에서 발동하므로 두지 않습니다.
        try require(
            composer.sendButton.keyEquivalent.isEmpty && composer.stopButton.keyEquivalent.isEmpty,
            "보내기·중지 버튼에 창 전체 키 등가가 있음")
        try require(controller.view is AgentPanelRootView, "패널 루트 뷰가 단축키 범위 뷰가 아님")
        try require(composer.stopButton.isEnabled, "활성 세션에서 중지 비활성")

        // 세션 변경 막대.
        try require(controller.isSessionBarVisible, "\(context): 세션 변경 막대 숨김")
        try require(controller.sessionBar.changesLabel.stringValue == AgentPanelText.changeSummary(
            AgentSessionChangeSummary(appliedCount: 3, revertableCount: 2)), "변경 집계 문구")
        try require(
            controller.sessionBar.revertAllButton.isEnabled && controller.sessionBar.revertAllButton.hasDestructiveAction,
            "세션 되돌리기 버튼 상태")

        // 모든 행: 행마다 화면에 들여온 직후 모호성, 폭, 잘림, 접근성을 검사합니다
        // (화면 밖으로 나간 셀은 표가 재사용하므로 나중에 검사하지 않습니다).
        let columnWidth = controller.tableView.tableColumns[0].width
        try require(
            columnWidth > typicalSize.width - 2 * AgentPanelMetrics.edgeInset,
            "\(context): 열 폭 \(columnWidth) (기록 \(controller.chatScrollView.frame.size))")
        let rowCount = controller.model.items.count
        try require(controller.tableView.numberOfRows == rowCount, "\(context): 행 수 불일치")
        var checkedLabels = 0
        for row in 0..<rowCount {
            let cell = try harness.realizedCell(row)
            try requireNoAmbiguity(cell, "\(context) 행 \(row)")
            checkedLabels += try requireNoClipping(cell, row: row, columnWidth: columnWidth)
            try requireLabel(cell, "행 \(row)")
            try require(abs(cell.frame.width - columnWidth) < 2, "행 \(row) 셀 폭 \(cell.frame.width) != 열 폭")
        }
        try requireNoAmbiguity(harness.window.contentView!, context)

        // 승인 카드: 일반 편집(대기), 민감(대기), 처리됨.
        guard let commitCard = harness.approvalCell(2), let commitApprove = commitCard.approveButton else {
            throw CheckFailure(message: "Git commit 승인 카드 없음")
        }
        try require(commitApprove.hasDestructiveAction, "민감 승인 버튼에 파괴적 동작 표시 없음")
        try require(commitApprove.keyEquivalent.isEmpty, "민감 승인 버튼에 키 등가 있음")
        try require(commitApprove.title == AgentPanelText.approveSensitive, "민감 승인 버튼 제목")
        try require(
            commitCard.agentProvenanceLabel?.stringValue == AgentPanelText.agentNoteProvenance
                && commitCard.agentNoteLabel != nil,
            "에이전트 설명의 출처 표시 없음")
        try require(commitCard.accessibilityCustomActions()?.count == 2, "승인 카드 접근성 동작")
        guard let editCard = harness.approvalCell(3), let editApprove = editCard.approveButton else {
            throw CheckFailure(message: "편집 승인 카드 없음")
        }
        try require(!editApprove.hasDestructiveAction && editApprove.keyEquivalent.isEmpty, "일반 승인 버튼 구성")
        try require(editCard.agentNoteLabel == nil, "설명 없는 카드에 에이전트 설명 표시")
        try requireLabel(editApprove, "승인 버튼")
        try requireLabel(editCard.declineButton!, "거부 버튼")
        guard let doneCard = harness.approvalCell(1) else { throw CheckFailure(message: "처리된 승인 카드 없음") }
        try require(doneCard.approveButton == nil && doneCard.declineButton == nil, "처리된 카드에 승인 버튼 표시")

        // 변경 행: 적용됨은 되돌리기 가능, 되돌림은 버튼 숨김.
        guard let applied = harness.changeCell(1), let reverted = harness.changeCell(2) else {
            throw CheckFailure(message: "변경 행 없음")
        }
        try require(!applied.revertButton.isHidden && applied.revertButton.isEnabled, "적용된 변경의 되돌리기 버튼")
        try requireLabel(applied.revertButton, "되돌리기 버튼")
        try require(reverted.revertButton.isHidden, "되돌린 변경에 되돌리기 버튼 표시")
        try require(applied.captionLabel.stringValue.hasPrefix(AgentPanelText.appliedByAgent), "변경 출처 표시")

        // 접근성 표시 설정 변경 경로와 그리기.
        controller.accessibilityDisplayOptionsChanged()
        try render(harness)
        try require(checkedLabels >= rowCount, "\(context): 검사한 여러 줄 레이블 수 \(checkedLabels)")
        return "\(appearance == .darkAqua ? "dark" : "light") \(rowCount)행·레이블 \(checkedLabels)개"
    }

    private static func checkInteractions() throws -> String {
        let harness = Harness(scenario: .activeSession)
        defer { harness.close() }
        let controller = harness.controller
        let presenter = harness.presenter

        // 1) 민감 승인: 확인을 거부하면 명령이 나가지 않고, 확인하면 나갑니다.
        presenter.answer = false
        harness.approvalCell(2)?.approveButton?.performClick(nil)
        try require(presenter.presented.last?.kind == .sensitiveApproval(.gitCommit), "민감 승인 확인 화면 없음")
        let approveCommit = AgentPanelCommand.respondToApproval(AgentApprovalID(rawValue: 2), .approve)
        try require(!harness.commands.contains(approveCommit), "확인 거부 후에도 민감 승인 명령이 나감")
        presenter.answer = true
        harness.approvalCell(2)?.approveButton?.performClick(nil)
        try require(harness.commands.last == approveCommit, "확인 후 민감 승인 명령 없음")
        harness.settle()
        try require(harness.approvalCell(2)?.approveButton == nil, "승인 후 카드가 갱신되지 않음")

        // 2) 일반 승인은 확인 화면 없이 바로 명령으로 나갑니다(첫 응답만 유효).
        //    코어 갱신 전 두 번째 클릭·거부는 명령으로 나가지 않습니다(카드 버튼이 꺼짐).
        let presentedBefore = presenter.presented.count
        guard let editCard = harness.approvalCell(3), let editApprove = editCard.approveButton,
            case .approval(let editRequest)? = controller.model.items.first(where: {
                if case .approval(let request) = $0.content { return request.approvalID.rawValue == 3 }
                return false
            })?.content
        else { throw CheckFailure(message: "편집 승인 카드 없음") }
        editApprove.performClick(nil)
        try require(presenter.presented.count == presentedBefore, "일반 승인에 확인 화면이 뜸")
        try require(
            harness.commands.last == .respondToApproval(AgentApprovalID(rawValue: 3), .approve), "일반 승인 명령 없음")
        try require(
            !editApprove.isEnabled && editCard.declineButton?.isEnabled == false, "응답 뒤 카드 버튼이 켜져 있음")
        editApprove.performClick(nil)
        // 셀을 거치지 않고 컨트롤러에 바로 요청해도 두 번째 응답은 나가지 않습니다(검사는 메인 스레드에서 실행).
        MainActor.assumeIsolated { controller.conversationRequestsDecision(.decline, for: editRequest) }
        let responses = harness.commands.filter {
            if case .respondToApproval(let id, _) = $0 { return id.rawValue == 3 }
            return false
        }
        try require(responses.count == 1, "같은 승인에 응답이 여러 번 나감 \(responses)")
        harness.settle()
        try require(controller.approvalsAwaitingUpdate.isEmpty, "코어 갱신 뒤에도 응답 대기 표시가 남음")
        try require(controller.model.changeSummary.appliedCount == 4, "승인 후 변경 집계 갱신 없음")

        // 3) 개별 되돌리기.
        harness.changeCell(1)?.revertButton.performClick(nil)
        try require(harness.commands.last == .revertChange(AgentChangeID(rawValue: 1)), "개별 되돌리기 명령 없음")
        harness.settle()
        try require(harness.changeCell(1)?.revertButton.isHidden == true, "되돌린 변경 행 갱신 없음")

        // 4) 세션 단위 되돌리기는 파괴적 확인을 거칩니다.
        controller.sessionBar.revertAllButton.performClick(nil)
        try require(presenter.presented.last?.kind == .revertAllSessionChanges, "세션 되돌리기 확인 없음")
        try require(harness.commands.last == .revertAllSessionChanges, "세션 되돌리기 명령 없음")
        harness.settle()
        try require(!controller.sessionBar.revertAllButton.isEnabled, "되돌릴 변경이 없는데 세션 되돌리기 활성")

        // 5) 보내기: Return은 보내고, IME 조합 중 Return은 조합 확정에 남겨 둡니다.
        //    입력은 텍스트 시스템 경로(insertText)로 넣어 실제 타이핑처럼 Undo 기록을 남깁니다.
        let composer = controller.composerView
        let textView = composer.textView
        harness.window.makeFirstResponder(textView)
        textView.insertText("Hello", replacementRange: NSRange(location: NSNotFound, length: 0))
        harness.settle()  // 타이핑 Undo 묶음을 닫습니다(런 루프 한 번).
        try require(composer.sendButton.isEnabled, "입력 후 보내기 비활성")
        let handled = composer.textView(textView, doCommandBy: #selector(NSResponder.insertNewline(_:)))
        try require(handled && harness.commands.last == .sendMessage("Hello"), "Return 보내기 명령 없음")
        try require(textView.string.isEmpty, "보낸 뒤 입력창이 비지 않음")
        harness.settle()

        // 5-1) 회귀: 보낸 뒤 ⌘Z가 종료되지 않고 보낸 문장을 되살립니다. 입력을 텍스트 시스템 밖에서 비우면
        //      이전 타이핑 Undo가 없는 범위를 가리켜 NSRangeException으로 프로세스가 끝납니다.
        guard let undoManager = textView.undoManager else { throw CheckFailure(message: "입력창 Undo 관리자 없음") }
        try require(undoManager.canUndo, "보낸 뒤 입력창 Undo 기록 없음")
        undoManager.undo()
        try require(textView.string == "Hello", "Undo가 보낸 문장을 되살리지 않음 '\(textView.string)'")
        undoManager.undo()
        try require(textView.string.isEmpty, "두 번째 Undo가 타이핑을 되돌리지 않음 '\(textView.string)'")
        undoManager.redo()
        undoManager.redo()
        try require(textView.string.isEmpty, "Redo 뒤 입력창 상태 '\(textView.string)'")
        harness.settle()

        // 5-2) 안내 문구 위 클릭은 입력창으로 갑니다(안내 문구가 클릭을 가로채지 않음).
        let placeholder = composer.placeholderLabel
        try require(!placeholder.isHidden, "빈 입력창에 안내 문구가 없음")
        let pointInWindow = placeholder.convert(NSPoint(x: placeholder.bounds.midX, y: placeholder.bounds.midY), to: nil)
        let hit = harness.window.contentView?.hitTest(pointInWindow)
        // 창은 mouseDown의 hit test 대상이 first responder를 받을 수 있으면 그 뷰에 포커스를 줍니다. 화면에 띄우지
        // 않은 창은 key 창이 되지 않아 실제 클릭 전달은 검사하지 않고, 대상과 포커스 가능 여부를 확인합니다.
        try require(
            hit.map { $0 === textView || $0.isDescendant(of: textView) } == true,
            "안내 문구 위 클릭 대상이 입력창이 아님: \(hit.map { String(describing: type(of: $0)) } ?? "nil")")
        try require(textView.acceptsFirstResponder, "입력창이 포커스를 받을 수 없음")

        // 5-3) Tab은 탭 문자를 넣지 않고 다음 컨트롤로 포커스를 옮깁니다(명세 6장).
        try require(textView.nextKeyView === composer.stopButton, "입력창 다음 key view가 중지 버튼이 아님")
        // (버튼이 key view가 되는지는 시스템 키보드 탐색 설정에 따르므로 포커스 위치는 검사하지 않습니다.)
        let tabHandled = composer.textView(textView, doCommandBy: #selector(NSResponder.insertTab(_:)))
        try require(tabHandled && !textView.string.contains("\t"), "Tab이 입력창에 탭 문자를 넣음")
        harness.window.makeFirstResponder(textView)
        let countBeforeIME = harness.commands.count
        textView.setMarkedText(
            "ㅎ", selectedRange: NSRange(location: 1, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0))
        let imeHandled = composer.textView(textView, doCommandBy: #selector(NSResponder.insertNewline(_:)))
        try require(!imeHandled && harness.commands.count == countBeforeIME, "IME 조합 중 Return이 보내기로 처리됨")
        textView.unmarkText()
        composer.clear()
        harness.settle()

        // 6) 보기 전환: 원문 텍스트 보기와 채팅 보기.
        let header = controller.headerView!
        header.displayModeControl.selectedSegment = AgentConversationDisplayMode.rawText.rawValue
        controller.displayModeChanged(header.displayModeControl)
        harness.settle()
        try require(
            controller.displayMode == .rawText && !controller.rawScrollView.isHidden && controller.chatScrollView.isHidden,
            "원문 텍스트 보기 전환 실패")
        try require(controller.rawTextView.string.contains("Raw CLI output"), "원문 텍스트 내용 없음")
        try require(controller.rawTextView.string.contains("> Hello"), "원문 텍스트 이어 붙이기 없음")
        try require(!controller.rawTextView.isEditable, "원문 텍스트가 편집 가능")
        try requireNoAmbiguity(harness.window.contentView!, "원문 텍스트 보기")
        controller.showAgentChatView(nil)
        try require(controller.displayMode == .chat && !controller.chatScrollView.isHidden, "채팅 보기 복귀 실패")

        // 7) 문맥 메뉴: 적용된 변경 행에서 되돌리기 제공.
        if let row = controller.model.items.firstIndex(where: {
            if case .appliedChange(let change) = $0.content { return change.state == .applied }
            return false
        }) {
            controller.tableView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
            let menu = NSMenu()
            controller.menuNeedsUpdate(menu)
            try require(menu.items.contains { $0.title == AgentPanelText.revertToolTip }, "문맥 메뉴에 되돌리기 없음")
            try require(menu.items.first?.title == AgentPanelText.copy, "문맥 메뉴에 복사 없음")
        }

        // 8) 중지: 명령만 보내고 자동 롤백하지 않습니다. 변경은 표시한 채 남습니다.
        let summaryBeforeStop = controller.model.changeSummary
        let countBeforeStop = harness.commands.count
        composer.stopButton.performClick(nil)
        try require(harness.commands.count == countBeforeStop + 1 && harness.commands.last == .stopSession, "중지 명령")
        harness.settle()
        try require(controller.model.header.connection == .ended, "중지 후 연결 상태")
        try require(controller.model.changeSummary == summaryBeforeStop, "중지가 변경 집계를 바꿈(자동 롤백 의심)")
        try require(controller.isSessionBarVisible, "중지 후 세션 막대 숨김")
        try require(!composer.sendButton.isEnabled && !composer.stopButton.isEnabled, "중지 후 입력 컨트롤 활성")
        try require(harness.attentions == [.sessionEnded], "세션 종료 주의 요청 \(harness.attentions)")
        try require(!controller.requiresConfirmationBeforeClosing, "종료된 세션인데 닫기 확인 필요")
        let stopItem = NSMenuItem(title: "", action: #selector(AgentPanelViewController.stopAgentSession(_:)), keyEquivalent: "")
        try require(!controller.validateMenuItem(stopItem), "종료된 세션에서 중지 메뉴 활성")
        // 대기 승인 만료는 승인하지 않은 카드가 남은 세션에서 따로 검사합니다(checkStopExpiresPendingApprovals).

        // 9) 새 세션: 시작 확인(프로젝트·CLI·모드)을 거쳐 새 세대로 교체됩니다.
        try require(controller.canStartSession, "종료 후 새 세션 시작 불가")
        controller.sessionBar.startButton.performClick(nil)
        try require(presenter.presented.last?.kind == .startSession, "세션 시작 확인 없음")
        try require(
            harness.commands.last == .startSession(cli: AgentSessionStub.primaryCLI.id, mode: .reviewBeforeApplying),
            "세션 시작 명령")
        harness.settle()
        try require(controller.model.generation == 2 && controller.model.items.count == 1, "새 세션 교체(reset) 실패")
        try require(controller.requiresConfirmationBeforeClosing, "활성 세션인데 닫기 확인 불필요")
        try requireNoAmbiguity(harness.window.contentView!, "새 세션")
        return "상호작용 \(harness.commands.count)개 명령"
    }

    /// 세션을 중지하면 대기 중인 승인(2: Git commit, 3: 편집)이 거부로 정리되고 카드에서 버튼이 사라집니다.
    private static func checkStopExpiresPendingApprovals() throws -> String {
        let harness = Harness(scenario: .activeSession)
        defer { harness.close() }
        let controller = harness.controller
        let pendingIDs: [UInt64] = [2, 3]
        func status(_ id: UInt64) -> AgentApprovalStatus? {
            for item in controller.model.items {
                if case .approval(let request) = item.content, request.approvalID.rawValue == id { return request.status }
            }
            return nil
        }
        for id in pendingIDs {
            try require(status(id) == .pending, "중지 전 승인 \(id)이 대기 상태가 아님")
            try require(harness.approvalCell(id)?.approveButton != nil, "중지 전 승인 \(id) 카드에 버튼 없음")
        }
        controller.composerView.stopButton.performClick(nil)
        harness.settle()
        try require(controller.model.header.connection == .ended, "중지 후 연결 상태")
        for id in pendingIDs {
            try require(status(id) == .expired, "중지 후 승인 \(id) 상태 \(String(describing: status(id)))")
            guard let card = harness.approvalCell(id) else { throw CheckFailure(message: "중지 후 승인 \(id) 카드 없음") }
            try require(
                card.approveButton == nil && card.declineButton == nil && card.accessibilityCustomActions() == nil,
                "중지 후 승인 \(id) 카드에 응답 수단이 남음")
        }
        try require(
            !harness.commands.contains { if case .respondToApproval = $0 { return true }; return false },
            "중지가 승인 응답 명령을 보냄")
        return "중지 시 대기 승인 \(pendingIDs.count)건 만료"
    }

    /// ⌘Return·⌘.은 패널 안에 포커스가 있을 때만 동작합니다. 패널 밖(같은 창의 다른 컨트롤)에서는 넘깁니다.
    private static func checkShortcutScope() throws -> String {
        let stub = AgentSessionStub(scenario: .activeSession)
        let controller = AgentPanelViewController(source: stub)
        controller.confirmationPresenter = RecordingPresenter()
        let outside = NSTextField(string: "")
        outside.translatesAutoresizingMaskIntoConstraints = false
        let container = NSView()
        let size = NSSize(width: typicalSize.width, height: typicalSize.height + 40)
        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: size), styleMask: [.titled, .closable, .resizable],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        window.contentView = container
        let panel = controller.view
        panel.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(outside)
        container.addSubview(panel)
        NSLayoutConstraint.activate([
            outside.topAnchor.constraint(equalTo: container.topAnchor, constant: 8),
            outside.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 8),
            outside.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -8),
            panel.topAnchor.constraint(equalTo: outside.bottomAnchor, constant: 8),
            panel.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            panel.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            panel.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])
        container.layoutSubtreeIfNeeded()
        controller.pumpEvents()

        guard let send = commandKey("\r", keyCode: 36, window: window),
            let stop = commandKey(".", keyCode: 47, window: window)
        else { throw CheckFailure(message: "단축키 이벤트 생성 실패") }
        let textView = controller.composerView.textView
        textView.insertText("Scoped", replacementRange: NSRange(location: NSNotFound, length: 0))

        // 패널 밖 포커스: 넘기고 명령을 보내지 않습니다.
        window.makeFirstResponder(outside)
        let outsideSend = window.performKeyEquivalent(with: send)
        let outsideStop = window.performKeyEquivalent(with: stop)
        try require(!outsideSend && !outsideStop, "패널 밖 포커스에서 패널 단축키가 처리됨")
        try require(stub.performedCommands.isEmpty, "패널 밖 포커스에서 명령이 나감 \(stub.performedCommands)")

        // 입력창 포커스: ⌘Return 보내기.
        window.makeFirstResponder(textView)
        try require(window.performKeyEquivalent(with: send), "입력창 포커스에서 ⌘Return 미처리")
        try require(stub.performedCommands == [.sendMessage("Scoped")], "⌘Return 보내기 명령 \(stub.performedCommands)")
        try require(textView.string.isEmpty, "⌘Return 보낸 뒤 입력창이 비지 않음")

        // 대화 기록 포커스(패널 안): ⌘. 세션 중지.
        window.makeFirstResponder(controller.tableView)
        try require(window.performKeyEquivalent(with: stop), "대화 기록 포커스에서 ⌘. 미처리")
        try require(stub.performedCommands.last == .stopSession, "⌘. 중지 명령 \(stub.performedCommands)")
        return "단축키 범위(패널 밖 무시, 안에서 보내기·중지)"
    }

    /// reset은 더 새로운 세대에서만 적용합니다. 같은 세대 reset과 세대가 어긋난 reset은 버립니다(명세 9장).
    private static func checkResetGeneration() throws -> String {
        let initial = AgentSessionStub.makeSnapshot(.activeSession)
        let source = ScriptedSource(snapshot: initial)
        let controller = AgentPanelViewController(source: source)
        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: typicalSize), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        window.contentViewController = controller
        window.contentView?.layoutSubtreeIfNeeded()

        let extra = AgentConversationItem(id: AgentItemID(rawValue: 9_001), content: .userMessage("Incremental"))
        source.events = [
            AgentPanelEvent(generation: initial.generation, change: .itemsAppended([extra])),
            AgentPanelEvent(generation: initial.generation, change: .reset(initial)),
        ]
        controller.pumpEvents()
        try require(controller.model.items.count == initial.items.count + 1, "같은 세대 reset이 증분 기록을 지움")

        var next = initial
        next.generation = initial.generation + 1
        next.items = [AgentConversationItem(id: AgentItemID(rawValue: 1), content: .notice(.leasesReleased))]
        source.events = [AgentPanelEvent(generation: initial.generation, change: .reset(next))]
        controller.pumpEvents()
        try require(controller.model.generation == initial.generation, "이벤트·스냅샷 세대가 어긋난 reset이 적용됨")

        source.events = [AgentPanelEvent(generation: next.generation, change: .reset(next))]
        controller.pumpEvents()
        try require(
            controller.model.generation == next.generation && controller.model.items.count == 1, "새 세대 reset 미적용")
        return "reset 세대 규칙"
    }

    private static func checkEmptyStates() throws -> String {
        let expectations: [(AgentSessionStub.Scenario, AgentPanelEmptyState.Action?, Bool)] = [
            (.noSession, .startSession, false),
            (.noInstalledCLI, .refreshInstalledCLIs, false),
            (.authenticationRequired, .openSignInTerminal, false),
            (.unsupportedVersion, .refreshInstalledCLIs, false),
            (.projectNotTrusted, .reviewProjectTrust, false),
            (.loading, nil, true),
            (.failed, .refreshInstalledCLIs, false),
        ]
        for (scenario, action, showsProgress) in expectations {
            let harness = Harness(scenario: scenario)
            defer { harness.close() }
            let controller = harness.controller
            let context = "빈 상태 \(scenario)"
            try require(!controller.stateView.isHidden, "\(context): 상태 뷰 숨김")
            try require(controller.chatScrollView.isHidden && controller.rawScrollView.isHidden, "\(context): 기록 표시")
            try require(controller.stateView.state?.action == action, "\(context): 동작 \(String(describing: controller.stateView.state?.action))")
            try require(controller.stateView.state?.showsProgress == showsProgress, "\(context): 진행 표시")
            try require(controller.stateView.showsActionButton == (action != nil), "\(context): 동작 버튼 표시")
            try require(!controller.composerView.sendButton.isEnabled, "\(context): 보내기 활성")
            try requireLabel(controller.stateView, context)
            try requireNoAmbiguity(harness.window.contentView!, context)
            try render(harness)

            guard action != nil else { continue }
            let before = harness.commands.count
            controller.stateView.actionButton.performClick(nil)
            try require(harness.commands.count == before + 1, "\(context): 동작 버튼이 명령을 보내지 않음")
            switch scenario {
            case .noSession:
                try require(harness.presenter.presented.last?.kind == .startSession, "\(context): 시작 확인 없음")
            case .authenticationRequired:
                try require(
                    harness.commands.last == .openSignInTerminal(AgentSessionStub.secondaryCLI.id), "\(context): 로그인 터미널")
            default:
                break
            }
        }
        return "빈·진행·오류 \(expectations.count)종"
    }
}
