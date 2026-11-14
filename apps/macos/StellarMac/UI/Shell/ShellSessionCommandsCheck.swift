import AppKit

/// 창 session 명령 검사(`--session-commands-check <fixture 폴더>`, XII.6.4 "창별 history와 save 호출").
///
/// 실제 앱 구성과 실제 코어 runtime으로 저장 프로젝트 fixture를 복사한 두 프로젝트 창을 화면 밖에 엽니다. 메뉴 항목으로
/// 보낸 Undo·Redo·Save Now가 그 창(응답자 체인의 router)의 창 session에만 가는지 확인합니다.
/// - 처음: 두 창 모두 Undo·Redo 비활성(history 없음), Save Now 활성(작업 문서 있음)
/// - A 창에서 선택(history 1개) → A만 Undo 활성 → A Undo → A 선택만 되돌아가고 B는 그대로 → A Redo
/// - 비활성 명령(B의 Undo)은 코어로 보내지 않음
/// - A Save Now → 코어 저장 완료(바뀐 문서가 없어 generation 1 그대로, alert 없음)
/// - B 폴더의 current pointer를 지운 뒤 B Save Now → 코어가 Stale로 거부, 저장 실패 alert 한 번, 디스크 그대로
enum ShellSessionCommandsCheck {
    static func run(arguments: [String]) -> Bool {
        guard let flag = arguments.firstIndex(of: "--session-commands-check"), arguments.indices.contains(flag + 1) else {
            FileHandle.standardError.write(Data("session-commands 실패: fixture 폴더 인자가 없습니다\n".utf8))
            return false
        }
        let fixture = URL(fileURLWithPath: arguments[flag + 1], isDirectory: true)
        return MainActor.assumeIsolated {
            ShellSessionCommandsCheckRunner(fixture: fixture).run()
        }
    }
}

@MainActor
private final class ShellSessionCommandsCheckRunner {
    /// fixture 문서의 첫 View root(10) 아래 node 11(`StableId::new_v7(11, 0, 11)`).
    static let childNode = UUID(uuidString: "00000000-000b-7000-8000-00000000000b")!

    private let fixture: URL
    private var failures: [String] = []
    private var checks = 0
    private var alerts: [String] = []

    init(fixture: URL) {
        self.fixture = fixture
    }

    private func expect(_ condition: Bool, _ message: @autoclosure () -> String) {
        checks += 1
        if !condition { failures.append(message()) }
    }

    func run() -> Bool {
        NSApplication.shared.setActivationPolicy(.accessory)
        let projects = ShellSavedProjectFixture(prefix: "StellarSessionCommandsCheck")
        defer { projects.remove() }
        let host = RuntimeHost()
        guard host.created else {
            FileHandle.standardError.write(Data("session-commands 실패: runtime create 실패\n".utf8))
            return false
        }
        defer { host.shutdown() }
        let coordinator = ShellAppCoordinator(runtimeHandle: host.handle)
        coordinator.presentsWindows = false
        let autosave = ShellCheckAutosave(prefix: "StellarSessionCommandsCheck")
        defer { autosave.removeRecords() }
        coordinator.windowConfiguration = { autosave.configuration }
        coordinator.alertPresenter = { [weak self] title, _, _ in
            self?.alerts.append(title)
        }
        guard let pathA = projects.copy(fixture, as: "A"), let pathB = projects.copy(fixture, as: "B") else {
            expect(false, "fixture 복사 실패: \(fixture.path)")
            return report()
        }
        autoreleasepool {
            check(coordinator, host: host, pathA: pathA, pathB: pathB)
        }
        return report()
    }

    private func report() -> Bool {
        if failures.isEmpty {
            print("session-commands ok: \(checks) checks (undo/redo/save now per window session via menu items)")
            return true
        }
        for failure in failures {
            FileHandle.standardError.write(Data("session-commands 실패: \(failure)\n".utf8))
        }
        return false
    }

    private func wait(_ coordinator: ShellAppCoordinator, _ label: String) {
        expect(coordinator.pump?.waitForPendingRequests(timeout: 10) == true, "\(label) 완료 event가 오지 않음")
    }

    private func enabled(_ command: ShellCommand, in window: ShellProjectWindow) -> Bool {
        window.router.validateMenuItem(MainMenuBuilder.makeItem(for: command, target: nil))
    }

