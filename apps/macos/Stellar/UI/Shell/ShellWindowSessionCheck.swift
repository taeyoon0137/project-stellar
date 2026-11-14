import AppKit

/// 창 session lifecycle 검사(`--window-session-check <fixture 폴더>`, XII.2.2 "창 2개 open/close lifecycle").
///
/// 실제 앱 구성(`ShellAppCoordinator`, 실제 코어 runtime)을 화면에 띄우지 않고 만들어, 저장 프로젝트 fixture
/// (`crates/runtime/tests/fixtures/window-session-project`)를 복사한 두 프로젝트를 차례로 엽니다. 다음을 확인합니다.
/// - 프로젝트 창마다 창 controller가 코어 창 session handle 하나를 소유하고, handle·창 편집 id가 창마다 다름
/// - 창 session 정보: 소속 프로젝트, 작업 문서·generation 1, 빈 history·선택
/// - 한 창을 닫으면 그 창 session만 닫히고(코어 수 2 → 1) 다른 창 session은 그대로
/// - 두 창을 모두 닫으면 코어 창·프로젝트 session이 0, 같은 cycle을 다시 돌려도 같은 결과
/// - 저장한 적 없는 프로젝트는 작업 문서 없이 창 session을 엶
enum ShellWindowSessionCheck {
    static func run(arguments: [String]) -> Bool {
        guard let flag = arguments.firstIndex(of: "--window-session-check"), arguments.indices.contains(flag + 1) else {
            FileHandle.standardError.write(Data("window-session 실패: fixture 폴더 인자가 없습니다\n".utf8))
            return false
        }
        let fixture = URL(fileURLWithPath: arguments[flag + 1], isDirectory: true)
        return MainActor.assumeIsolated {
            ShellWindowSessionCheckRunner(fixture: fixture).run()
        }
    }
}

/// 저장 프로젝트 fixture를 임시 폴더로 복사합니다(`.git` 폴더를 더해 Stellar 프로젝트로 엶).
@MainActor
struct ShellSavedProjectFixture {
    let root: URL

    init(prefix: String) {
        root = URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
            .appendingPathComponent("\(prefix)-\(UUID().uuidString)", isDirectory: true)
    }

    /// fixture를 `name` 폴더로 복사하고 경로를 돌려줍니다. 실패하면 nil입니다.
    func copy(_ fixture: URL, as name: String) -> String? {
        let target = root.appendingPathComponent(name, isDirectory: true)
        do {
            for marker in ["stellar.yaml", "design/current"] {
                guard try fixture.appendingPathComponent(marker)
                    .resourceValues(forKeys: [.isRegularFileKey]).isRegularFile == true else { return nil }
            }
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            try FileManager.default.copyItem(at: fixture, to: target)
            try FileManager.default.createDirectory(
                at: target.appendingPathComponent(".git", isDirectory: true), withIntermediateDirectories: true
            )
        } catch {
            FileHandle.standardError.write(Data("저장된 프로젝트 fixture 복사 실패: \(fixture.path): \(error.localizedDescription)\n".utf8))
            return nil
        }
        return target.path
    }

    /// 저장한 generation 없이 `stellar.yaml`만 있는 프로젝트 폴더.
    func unsavedProject(_ name: String) -> String? {
        let target = root.appendingPathComponent(name, isDirectory: true)
        do {
            try FileManager.default.createDirectory(
                at: target.appendingPathComponent(".git", isDirectory: true), withIntermediateDirectories: true
            )
            try ShellIntegrationCheckManifest.valid(name: name)
                .write(to: target.appendingPathComponent("stellar.yaml"), atomically: true, encoding: .utf8)
        } catch {
            return nil
        }
        return target.path
    }

    func remove() {
        try? FileManager.default.removeItem(at: root)
    }
}

/// 검사 fixture 매니페스트.
enum ShellIntegrationCheckManifest {
    static func valid(name: String) -> String {
        "schema_version: \"1\"\nproject:\n  id: \"0195c530-3acd-7bc1-8414-c213f61cb965\"\n  name: \(name)\n"
            + "target_profiles: []\ndependencies: {}\n"
    }
}

