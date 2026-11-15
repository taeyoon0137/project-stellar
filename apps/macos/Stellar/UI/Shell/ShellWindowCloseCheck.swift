import AppKit

#if DEBUG
/// 정상 창 닫기와 앱 종료(⌘Q)가 저장하지 않은 편집을 Save Now C ABI로 저장하는지 검사합니다.
///
/// 편집은 제품 경로인 레이어 이름 변경으로 만들고, 다시 연 창의 레이어 목록에서 저장된 이름을 확인합니다.
/// 신뢰 기록과 복구 체크포인트를 사용자 데이터에 쓰므로 smoke는 검사 전용 HOME에서 실행합니다.
enum ShellWindowCloseCheck {
    static func run(arguments: [String]) -> Bool {
        guard let flag = arguments.firstIndex(of: "--window-close-check"), arguments.indices.contains(flag + 1) else {
            return false
        }
        let fixture = URL(fileURLWithPath: arguments[flag + 1])
        return MainActor.assumeIsolated {
            NSApplication.shared.setActivationPolicy(.accessory)
            return ShellWindowCloseCheckRunner(fixture: fixture).run()
        }
    }
}

@MainActor
private final class ShellWindowCloseCheckRunner {
    let fixture: URL
    var failures: [String] = []
    var checks = 0
    var alerts: [String] = []
    /// fixture 문서 View 3(root 10) 아래의 Frame 11.
    let node = UUID(uuidString: "00000000-000b-7000-8000-00000000000b")!
    init(fixture: URL) { self.fixture = fixture }

    func expect(_ value: Bool, _ message: String) {
        checks += 1
        if !value { failures.append(message) }
    }

