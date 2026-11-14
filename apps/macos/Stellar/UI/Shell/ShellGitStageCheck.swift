import AppKit

/// 실제 프로젝트 창에서 표시한 변경이 오래됐을 때와 유효할 때 같은 이름 행의 체크박스를 누릅니다.
enum ShellGitStageCheck {
    static func run() -> Bool {
        let args = CommandLine.arguments
        guard let i = args.firstIndex(of: "--git-stage-check"), args.indices.contains(i + 2) else { return false }
        return MainActor.assumeIsolated { check(path: args[i + 1], replacement: args[i + 2], component: args.count > i + 3 ? args[i + 3] : nil) }
    }

    @MainActor private static func check(path: String, replacement: String, component: String?) -> Bool {
        NSApplication.shared.setActivationPolicy(.accessory)
        let runtime = RuntimeHost()
        defer { runtime.shutdown() }
        let coordinator = ShellAppCoordinator(runtimeHandle: runtime.handle)
        coordinator.presentsWindows = false
        let autosave = ShellCheckAutosave(prefix: "StellarGitStageCheck")
        defer { autosave.removeRecords() }
        coordinator.windowConfiguration = { autosave.configuration }
        guard let pump = coordinator.pump else { return false }
        func fail(_ step: String) -> Bool {
            FileHandle.standardError.write(Data("git-stage 실패: \(step)\n".utf8))
            return false
        }
        func spin(_ complete: () -> Bool) -> Bool {
            let deadline = Date().addingTimeInterval(12)
            repeat {
                pump.drain()
                if complete() { return true }
                RunLoop.current.run(until: Date().addingTimeInterval(0.015))
            } while Date() < deadline
            return false
        }
        func open() -> ShellProjectWindow? {
            coordinator.openProject(atPath: path, origin: .application)
            guard pump.waitForPendingRequests(timeout: 10) else { return nil }
            return coordinator.projectWindow(forPath: path)
        }
        func showGitPanel(_ project: ShellProjectWindow) -> GitPanelViewController? {
            let controller = project.windowController
            guard let window = project.window, let pane = controller.splitController.centerController.auxiliaryItem else { return nil }
            window.orderFront(nil)
            if pane.isCollapsed { controller.toggleAuxiliaryPanel(nil) }
            let panel = project.panelsSlot.gitPanel
            panel.loadViewIfNeeded()
            let outline = panel.changeList.outlineView
            return spin {
                window.isVisible && outline.window === window && !outline.isHiddenOrHasHiddenAncestor && !outline.visibleRect.isEmpty
            } ? panel : nil
        }
        func click(_ panel: GitPanelViewController, state: GitStageState = .unstaged) -> Bool {
            let outline = panel.changeList.outlineView
            guard spin({
                guard panel.changeList.generation == panel.status?.changes.generation, !panel.isBusy else { return false }
                for i in 0..<outline.numberOfRows {
                    guard let item = outline.item(atRow: i) as? GitChangeOutlineItem else { continue }
                    if panel.changeList.row(of: item) == nil {
                        outline.scrollRowToVisible(i)
                        _ = outline.view(atColumn: 0, row: i, makeIfNecessary: true)
                        continue
                    }
                    guard let row = panel.changeList.row(of: item) else { continue }
                    if row.level != .change {
                        if component != nil, row.level == .document, row.title != String(localized: "Components") { continue }
                        if let component, row.level == .object, row.title != component { continue }
                        if !outline.isItemExpanded(item) {
                            outline.scrollRowToVisible(i)
                            _ = outline.view(atColumn: 0, row: i, makeIfNecessary: true)
                            outline.expandItem(item)
                            return false
                        }
                    } else if (component.map { row.detail == "design/components/" + $0 } ?? (row.title == String(localized: "Node name"))), row.stageState == state {
                        outline.scrollRowToVisible(i)
                        guard let cell = outline.view(atColumn: 0, row: i, makeIfNecessary: true) as? GitChangeCellView,
                              cell.stageCheckbox.isEnabled, !cell.stageCheckbox.isHidden,
                              outline.row(for: cell.stageCheckbox) == i else { continue }
                        cell.stageCheckbox.performClick(nil)
                        return true
                    }
                }
                return false
            }) else {
                let details = (0..<outline.numberOfRows).compactMap { i -> String? in
                    guard let item = outline.item(atRow: i) as? GitChangeOutlineItem, let row = panel.changeList.row(of: item) else { return nil }
                    return "\(row.title) \(row.detail) \(row.stageState)"
                }
                return fail("선택 가능한 Git 행: \(details)")
            }
            return true
        }
        guard let window = open() else { return fail("프로젝트 창 열기") }
        guard let panel = showGitPanel(window) else { return fail("프로젝트 창의 Git 패널 표시") }
        if let component {
            let file = URL(fileURLWithPath: path).appendingPathComponent("design/components").appendingPathComponent(component)
            let gitIndex = URL(fileURLWithPath: path).appendingPathComponent(".git/index")
            func counts(_ staged: Int, _ unstaged: Int) -> Bool {
                spin { !panel.isBusy && panel.status?.stagedChangeCount == staged && panel.status?.unstagedChangeCount == unstaged }
            }
            guard spin({ panel.status?.availability == .ready && (panel.status?.unstagedChangeCount ?? 0) > 0 }),
                  let staged = panel.status?.stagedChangeCount, let unstaged = panel.status?.unstagedChangeCount,
                  let original = try? Data(contentsOf: gitIndex), let baseline = try? Data(contentsOf: file),
                  let owner = window.windowController.windowSession, let shown = panel.status?.changes.generation
            else { return fail("컴포넌트 파일 변경 조회: \(String(describing: panel.status?.semanticStatusError))") }
            // 같은 파일에 편집이 추가되면 이전 checkbox로 준비된 bytes를 index에 넣지 않습니다.
            do { try (baseline + Data("\n".utf8)).write(to: file, options: .atomic) } catch { return fail("선택 파일 재편집") }
            guard click(panel), spin({ panel.status?.semanticStatusError?.contains("바뀌") == true }),
                  (try? Data(contentsOf: gitIndex)) == original else { return fail("오래된 컴포넌트 편집 보존: error=\(String(describing: panel.status?.semanticStatusError)), selected=\(component), staged=\(panel.status?.stagedChangeCount ?? -1), unstaged=\(panel.status?.unstagedChangeCount ?? -1), unchanged=\((try? Data(contentsOf: gitIndex)) == original)") }
            do { try baseline.write(to: file, options: .atomic) } catch { return fail("fixture 원본 복원") }
            window.gitSource.updateTrust(false)
            guard spin({ panel.status?.availability == .restricted }) else { return fail("컴포넌트 신뢰 철회") }
            var rejected: CoreRuntimeEvent?
            _ = pump.gitNames(owner.session, selection: (shown, 0)) { rejected = $0 }
            guard pump.waitForPendingRequests(timeout: 10), case .requestFailed? = rejected,
                  (try? Data(contentsOf: gitIndex)) == original else { return fail("제한 모드 index 불변") }
            window.gitSource.updateTrust(true)
            guard counts(staged, unstaged), click(panel), counts(staged + 1, unstaged - 1) else {
                return fail("컴포넌트 파일 Stage: \(String(describing: panel.status?.semanticStatusError))")
            }
            do { try Data(contentsOf: gitIndex).write(to: URL(fileURLWithPath: replacement)) }
            catch { return fail("검사용 stage 결과 저장") }
            guard click(panel, state: .staged), counts(staged, unstaged),
                  (try? Data(contentsOf: file)) == baseline else { return fail("컴포넌트 파일 Unstage") }
            print("component-stage ok: stale same-file edit and revoked trust rejected; selected checkbox staged and unstaged")
            return true
        }
        guard spin({ panel.status?.availability == .ready && panel.status?.unstagedChangeCount == 1 }) else {
            return fail("실제 저장된 이름 변경 조회: \(String(describing: panel.status?.semanticStatusError))")
        }
        let before = (path as NSString).appendingPathComponent(".git/index")
        guard let owner = window.windowController.windowSession,
              let shownBeforeRevoke = panel.status?.changes.generation,
              let beforeRevoke = FileManager.default.contents(atPath: before) else { return fail("철회 전 세대 읽기") }
        window.gitSource.updateTrust(false)
        guard spin({ panel.status?.availability == .restricted }) else { return fail("제한 모드 전환") }
        var restricted: CoreRuntimeEvent?
        _ = pump.gitNames(owner.session, selection: (shownBeforeRevoke, 0)) { restricted = $0 }
        guard pump.waitForPendingRequests(timeout: 10), case .requestFailed? = restricted,
              FileManager.default.contents(atPath: before) == beforeRevoke else { return fail("비신뢰 창 Stage 거부") }
        window.gitSource.updateTrust(true)
        guard spin({ panel.status?.unstagedChangeCount == 1 && panel.status?.availability == .ready }) else {
            return fail("재승인 후 현재 변경 조회")
        }
        guard let visible = panel.status?.changes.generation, let opened = window.windowController.windowSession,
              let baseline = FileManager.default.contents(atPath: before) else { return fail("세대·index 확인") }
        var cancelledResult: CoreRuntimeEvent?
        switch pump.gitNames(opened.session, selection: (visible, 0), completion: { cancelledResult = $0 }) {
        case .success(let id):
            CoreRuntime.cancelGitNames(handle: pump.runtimeHandle, session: opened.session, request: id)
        case .failure: return fail("취소 요청 접수")
        }
        guard pump.waitForPendingRequests(timeout: 10), case .requestFailed? = cancelledResult,
              FileManager.default.contents(atPath: before) == baseline else { return fail("취소 후 index 유지") }
        window.gitSource.send(.refreshStatus)
        guard spin({ (panel.status?.changes.generation ?? 0) > visible && panel.status?.unstagedChangeCount == 1 }) else { return fail("취소 후 갱신") }

        guard let original = FileManager.default.contents(atPath: before) else { return fail("index 읽기") }
        // 미리 준비한 다음 세대는 같은 프로젝트가 정상 저장해 만든 current pointer입니다.
        do {
            let next = try Data(contentsOf: URL(fileURLWithPath: replacement))
            try next.write(to: URL(fileURLWithPath: path).appendingPathComponent("design/current"), options: .atomic)
        } catch { return fail("새 저장 세대 열기") }
        guard click(panel) else { return fail("이름 행 클릭: rows=\(panel.changeList.outlineView.numberOfRows), docs=\(panel.status?.changes.documentCount ?? -1)") }
        guard spin({ panel.status?.semanticStatusError?.contains("바뀌") == true }) else { return fail("오래된 선택 오류: \(String(describing: panel.status?.semanticStatusError))") }
        guard FileManager.default.contents(atPath: before) == original else { return fail("오래된 선택이 index를 수정함") }
        window.gitSource.send(.refreshStatus)
        guard spin({ panel.status?.unstagedChangeCount == 1 }), click(panel),
              spin({ panel.status?.stagedChangeCount == 1 && panel.status?.unstagedChangeCount == 0 }) else {
            return fail("현재 이름 한 건 실제 Stage: \(String(describing: panel.status?.semanticStatusError))")
        }
        // 다른 창 구성에서 열면 보조 패널의 기본값인 접힌 상태에서 다시 시작합니다.
        let reopenedAutosave = ShellCheckAutosave(prefix: "StellarGitStageReopenedCheck")
        defer { reopenedAutosave.removeRecords() }
        coordinator.windowConfiguration = { reopenedAutosave.configuration }
        window.window?.close()
        guard pump.waitForPendingRequests(timeout: 10), let again = open(), again !== window else { return fail("창 닫기/재열기") }
        guard let reopened = showGitPanel(again) else { return fail("재열린 창의 Git 패널 표시") }
        guard spin({ reopened.status?.stagedChangeCount == 1 && reopened.status?.unstagedChangeCount == 0 }) else { return fail("재열기 index 확인") }
        // Python harness가 두 시점의 실제 index를 별도로 재열 수 있게 Stage 결과를 복사합니다.
        do {
            try Data(contentsOf: URL(fileURLWithPath: before)).write(
                to: URL(fileURLWithPath: replacement).appendingPathExtension("staged-index"))
        } catch { return fail("Stage index 복사") }
        guard click(reopened, state: .staged) else { return fail("재열린 창의 Stage된 체크박스 클릭") }
        guard spin({ reopened.status?.stagedChangeCount == 0 && reopened.status?.unstagedChangeCount == 1 }) else {
            return fail("Unstage 후 변경 개수: staged=\(reopened.status?.stagedChangeCount ?? -1), unstaged=\(reopened.status?.unstagedChangeCount ?? -1), \(String(describing: reopened.status?.semanticStatusError))")
        }
        guard let closingSession = again.windowController.windowSession else { return fail("재개한 창 session") }
        var late: CoreRuntimeEvent?
        _ = pump.gitNames(closingSession.session, selection: nil) { late = $0 }
        again.window?.close()
        guard pump.waitForPendingRequests(timeout: 10), late != nil else { return fail("Git 조회 중 창 닫기") }
        guard CoreRuntime.gitPanelRow(handle: pump.runtimeHandle, session: closingSession.session,
                                     generation: late?.request ?? 0, row: 0) == nil else { return fail("닫힌 창 행 유지") }
        print("git-stage ok: trust revoked, cancel and stale click preserved index, current click staged, reopen and unstage, close during refresh")
        return true
    }
}