@MainActor
private final class ShellWindowSessionCheckRunner {
    private let fixture: URL
    private var failures: [String] = []
    private var checks = 0

    init(fixture: URL) {
        self.fixture = fixture
    }

    private func expect(_ condition: Bool, _ message: @autoclosure () -> String) {
        checks += 1
        if !condition { failures.append(message()) }
    }

    func run() -> Bool {
        NSApplication.shared.setActivationPolicy(.accessory)
        let projects = ShellSavedProjectFixture(prefix: "StellarWindowSessionCheck")
        defer { projects.remove() }
        let host = RuntimeHost()
        guard host.created else {
            FileHandle.standardError.write(Data("window-session 실패: runtime create 실패\n".utf8))
            return false
        }
        defer { host.shutdown() }

        let coordinator = ShellAppCoordinator(runtimeHandle: host.handle)
        coordinator.presentsWindows = false
        let autosave = ShellCheckAutosave(prefix: "StellarWindowSessionCheck")
        defer { autosave.removeRecords() }
        coordinator.windowConfiguration = { autosave.configuration }
        coordinator.alertPresenter = { [weak self] title, _, _ in
            self?.failures.append("예상하지 못한 alert: \(title)")
        }

        guard let first = projects.copy(fixture, as: "First"), let second = projects.copy(fixture, as: "Second") else {
            expect(false, "fixture 복사 실패: \(fixture.path)")
            return report()
        }
        for cycle in 1...2 {
            autoreleasepool {
                checkTwoWindowLifecycle(coordinator, host: host, paths: (first, second), cycle: cycle)
            }
        }
        if let unsaved = projects.unsavedProject("Unsaved") {
            checkUnsavedProject(coordinator, host: host, path: unsaved)
        } else {
            expect(false, "빈 프로젝트 fixture를 만들지 못함")
        }
        return report()
    }

    private func report() -> Bool {
        if failures.isEmpty {
            print("window-session ok: \(checks) checks (2 windows × 2 cycles open/close, one session handle per window controller)")
            return true
        }
        for failure in failures {
            FileHandle.standardError.write(Data("window-session 실패: \(failure)\n".utf8))
        }
        return false
    }

    private func counts(_ host: RuntimeHost) -> (projects: UInt64, windows: UInt64) {
        let state = CoreRuntime.state(handle: host.handle)
        return (state?.project_count ?? .max, state?.window_session_count ?? .max)
    }

    private func open(_ coordinator: ShellAppCoordinator, path: String) -> ShellProjectWindow? {
        coordinator.openProject(atPath: path, origin: .application)
        expect(coordinator.pump?.waitForPendingRequests(timeout: 10) == true, "\(path) 열기 완료 event가 오지 않음")
        return coordinator.projectWindow(forPath: path)
    }

