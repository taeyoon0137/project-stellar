import AppKit

#if DEBUG
/// 실제 AppKit close delegate → Save Now C ABI → Rust 저장/재로드 검사. 모든 파일은 임시 fixture입니다.
enum ShellWindowCloseCheck {
    static func run(arguments: [String]) -> Bool {
        guard let flag = arguments.firstIndex(of: "--window-close-check"), arguments.indices.contains(flag + 1) else {
            return false
        }
        return MainActor.assumeIsolated {
            let fixture = URL(fileURLWithPath: arguments[flag + 1])
            let native = arguments.contains("--native-close")
            let app = NSApplication.shared
            app.setActivationPolicy(native ? .regular : .accessory)
            if !native { return ShellWindowCloseCheckRunner(fixture: fixture, native: false).run() }
            var result = false
            DispatchQueue.main.async {
                result = ShellWindowCloseCheckRunner(fixture: fixture, native: true).run()
                app.stop(nil)
                if let wake = NSEvent.otherEvent(
                    with: .applicationDefined, location: .zero, modifierFlags: [], timestamp: 0,
                    windowNumber: 0, context: nil, subtype: 0, data1: 0, data2: 0
                ) { app.postEvent(wake, atStart: true) }
            }
            app.run()
            return result
        }
    }
}

@MainActor
private final class ShellWindowCloseCheckRunner {
    let fixture: URL
    let native: Bool
    var failures: [String] = []
    var checks = 0
    var alerts: [String] = []
    let node = UUID(uuidString: "00000000-000b-7000-8000-00000000000b")!
    init(fixture: URL, native: Bool) { self.fixture = fixture; self.native = native }

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
        let standard = ShellWindowConfiguration.standard
        let keys = ["NSWindow Frame \(standard.autosaveName)",
                    "NSSplitView Subview Frames \(standard.mainSplitAutosaveName)",
                    "NSSplitView Subview Frames \(standard.centerSplitAutosaveName)",
                    "NSToolbar Configuration \(standard.toolbarIdentifier)"]
        let before = keys.map { UserDefaults.standard.object(forKey: $0) as? NSObject }
        let previousMenu = NSApp.mainMenu
        _ = coordinator.installMainMenu(on: NSApp)
        defer { NSApp.mainMenu = previousMenu }
        guard let path = projects.copy(fixture, as: "Project"), let pump = coordinator.pump else { return false }
        defer {
            for window in coordinator.projectWindows { window.window?.close() }
            _ = pump.waitForPendingRequests(timeout: 5)
        }
        func wait() { expect(pump.waitForPendingRequests(timeout: 5), "코어 완료 대기 실패") }
        func open() -> ShellProjectWindow? {
            coordinator.openProject(atPath: path, origin: .application)
            wait()
            guard let window = coordinator.projectWindows.first else { expect(false, "창 열기 실패"); return nil }
            if native {
                window.windowController.showWindow(nil)
                NSApp.activate()
                window.window?.makeKeyAndOrderFront(nil)
                let deadline = Date().addingTimeInterval(2)
                while Date() < deadline, !NSApp.isActive || NSApp.keyWindow !== window.window {
                    RunLoop.current.run(until: Date().addingTimeInterval(0.02))
                }
                expect(NSApp.isActive && NSApp.keyWindow === window.window, "실제 key 창 활성화 실패")
            }
            return window
        }
        func close(_ window: ShellProjectWindow) {
            if native { window.window?.standardWindowButton(.closeButton)?.performClick(nil) }
            else { window.window?.performClose(nil) }
        }
        func retained(_ window: ShellProjectWindow, _ message: String) {
            expect(coordinator.projectWindows.contains(where: { $0 === window }), message + ": 창 목록")
            expect(window.windowController.windowSession?.isClosed == false, message + ": 메모리 session")
            if native { expect(window.window?.isVisible == true, message + ": 실제 창 표시") }
        }
        func edit(_ session: ShellWindowSessionHandle, _ name: String, verify: Bool = false, delay: UInt32 = 0) {
            expect(session.track { done in
                pump.checkWindowEdit(session: session.session, node: node, name: name, verify: verify, delayMS: delay, completion: done)
            } completion: { event in
                if case .requestFailed = event { self.expect(false, "Rust 편집/이름 검증 실패: \(name)") }
            }, "검사 편집 접수 실패")
        }
        guard let clean = open(), let cleanSession = clean.windowController.windowSession else { return false }
        if native && !failures.isEmpty { return report() }
        // 선택만 바뀐 창도 저장 요청 없이 즉시 닫혀야 합니다.
        let cleanBytes = bytes(path)
        expect(clean.selectNodes([node]), "선택 접수 실패")
        wait()
        expect(cleanSession.info?.has_unsaved_changes == 0, "선택이 dirty를 만듦")
        close(clean)
        expect(coordinator.projectWindows.isEmpty && cleanSession.isClosed, "clean 즉시 닫기 실패")
        wait()
        expect(bytes(path) == cleanBytes && alerts.isEmpty, "clean 닫기에 쓰기/경고 발생")