    private func check(_ coordinator: ShellAppCoordinator, host: RuntimeHost, pathA: String, pathB: String) {
        coordinator.openProject(atPath: pathA, origin: .application)
        wait(coordinator, "A 열기")
        coordinator.openProject(atPath: pathB, origin: .application)
        wait(coordinator, "B 열기")
        guard let a = coordinator.projectWindow(forPath: pathA), let b = coordinator.projectWindow(forPath: pathB),
            let sessionA = a.windowController.windowSession, let sessionB = b.windowController.windowSession
        else {
            expect(false, "프로젝트 창·창 session 없음")
            return
        }
        expect(ShellWindowSessionCommand.allCases.map(\.coreCode)
            == [UInt32(STELLAR_COMMAND_UNDO), UInt32(STELLAR_COMMAND_REDO), UInt32(STELLAR_COMMAND_SAVE_NOW)],
            "창 session 명령 코드가 코어 상수와 다름")
        for window in [a, b] {
            expect(!enabled(.undo, in: window) && !enabled(.redo, in: window), "history 없는 창에서 Undo·Redo가 활성")
            expect(enabled(.saveNow, in: window), "작업 문서가 있는 창에서 Save Now가 비활성")
        }

        // A에서 선택 → A만 Undo 활성.
        expect(a.selectNodes([Self.childNode]), "A 선택 요청 실패")
        wait(coordinator, "A 선택")
        expect(sessionA.info?.selection_count == 1 && sessionA.info?.undo_depth == 1, "A 선택이 history에 기록되지 않음")
        expect(enabled(.undo, in: a), "선택 뒤 A Undo가 비활성")
        expect(!enabled(.undo, in: b), "A 선택 뒤 B Undo가 활성")

        // 메뉴 Undo(응답자 체인으로 A router) → A 선택만 되돌아감.
        a.router.undo(MainMenuBuilder.makeItem(for: .undo, target: nil))
        wait(coordinator, "A Undo")
        if case .historyReplayed(_, let session, let redo, let applied, _)? = a.lastSessionCommandEvent {
            expect(session == sessionA.session && !redo && applied, "A Undo 완료 event 불일치")
        } else {
            expect(false, "A Undo 완료 event 없음: \(String(describing: a.lastSessionCommandEvent))")
        }
        expect(sessionA.info?.selection_count == 0 && sessionA.info?.redo_depth == 1, "A Undo가 선택을 되돌리지 않음")
        expect(sessionB.info?.undo_depth == 0 && sessionB.info?.redo_depth == 0, "A Undo가 B history를 바꿈")
        expect(!enabled(.undo, in: a) && enabled(.redo, in: a), "A Undo 뒤 Undo·Redo 활성 상태 불일치")

        // 비활성 명령은 코어로 보내지 않습니다.
        let pending = coordinator.pump?.pendingRequestCount ?? -1
        expect(!b.router.dispatch(.undo), "비활성 B Undo를 보냄")
        expect(coordinator.pump?.pendingRequestCount == pending, "비활성 명령이 코어 요청을 만듦")

        // 메뉴 Redo.
        a.router.redo(MainMenuBuilder.makeItem(for: .redo, target: nil))
        wait(coordinator, "A Redo")
        expect(sessionA.info?.selection_count == 1 && sessionA.info?.redo_depth == 0, "A Redo가 선택을 다시 적용하지 않음")

        // 메뉴 Save Now(A): 바뀐 문서가 없어 generation 1 그대로.
        a.router.performShellCommand(MainMenuBuilder.makeItem(for: .saveNow, target: nil))
        wait(coordinator, "A Save Now")
        if case .windowSessionSaved(_, let session, let generation, let written)? = a.lastSessionCommandEvent {
            expect(session == sessionA.session && generation == 1 && !written, "A Save Now 결과 불일치")
        } else {
            expect(false, "A Save Now 완료 event 없음: \(String(describing: a.lastSessionCommandEvent))")
        }
        expect(alerts.isEmpty, "성공한 Save Now가 alert를 띄움: \(alerts)")

        // B: 디스크 current pointer가 사라지면(다른 쓰기) 코어가 Stale로 거부하고 alert 한 번.
        let current = URL(fileURLWithPath: pathB).appendingPathComponent("design/current")
        try? FileManager.default.removeItem(at: current)
        b.router.performShellCommand(MainMenuBuilder.makeItem(for: .saveNow, target: nil))
        wait(coordinator, "B Save Now")
        if case .requestFailed(_, let failure, _)? = b.lastSessionCommandEvent {
            expect(Int32(bitPattern: failure) == STELLAR_FAILURE_SAVE_STALE, "B Save Now 실패 코드가 SAVE_STALE이 아님: \(failure)")
        } else {
            expect(false, "B Save Now가 실패하지 않음: \(String(describing: b.lastSessionCommandEvent))")
        }
        expect(alerts == ["The Design Couldn’t Be Saved"], "Stale 저장 alert 불일치: \(alerts)")
        expect(!FileManager.default.fileExists(atPath: current.path), "Stale 저장이 디스크에 썼음")
        expect(sessionB.info?.generation == 1, "Stale 저장 뒤 B generation이 바뀜")

        a.window?.close()
        b.window?.close()
        wait(coordinator, "창 닫기")
        let state = CoreRuntime.state(handle: host.handle)
        expect(state?.window_session_count == 0 && state?.project_count == 0, "창을 닫은 뒤 코어 session이 남음")
    }
}