    func run() -> Bool {
        let projects = ShellSavedProjectFixture(prefix: "StellarWindowCloseCheck")
        let autosave = ShellCheckAutosave(prefix: "StellarWindowCloseCheck")
        defer { projects.remove(); autosave.removeRecords() }
        let host = RuntimeHost()
        guard host.created else { return false }
        defer { host.shutdown() }
        let coordinator = ShellAppCoordinator(runtimeHandle: host.handle)
        coordinator.presentsWindows = false
        coordinator.windowConfiguration = { autosave.configuration }
        coordinator.alertPresenter = { [weak self] _, message, _ in self?.alerts.append(message) }
        guard let path = projects.copy(fixture, as: "Project"), let pump = coordinator.pump else { return false }
        defer {
            for window in coordinator.projectWindows { window.window?.close() }
            _ = pump.waitForPendingRequests(timeout: 5)
        }
        func wait() { expect(pump.waitForPendingRequests(timeout: 5), "코어 완료 대기 실패") }
        func open() -> ShellProjectWindow? {
            coordinator.openProject(atPath: path, origin: .application)
            wait()
            guard let window = coordinator.projectWindow(forPath: path), window.windowController.windowSession != nil else {
                expect(false, "창 열기 실패")
                return nil
            }
            return window
        }
        func rename(_ window: ShellProjectWindow, _ name: String) {
            // D1: 신뢰를 부여하지 않은 제한 모드 그대로 편집·닫기 저장을 확인합니다.
            expect(!window.gitSource.trusted, "제한 모드가 아닌 창에서 편집 검사")
            guard let session = window.windowController.windowSession, let view = session.viewInfo,
                  view.has_view == 1, let info = session.info else {
                expect(false, "이름 변경 대상 창 상태 없음")
                return
            }
            expect(session.track({ pump.renameLayer(session: session.session, view: UUID(uuid: view.active_view),
                                                    node: node, revision: info.revision, name: name, completion: $0) }) {
                [weak self] event in
                if case .requestFailed(_, _, let message) = event { self?.expect(false, "레이어 이름 변경 실패: \(message)") }
            }, "레이어 이름 변경 접수 실패")
            wait()
            expect(session.info?.has_unsaved_changes == 1, "레이어 이름 변경이 dirty를 만들지 않음: \(name)")
        }
        func savedName(_ window: ShellProjectWindow) -> String? {
            guard let session = window.windowController.windowSession, let view = session.viewInfo,
                  view.has_view == 1, let info = session.info else { return nil }
            let handle = pump.runtimeHandle, id = session.session, revision = info.revision
            let viewID = UUID(uuid: view.active_view), target = node
            var name: String?
            var finished = false
            DispatchQueue.global(qos: .userInitiated).async {
                let page = CoreRuntime.layerPage(handle: handle, session: id, view: viewID, revision: revision,
                                                 parent: nil, after: nil)
                let found = page?.rows.first { $0.id.uuid == target }?.name
                DispatchQueue.main.async {
                    name = found
                    finished = true
                }
            }
            let deadline = Date().addingTimeInterval(5)
            while !finished, Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.01)) }
            return name
        }

        // 선택만 바뀐 창은 dirty가 아니므로 저장 요청 없이 즉시 닫힙니다.
        guard let clean = open(), let cleanSession = clean.windowController.windowSession else { return report() }
        let cleanBytes = bytes(path)
        expect(clean.selectNodes([node]), "선택 접수 실패")
        wait()
        expect(cleanSession.info?.has_unsaved_changes == 0, "선택이 dirty를 만듦")
        clean.window?.performClose(nil)
        expect(coordinator.projectWindows.isEmpty && cleanSession.isClosed, "clean 창 즉시 닫기 실패")
        wait()
        expect(bytes(path) == cleanBytes && alerts.isEmpty, "clean 창 닫기에서 쓰기 또는 경고 발생")

        // 정상 닫기: 저장이 성공한 뒤에만 창과 session이 닫힙니다.
        guard let dirty = open(), let dirtySession = dirty.windowController.windowSession else { return report() }
        rename(dirty, "닫기 저장")
        dirty.window?.performClose(nil)
        expect(coordinator.projectWindows.contains { $0 === dirty } && dirty.isWaitingToClose, "저장 전에 창이 닫힘")
        wait()
        expect(coordinator.projectWindows.isEmpty && dirtySession.isClosed, "저장 후 창 닫기 실패")
        expect(bytes(path) != cleanBytes && alerts.isEmpty, "정상 닫기 저장 bytes 또는 경고 오류")
        guard let closed = open() else { return report() }
        expect(savedName(closed) == "닫기 저장", "정상 닫기로 저장한 이름을 다시 읽지 못함")

        // 활성 에이전트 세션이 있는 창이 있으면 종료 전에 한 번 확인하고, 취소하면 종료하지 않습니다(ADR-0005 결정 4.5).
        closed.hasActiveAgentSession = { true }
        expect(coordinator.requiresConfirmationBeforeTerminating, "활성 에이전트 fixture 없음")
        var confirmations = 0
        let cancelledQuit = coordinator.prepareToTerminate(confirmAgent: {
            confirmations += 1
            expect(coordinator.prepareToTerminate(confirmAgent: { confirmations += 1; return true }, reply: { _ in }) ==
                .terminateCancel, "확인 창 중복 종료")
            return false
        }, reply: { _ in self.expect(false, "사용자 취소 후 비동기 종료 응답") })
        expect(cancelledQuit == .terminateCancel && confirmations == 1, "활성 에이전트 종료 취소")
        closed.hasActiveAgentSession = { false }

        // 앱 종료(⌘Q): 모든 창의 저장이 끝난 뒤에만 종료를 승인합니다.
        guard let quitSession = closed.windowController.windowSession else { return report() }
        rename(closed, "종료 저장")
        var replies: [Bool] = []
        expect(coordinator.prepareToTerminate(confirmAgent: { true }) { replies.append($0) } == .terminateLater,
               "dirty 창이 있는데 종료를 바로 판단함")
        let deadline = Date().addingTimeInterval(6)
        while replies.isEmpty, Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.01)) }
        expect(replies == [true] && quitSession.info?.has_unsaved_changes == 0, "종료 전 저장 또는 승인 실패")
        expect(coordinator.prepareToTerminate(confirmAgent: { true }) { _ in } == .terminateNow,
               "저장된 창만 남았는데 즉시 종료하지 않음")
        closed.window?.close()
        wait()
        guard let quit = open() else { return report() }
        expect(savedName(quit) == "종료 저장", "종료 전 저장한 이름을 다시 읽지 못함")
        quit.window?.close()
        wait()
        expect(alerts.isEmpty, "성공 경로에서 경고 발생: \(alerts)")
        expect(CoreRuntime.state(handle: host.handle)?.window_session_count == 0, "코어 창 session 누수")
        expect(CoreRuntime.state(handle: host.handle)?.project_count == 0, "코어 프로젝트 session 누수")
        return report()
    }

    func bytes(_ path: String) -> [String: Data] {
        let root = URL(fileURLWithPath: path).appendingPathComponent("design")
        var result: [String: Data] = [:]
        let files = FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil)
        while let file = files?.nextObject() as? URL {
            if let data = try? Data(contentsOf: file) { result[file.path] = data }
        }
        return result
    }

    func report() -> Bool {
        for failure in failures { FileHandle.standardError.write(Data("window-close 실패: \(failure)\n".utf8)) }
        if failures.isEmpty { print("window-close ok: \(checks) checks (정상 닫기·⌘Q → Save Now → 다시 열어 확인)") }
        return failures.isEmpty
    }
}
#endif