        guard let dirty = open(), let dirtySession = dirty.windowController.windowSession else { return false }
        // 접수됐지만 snapshot에 아직 없는 편집도 닫기보다 먼저 반영합니다.
        edit(dirtySession, "닫기 저장", delay: 50)
        close(dirty)
        retained(dirty, "편집 완료 전 닫힘")
        expect(dirty.isWaitingToClose, "비동기 닫기 대기 없음")
        close(dirty)
        expect(!dirty.performSessionCommand(.undo), "닫기 대기 중 새 Undo 접수")
        wait()
        expect(coordinator.projectWindows.isEmpty && dirtySession.isClosed, "저장 후 닫기 실패")
        expect(bytes(path) != cleanBytes && alerts.isEmpty, "dirty 저장 bytes/경고 오류")
        guard let saved = open(), let savedSession = saved.windowController.windowSession else { return false }
        edit(savedSession, "닫기 저장", verify: true)
        wait()

        // 쓰기 오류: design/objects를 파일로 막아 실제 Rust writer가 실패하게 합니다.
        edit(savedSession, "실패 후 재시도")
        wait()
        let chunks = URL(fileURLWithPath: path).appendingPathComponent("design/objects")
        let backup = URL(fileURLWithPath: path).appendingPathComponent("objects-backup")
        do {
            try FileManager.default.moveItem(at: chunks, to: backup)
            try Data("blocked".utf8).write(to: chunks)
        } catch { expect(false, "쓰기 실패 fixture 구성 오류: \(error)") }
        let failureBytes = bytes(path)
        expect(saved.performSessionCommand(.saveNow), "닫기 직전 Save Now 접수 실패")
        close(saved)
        wait()
        retained(saved, "쓰기 실패")
        expect(savedSession.info?.has_unsaved_changes == 1 && bytes(path) == failureBytes, "쓰기 실패 시 dirty/디스크 손실")
        expect(alerts.count == 1, "쓰기 실패 안내 횟수")
        edit(savedSession, "실패 후 재시도", verify: true)
        wait()
        try? FileManager.default.removeItem(at: chunks)
        try? FileManager.default.moveItem(at: backup, to: chunks)
        close(saved)
        wait()
        expect(coordinator.projectWindows.isEmpty, "쓰기 오류 복구 후 재시도 실패")

        guard let stale = open(), let staleSession = stale.windowController.windowSession else { return false }
        edit(staleSession, "원래 창 편집")
        wait()
        // 같은 프로젝트의 독립 sibling session이 먼저 저장한 디스크를 원래 창이 덮어쓰지 않아야 합니다.
        var sibling: ShellWindowSessionHandle?
        pump.openWindowSession(project: staleSession.project) { event in
            if case .windowSessionOpened(_, let project, let session, let windowID, let documentID, let generation) = event {
                sibling = ShellWindowSessionHandle(session: session, project: project, windowID: windowID,
                                                   documentID: documentID, openedGeneration: generation, pump: pump)
            }
        }
        wait()
        if let sibling {
            edit(sibling, "다른 창 저장")
            wait()
            expect(sibling.perform(coreCommand: coordinator.coreCommandIDs.coreCommandID(for: .saveNow)!) { event in
                if case .requestFailed = event { self.expect(false, "sibling 저장 실패") }
            }, "sibling 저장 접수 실패")
            wait()
            let siblingBytes = bytes(path)
            close(stale)
            wait()
            retained(stale, "stale 실패")
            expect(staleSession.info?.has_unsaved_changes == 1 && bytes(path) == siblingBytes, "다른 창 디스크 덮어씀")
            edit(staleSession, "원래 창 편집", verify: true)
            edit(sibling, "다른 창 저장", verify: true)
            wait()
            expect(alerts.count == 2, "stale 안내 횟수")
            sibling.close()
            wait()
        } else { expect(false, "sibling 열기 실패") }
        // 명시적 검사 정리(close는 AppKit delegate를 우회하는 저수준 API).
        stale.window?.close()
        wait()

