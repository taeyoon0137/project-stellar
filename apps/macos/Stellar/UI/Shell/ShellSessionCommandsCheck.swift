import AppKit

/// 창 session 명령 검사(`--session-commands-check <fixture 폴더>`, XII.6.4 "창별 history와 save 호출").
///
/// 실제 앱 구성과 실제 코어 runtime으로 저장 프로젝트 fixture를 복사한 두 프로젝트 창을 화면 밖에 엽니다. 메뉴 항목으로
/// 보낸 Undo·Redo·Save Now가 그 창(응답자 체인의 router)의 창 session에만 가는지 확인합니다.
/// - 처음: 두 창 모두 Undo·Redo 비활성(history 없음), Save Now 활성(작업 문서 있음)
/// - A 창에서 선택(history 1개) → A만 Undo 활성 → A Undo → A 선택만 되돌아가고 B는 그대로 → A Redo
/// - 비활성 명령(B의 Undo)은 코어로 보내지 않음
/// - A Save Now → 코어 저장 완료(바뀐 문서가 없어 generation 1 그대로, alert 없음)
/// - A 메뉴 Zoom In·Zoom Out·Actual Size(XII.4.3) → A viewport 배율과 툴바 배율 표시만 바뀌고 B는 그대로, 이동·확대 뒤
///   Save Now도 디자인 폴더 bytes를 바꾸지 않음
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

    /// 디자인 폴더 아래 파일의 상대 경로 → bytes.
    private func designBytes(_ path: String) -> [String: Data] {
        let root = URL(fileURLWithPath: path).appendingPathComponent("design")
        var out: [String: Data] = [:]
        let enumerator = FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil)
        while let url = enumerator?.nextObject() as? URL {
            if let data = try? Data(contentsOf: url) {
                out[url.path.replacingOccurrences(of: root.path, with: "")] = data
            }
        }
        return out
    }

    private func checkZoom(_ coordinator: ShellAppCoordinator, a: ShellProjectWindow, b: ShellProjectWindow, pathA: String) {
        let before = designBytes(pathA)
        expect(!before.isEmpty, "A 디자인 폴더를 읽지 못함")
        expect(a.toolsModel.snapshot.zoomScale == 1.0, "처음 배율이 100%가 아님")
        expect(enabled(.zoomIn, in: a) && enabled(.zoomOut, in: a), "active view가 있는데 Zoom In·Out이 비활성")
        expect(!enabled(.zoomToActualSize, in: a), "100%에서 Actual Size가 활성")
        expect(!enabled(.zoomToFit, in: a) && !enabled(.zoomToSelection, in: a), "코어 경로 없는 맞춤 줌이 활성")
        for (command, expected) in [(ShellCommand.zoomIn, 2.0), (.zoomIn, 4.0), (.zoomOut, 2.0), (.zoomToActualSize, 1.0), (.zoomOut, 0.5)] {
            a.router.performShellCommand(MainMenuBuilder.makeItem(for: command, target: nil))
            wait(coordinator, "A \(command.rawValue)")
            expect(a.windowController.windowSession?.viewInfo?.zoom == expected, "\(command.rawValue) 뒤 코어 배율이 \(expected)가 아님")
            expect(a.toolsModel.snapshot.zoomScale == expected, "\(command.rawValue) 뒤 배율 표시가 \(expected)가 아님")
        }
        // 실제 기본 창 CanvasView.scrollWheel 진입점 → 창 session viewport. 합성 AppKit 입력이며 화면 실측은 아닙니다.
        let initialA = a.windowController.windowSession?.viewInfo
        let initialB = b.windowController.windowSession?.viewInfo
        let initialInfo = a.windowController.windowSession?.info
        for (phase, momentum) in [(1, 0), (2, 0), (0, 2)] {
            guard let cg = CGEvent(scrollWheelEvent2Source: nil, units: .pixel,
                                   wheelCount: 2, wheel1: 12, wheel2: -8, wheel3: 0) else {
                expect(false, "팬 CGEvent 생성 실패")
                continue
            }
            cg.flags = []
            cg.setIntegerValueField(.scrollWheelEventScrollPhase, value: Int64(phase))
            cg.setIntegerValueField(.scrollWheelEventMomentumPhase, value: Int64(momentum))
            guard let event = NSEvent(cgEvent: cg), let beforePan = a.windowController.windowSession?.viewInfo else {
                expect(false, "팬 NSEvent 또는 viewport 없음")
                continue
            }
            a.canvasHost.canvasView.scrollWheel(with: event)
            wait(coordinator, "캔버스 스크롤 팬")
            let afterPan = a.windowController.windowSession?.viewInfo
            expect(afterPan?.center_x == beforePan.center_x - Double(event.scrollingDeltaX) / beforePan.zoom,
                   "스크롤 x가 창 camera에 반영되지 않음")
            expect(afterPan?.center_y == beforePan.center_y - Double(event.scrollingDeltaY) / beforePan.zoom,
                   "스크롤 y가 창 camera에 반영되지 않음")
            expect(afterPan?.center_x != beforePan.center_x && afterPan?.center_y != beforePan.center_y,
                   "유효한 스크롤이 camera를 이동하지 않음")
            expect(afterPan?.zoom == beforePan.zoom, "스크롤 팬이 배율을 바꿈")
        }
        if let session = a.windowController.windowSession, let before = session.viewInfo,
           let cg = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 2, wheel1: 2, wheel2: -1, wheel3: 0) {
            cg.flags = []
            if let event = NSEvent(cgEvent: cg) {
                for _ in 0..<32 { a.canvasHost.canvasView.scrollWheel(with: event) }
                wait(coordinator, "연속 정밀 스크롤")
                let after = session.viewInfo
                expect(after?.center_x == before.center_x - 32 * Double(event.scrollingDeltaX) / before.zoom &&
                       after?.center_y == before.center_y - 32 * Double(event.scrollingDeltaY) / before.zoom,
                       "연속 스크롤의 delta 유실")
                session.isClosing = true
                expect(a.canvasHost.canvasView.scrollWheelHandler?(event) == false && session.pendingCommandCount == 0,
                       "닫기 대기 중인 창에 팬 요청 접수")
                session.isClosing = false
            } else { expect(false, "연속 스크롤 NSEvent 생성 실패") }
        } else { expect(false, "연속 스크롤 fixture 생성 실패") }
        // 일반 마우스 휠도 실제 CanvasView 진입점에서 창별 팬으로 연결합니다.
        for zoom in [0.5, 1.0] {
            if zoom == 1.0 {
                a.router.performShellCommand(MainMenuBuilder.makeItem(for: .zoomToActualSize, target: nil))
                wait(coordinator, "휠 검사 100% 배율")
            }
            for (x, y) in [(0, 3), (-2, 0), (1, -3)] {
                guard let cg = CGEvent(scrollWheelEvent2Source: nil, units: .line, wheelCount: 2,
                                       wheel1: Int32(y), wheel2: Int32(x), wheel3: 0) else {
                    expect(false, "일반 휠 CGEvent 생성 실패")
                    continue
                }
                cg.flags = []
                guard let event = NSEvent(cgEvent: cg), let before = a.windowController.windowSession?.viewInfo else {
                    expect(false, "일반 휠 fixture 생성 실패")
                    continue
                }
                expect(!event.hasPreciseScrollingDeltas, "일반 휠 fixture가 정밀 스크롤임")
                expect(before.zoom == zoom, "휠 검사 배율 불일치")
                a.canvasHost.canvasView.scrollWheel(with: event)
                wait(coordinator, "일반 휠 팬")
                let after = a.windowController.windowSession?.viewInfo
                expect(after?.center_x != before.center_x || after?.center_y != before.center_y,
                       "일반 휠이 camera를 이동하지 않음")
                expect(after?.center_x == before.center_x - Double(event.scrollingDeltaX) * 10 / zoom &&
                       after?.center_y == before.center_y - Double(event.scrollingDeltaY) * 10 / zoom,
                       "일반 휠 이동량이 창 camera에 반영되지 않음")
                expect(after?.zoom == before.zoom, "일반 휠이 배율을 바꿈")
            }
        }
        a.router.performShellCommand(MainMenuBuilder.makeItem(for: .zoomOut, target: nil))
        wait(coordinator, "휠 검사 뒤 50% 복귀")
        for (units, flags) in [(CGScrollEventUnit.line, CGEventFlags.maskCommand), (.line, .maskShift),
                               (.line, .maskAlternate), (.line, .maskControl), (.pixel, .maskCommand), (.pixel, .maskShift),
                               (.pixel, .maskAlternate), (.pixel, .maskControl)] {
            guard let cg = CGEvent(scrollWheelEvent2Source: nil, units: units, wheelCount: 2,
                                   wheel1: 12, wheel2: -8, wheel3: 0) else {
                expect(false, "비대상 스크롤 생성 실패")
                continue
            }
            cg.flags = flags
            guard let event = NSEvent(cgEvent: cg) else {
                expect(false, "비대상 NSEvent 생성 실패")
                continue
            }
            expect(a.canvasHost.canvasView.scrollWheelHandler?(event) == false,
                   "수식키 스크롤을 팬이 소비함")
        }
        expect(a.windowController.windowSession?.viewInfo?.center_x != initialA?.center_x, "팬 누적 이동 없음")
        expect(b.windowController.windowSession?.viewInfo?.center_x == initialB?.center_x &&
               b.windowController.windowSession?.viewInfo?.center_y == initialB?.center_y, "팬이 다른 창을 이동함")
        expect(a.windowController.windowSession?.info?.revision == initialInfo?.revision &&
               a.windowController.windowSession?.info?.undo_depth == initialInfo?.undo_depth &&
               a.windowController.windowSession?.info?.has_unsaved_changes == initialInfo?.has_unsaved_changes,
               "팬이 문서 revision/history/dirty를 바꿈")
        expect(enabled(.zoomToActualSize, in: a), "50%에서 Actual Size가 비활성")
        expect(b.windowController.windowSession?.viewInfo?.zoom == 1.0 && b.toolsModel.snapshot.zoomScale == 1.0,
               "A 줌이 B viewport·배율 표시를 바꿈")
        expect(a.windowController.windowSession?.info?.undo_depth == 1, "줌이 history에 기록됨")
        a.router.performShellCommand(MainMenuBuilder.makeItem(for: .saveNow, target: nil))
        wait(coordinator, "줌 뒤 A Save Now")
        if case .windowSessionSaved(_, _, 1, false)? = a.lastSessionCommandEvent {} else {
            expect(false, "줌 뒤 Save Now가 새 generation을 씀: \(String(describing: a.lastSessionCommandEvent))")
        }
        expect(designBytes(pathA) == before, "pan/zoom 뒤 저장이 디자인 폴더 bytes를 바꿈")
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
        expect([ShellCommand.undo, .redo, .saveNow].map { a.coreCommandIDs.coreCommandID(for: $0) }
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

        checkZoom(coordinator, a: a, b: b, pathA: pathA)

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