    private func checkTwoWindowLifecycle(
        _ coordinator: ShellAppCoordinator, host: RuntimeHost, paths: (String, String), cycle: Int
    ) {
        guard let pump = coordinator.pump else {
            expect(false, "runtime event 펌프 없음")
            return
        }
        guard let windowA = open(coordinator, path: paths.0) else {
            expect(false, "[\(cycle)] 첫 프로젝트 창 없음")
            return
        }
        expect(counts(host) == (1, 1), "[\(cycle)] 창 1개일 때 코어 session 수 불일치: \(counts(host))")
        guard let windowB = open(coordinator, path: paths.1) else {
            expect(false, "[\(cycle)] 둘째 프로젝트 창 없음")
            return
        }
        expect(coordinator.projectWindows.count == 2, "[\(cycle)] 프로젝트 창 수가 2가 아님")
        expect(counts(host) == (2, 2), "[\(cycle)] 창 2개일 때 코어 session 수 불일치: \(counts(host))")

        guard let sessionA = windowA.windowController.windowSession, let sessionB = windowB.windowController.windowSession
        else {
            expect(false, "[\(cycle)] 창 controller가 창 session handle을 소유하지 않음")
            return
        }
        expect(sessionA.session != sessionB.session, "[\(cycle)] 두 창의 창 session handle이 같음")
        expect(sessionA.windowID != sessionB.windowID, "[\(cycle)] 두 창의 창 편집 id가 같음")
        for (window, session) in [(windowA, sessionA), (windowB, sessionB)] {
            expect(window.dataSource.currentSnapshot.status == .ready, "[\(cycle)] 창 session을 연 창이 ready가 아님")
            expect(session.project == window.dataSource.session?.project, "[\(cycle)] 창 session의 소속 프로젝트가 창의 프로젝트가 아님")
            expect(session.documentID != nil && session.openedGeneration == 1, "[\(cycle)] fixture 작업 문서·generation 1을 열지 않음")
            guard let info = session.info else {
                expect(false, "[\(cycle)] 창 session 정보 없음")
                continue
            }
            expect(info.has_document == 1 && info.generation == 1 && info.revision == 0, "[\(cycle)] 창 session 정보 불일치")
            expect(info.undo_depth == 0 && info.redo_depth == 0 && info.selection_count == 0, "[\(cycle)] 새 창에 history·선택이 있음")
            expect(info.project == session.project && info.session == session.session, "[\(cycle)] 정보의 handle 불일치")
        }

        // 첫 창 닫기: 그 창 session만 닫힙니다.
        windowA.window?.close()
        expect(sessionA.isClosed, "[\(cycle)] 창을 닫았는데 창 session handle을 닫지 않음")
        expect(windowA.dataSource.session?.isClosed == true, "[\(cycle)] 창을 닫았는데 프로젝트 session을 닫지 않음")
        expect(pump.waitForPendingRequests(timeout: 10), "[\(cycle)] 창 닫기 완료 event가 오지 않음")
        expect(counts(host) == (1, 1), "[\(cycle)] 한 창을 닫은 뒤 코어 session 수 불일치: \(counts(host))")
        expect(sessionA.info == nil, "[\(cycle)] 닫은 창 session 정보가 남음")
        expect(CoreRuntime.windowSessionInfo(handle: host.handle, session: sessionA.session) == nil,
               "[\(cycle)] 코어에 닫은 창 session이 남음")
        expect(sessionB.info?.session == sessionB.session, "[\(cycle)] 다른 창 session이 같이 닫힘")
        expect(coordinator.projectWindows.count == 1, "[\(cycle)] 닫은 창이 목록에 남음")

        // 둘째 창 닫기: 코어 session 0.
        windowB.window?.close()
        expect(sessionB.isClosed, "[\(cycle)] 둘째 창 session handle을 닫지 않음")
        expect(pump.waitForPendingRequests(timeout: 10), "[\(cycle)] 둘째 창 닫기 완료 event가 오지 않음")
        expect(counts(host) == (0, 0), "[\(cycle)] 두 창을 닫은 뒤 코어 session이 남음: \(counts(host))")
        expect(coordinator.projectWindows.isEmpty, "[\(cycle)] 창 목록이 비지 않음")
        expect(pump.unclaimedEventCount == 0, "[\(cycle)] 받을 곳 없는 완료 event가 있음: \(pump.unclaimedEventCount)")
    }

    private func checkUnsavedProject(_ coordinator: ShellAppCoordinator, host: RuntimeHost, path: String) {
        guard let window = open(coordinator, path: path), let session = window.windowController.windowSession else {
            expect(false, "빈 프로젝트 창·창 session 없음")
            return
        }
        expect(session.documentID == nil && session.openedGeneration == 0, "빈 프로젝트에 작업 문서·generation이 있음")
        expect(session.info?.has_document == 0, "빈 프로젝트 창 session 정보가 문서 있음으로 나옴")
        window.window?.close()
        _ = coordinator.pump?.waitForPendingRequests(timeout: 10)
        expect(counts(host) == (0, 0), "빈 프로젝트 창을 닫은 뒤 코어 session이 남음: \(counts(host))")
    }
}