        guard let timeout = open(), let timeoutSession = timeout.windowController.windowSession else { return false }
        timeout.closeReplyTimeout = 0.02
        edit(timeoutSession, "timeout 편집", delay: 500)
        close(timeout)
        let timeoutDeadline = Date().addingTimeInterval(0.3)
        while timeout.isWaitingToClose, Date() < timeoutDeadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.005))
        }
        retained(timeout, "timeout")
        expect(!timeout.isWaitingToClose && alerts.count == 3, "timeout 결정 상태 없음")
        wait()
        retained(timeout, "timeout 뒤 late callback")
        edit(timeoutSession, "timeout 편집", verify: true)
        wait()
        timeout.closeReplyTimeout = 10
        close(timeout)
        wait()
        expect(coordinator.projectWindows.isEmpty, "timeout 뒤 재시도 실패")

        guard let lateSave = open(), let lateSession = lateSave.windowController.windowSession else { return false }
        edit(lateSession, "늦은 저장 응답")
        wait()
        // 다른 요청으로 document thread가 바쁜 동안 실제 Save Now를 큐에 접수합니다.
        _ = pump.checkWindowEdit(session: lateSession.session, node: node, name: "늦은 저장 응답", verify: true, delayMS: 500) { _ in }
        lateSave.closeReplyTimeout = 0.02
        close(lateSave)
        let saveDeadline = Date().addingTimeInterval(0.3)
        while lateSave.isWaitingToClose, Date() < saveDeadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.005))
        }
        retained(lateSave, "저장 응답 timeout")
        expect(!lateSave.isWaitingToClose && alerts.count == 4, "저장 응답 timeout 안내 없음")
        wait()
        retained(lateSave, "timeout 뒤 실제 저장 성공")
        expect(lateSession.info?.has_unsaved_changes == 0, "늦은 저장이 실제 Rust에서 완료되지 않음")
        close(lateSave)
        expect(coordinator.projectWindows.isEmpty, "늦은 저장 완료 뒤 명시적 닫기 실패")
        wait()

        guard let cancelled = open(), let cancelledSession = cancelled.windowController.windowSession else { return false }
        edit(cancelledSession, "취소 후 저장")
        wait()
        close(cancelled)
        cancelled.cancelPendingClose()
        wait()
        retained(cancelled, "취소 뒤 실제 저장 응답")
        expect(cancelledSession.info?.has_unsaved_changes == 0, "취소 후 실제 저장 완료 확인 실패")
        // 닫힌 창을 계속 참조해도 late callback이 세션을 붙이거나 다시 열지 않습니다.
        edit(cancelledSession, "닫힌 창 응답", delay: 50)
        close(cancelled)
        cancelled.window?.close()
        wait()
        expect(coordinator.projectWindows.isEmpty && cancelledSession.isClosed, "닫힌 창에 late callback 재부착")
        expect(alerts.count == 4, "늦은 응답의 추가 경고")
        expect(CoreRuntime.state(handle: host.handle)?.window_session_count == 0, "코어 창 session 누수")
        expect(CoreRuntime.state(handle: host.handle)?.project_count == 0, "코어 프로젝트 session 누수")
        RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        expect(keys.map { UserDefaults.standard.object(forKey: $0) as? NSObject } == before, "앱 user preference 변경")
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
        if failures.isEmpty { print("window-close ok: \(checks) checks (AppKit close → Rust save/reload, native=\(native))") }
        return failures.isEmpty
    }
}
#endif
