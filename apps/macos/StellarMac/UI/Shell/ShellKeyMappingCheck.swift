import AppKit

/// 단축키 대응 검사(`--key-mapping-check <fixture 폴더>`, XII.6.5 "key mapping smoke test").
///
/// command ID는 코어가 정하고 단축키는 셸이 macOS 관례로 둡니다. 실제 앱 구성과 실제 코어 runtime으로 다음을 확인합니다.
/// - 코어 command ID 표: 코어가 공유 이름으로 알려 준 Undo·Redo·Save Now ID가 코어 C ABI 상수와 같고, 셸 전용 명령에는
///   ID가 없으며, 표의 모든 ID가 서로 다르고 역 조회가 됨
/// - 메인 메뉴의 macOS 관례 단축키: Undo ⌘Z, Redo ⇧⌘Z, Save Now ⌘S, Zoom In ⌘+, Actual Size ⌘0, Zoom Out ⌘-(HIG,
///   `ShortcutPolicy` 감사 0건)
/// - 키 조합 → 메뉴 항목 → 셸 명령 → 코어 ID 대응: ⌘Z → UNDO, ⇧⌘Z → REDO, ⌘S → SAVE_NOW, 대문자·Caps Lock 무관
/// - 저장 프로젝트 fixture 두 창에서 메뉴 항목 action을 캔버스의 응답자 체인으로 보내면 그 창 session에 해당 코어
///   명령이 도착함(완료 event 종류로 확인). --native-menus 추가 시 실제 활성 앱의 nil target 메뉴 전달을 검사하며,
///   앱을 활성화할 수 없는 환경에서는 실패합니다.
enum ShellKeyMappingCheck {
    static func run(arguments: [String]) -> Bool {
        guard let flag = arguments.firstIndex(of: "--key-mapping-check"), arguments.indices.contains(flag + 1) else {
            FileHandle.standardError.write(Data("key-mapping 실패: fixture 폴더 인자가 없습니다\n".utf8))
            return false
        }
        let fixture = URL(fileURLWithPath: arguments[flag + 1], isDirectory: true)
        return MainActor.assumeIsolated {
            let nativeMenus = arguments.contains("--native-menus")
            if !nativeMenus { return ShellKeyMappingCheckRunner(fixture: fixture, nativeMenus: false).run() }
            // nil target 메뉴 검사는 NSApplication의 실제 event loop와 key-window 관리가 필요합니다.
            let app = NSApplication.shared
            app.setActivationPolicy(.regular)
            var passed = false
            DispatchQueue.main.async {
                passed = ShellKeyMappingCheckRunner(fixture: fixture, nativeMenus: true).run()
                app.stop(nil)
                if let wake = NSEvent.otherEvent(
                    with: .applicationDefined, location: .zero, modifierFlags: [], timestamp: 0,
                    windowNumber: 0, context: nil, subtype: 0, data1: 0, data2: 0
                ) {
                    app.postEvent(wake, atStart: true)
                }
            }
            app.run()
            return passed
        }
    }
}

@MainActor
private final class ShellKeyMappingCheckRunner {
    /// fixture 문서 node 11(`ShellSessionCommandsCheck`와 같은 node).
    static let childNode = UUID(uuidString: "00000000-000b-7000-8000-00000000000b")!

    private let fixture: URL
    private let nativeMenus: Bool
    private var failures: [String] = []
    private var checks = 0

    init(fixture: URL, nativeMenus: Bool) {
        self.fixture = fixture
        self.nativeMenus = nativeMenus
    }

    private func expect(_ condition: Bool, _ message: @autoclosure () -> String) {
        checks += 1
        if !condition { failures.append(message()) }
    }

    /// 확인할 대응: 셸 명령, macOS 단축키(문자·수식키), 코어 C ABI 상수.
    private static let expectations: [(command: ShellCommand, key: String, modifiers: NSEvent.ModifierFlags, code: UInt32)] = [
        (.undo, "z", [.command], UInt32(STELLAR_COMMAND_UNDO)),
        (.redo, "z", [.shift, .command], UInt32(STELLAR_COMMAND_REDO)),
        (.saveNow, "s", [.command], UInt32(STELLAR_COMMAND_SAVE_NOW)),
        // 줌(XII.4.3): 배율이 바뀌어야 다음 명령이 활성이므로 확대 → 100% → 축소 순서로 확인합니다.
        (.zoomIn, "+", [.command], UInt32(STELLAR_COMMAND_ZOOM_IN)),
        (.zoomToActualSize, "0", [.command], UInt32(STELLAR_COMMAND_ZOOM_ACTUAL_SIZE)),
        (.zoomOut, "-", [.command], UInt32(STELLAR_COMMAND_ZOOM_OUT)),
    ]

    func run() -> Bool {
        if !nativeMenus { NSApplication.shared.setActivationPolicy(.accessory) }
        let projects = ShellSavedProjectFixture(prefix: "StellarKeyMappingCheck")
        defer { projects.remove() }
        let host = RuntimeHost()
        guard host.created else {
            FileHandle.standardError.write(Data("key-mapping 실패: runtime create 실패\n".utf8))
            return false
        }
        defer { host.shutdown() }
        let coordinator = ShellAppCoordinator(runtimeHandle: host.handle)
        coordinator.presentsWindows = false
        let autosave = ShellCheckAutosave(prefix: "StellarKeyMappingCheck")
        defer { autosave.removeRecords() }
        coordinator.windowConfiguration = { autosave.configuration }
        coordinator.alertPresenter = { [weak self] title, _, _ in
            self?.failures.append("예상하지 못한 alert: \(title)")
        }
        let previousMenu = NSApp.mainMenu
        let mainMenu = coordinator.installMainMenu(on: NSApp)
        defer { NSApp.mainMenu = previousMenu }

        checkCoreTable(coordinator.coreCommandIDs)
        checkMenuShortcuts(mainMenu.menu, ids: coordinator.coreCommandIDs)
        if let path = projects.copy(fixture, as: "Project"), let otherPath = projects.copy(fixture, as: "Other") {
            autoreleasepool {
                checkLiveDispatch(coordinator, menu: mainMenu.menu, path: path, otherPath: otherPath)
            }
        } else {
            expect(false, "fixture 복사 실패: \(fixture.path)")
        }
        if failures.isEmpty {
            print("key-mapping mode: \(nativeMenus ? "active application menu" : "window responder chain")")
            print("key-mapping ok: \(checks) checks (core-shared command IDs, macOS shortcuts ⌘Z ⇧⌘Z ⌘S ⌘+ ⌘0 ⌘- → core commands)")
            return true
        }
        for failure in failures {
            FileHandle.standardError.write(Data("key-mapping 실패: \(failure)\n".utf8))
        }
        return false
    }

    // MARK: - 코어 command ID 표

    private func checkCoreTable(_ ids: ShellCommandCoreIDs) {
        for expectation in Self.expectations {
            expect(ids.coreCommandID(for: expectation.command) == expectation.code,
                   "\(expectation.command.rawValue) 코어 ID가 코어 상수 \(expectation.code)가 아님: \(String(describing: ids.coreCommandID(for: expectation.command)))")
            expect(ids.command(forCoreID: expectation.code) == expectation.command, "코어 ID \(expectation.code) 역 조회 실패")
            expect(CoreRuntime.commandCode(name: expectation.command.rawValue) == expectation.code,
                   "공유 이름 \(expectation.command.rawValue) 조회가 코어 상수와 다름")
        }
        expect(Set(ids.ids.values).count == ids.ids.count, "코어 ID 중복: \(ids.ids)")
        expect(ids.ids.keys.allSatisfy { $0.owner == .core }, "셸 전용 명령에 코어 ID가 있음")
        for command in ShellCommand.allCases where command.owner == .shell {
            expect(CoreRuntime.commandCode(name: command.rawValue) == nil, "셸 전용 \(command.rawValue)를 코어가 명령으로 앎")
        }
    }

    // MARK: - 메뉴 단축키

    /// 메뉴(하위 포함)에서 보이는 항목 중 키 조합이 맞는 항목. AppKit과 같이 대문자 key equivalent는 Shift를 뜻합니다.
    private func item(matching key: String, modifiers: NSEvent.ModifierFlags, in menu: NSMenu) -> NSMenuItem? {
        let relevant: NSEvent.ModifierFlags = [.command, .shift, .option, .control]
        for entry in menu.items {
            if let submenu = entry.submenu, let found = item(matching: key, modifiers: modifiers, in: submenu) {
                return found
            }
            guard !entry.isHidden || entry.allowsKeyEquivalentWhenHidden, !entry.keyEquivalent.isEmpty else { continue }
            var mask = entry.keyEquivalentModifierMask.intersection(relevant)
            let equivalent = entry.keyEquivalent
            if equivalent != equivalent.lowercased() { mask.insert(.shift) }
            if equivalent.lowercased() == key.lowercased(), mask == modifiers.intersection(relevant) {
                return entry
            }
        }
        return nil
    }

    private func checkMenuShortcuts(_ menu: NSMenu, ids: ShellCommandCoreIDs) {
        expect(ShortcutPolicy.audit().isEmpty, "macOS 단축키 관례 감사 위반: \(ShortcutPolicy.audit())")
        for expectation in Self.expectations {
            expect(expectation.command.shortcut == KeyShortcut(expectation.key, expectation.modifiers.contains(.shift)
                ? [.shift, .command] : [.command]),
                "\(expectation.command.rawValue) 단축키가 macOS 관례와 다름: \(String(describing: expectation.command.shortcut))")
            guard let entry = item(matching: expectation.key, modifiers: expectation.modifiers, in: menu) else {
                expect(false, "\(expectation.key)·\(expectation.modifiers.rawValue) 키 조합의 메뉴 항목 없음")
                continue
            }
            let command = ShellCommand.resolve(from: entry)
            expect(command == expectation.command, "키 조합이 \(expectation.command.rawValue)가 아니라 \(String(describing: command))로 감")
            expect(command.flatMap { ids.coreCommandID(for: $0) } == expectation.code, "키 조합 → 코어 ID 불일치: \(entry.title)")
            // Caps Lock·대문자 입력도 같은 항목입니다(대소문자 무관 비교, Shift는 수식키로만 판단).
            expect(item(matching: expectation.key.uppercased(), modifiers: expectation.modifiers, in: menu) === entry,
                   "대문자 입력이 다른 항목으로 감: \(entry.title)")
        }
        // 코어 명령이 아닌 같은 문자 조합(⌥⌘Z 등)은 코어 명령으로 가지 않습니다.
        if let other = item(matching: "z", modifiers: [.option, .command], in: menu) {
            expect(ShellCommand.resolve(from: other).flatMap { ids.coreCommandID(for: $0) } == nil, "⌥⌘Z가 코어 명령으로 감")
        }
    }

    // MARK: - 창 session 도착

    /// 기본 검사는 캔버스부터 실제 응답자 체인을 걷습니다. --native-menus는 AppKit이 활성 창을 선택합니다.
    private func send(_ entry: NSMenuItem, action: Selector, to project: ShellProjectWindow) -> Bool {
        if nativeMenus { return NSApp.sendAction(action, to: entry.target, from: entry) }
        return project.window?.firstResponder?.tryToPerform(action, with: entry) ?? false
    }

    /// 일반 앱과 같은 활성화 event를 처리한 뒤 실제 key 창의 캔버스에 포커스를 둡니다.
    private func focus(_ project: ShellProjectWindow) {
        project.window?.makeFirstResponder(project.canvasHost.canvasView)
        guard nativeMenus else { return }
        project.window?.makeKeyAndOrderFront(nil)
        NSApp.activate()
        let deadline = Date().addingTimeInterval(3)
        while NSApp.keyWindow !== project.window && Date() < deadline {
            if let event = NSApp.nextEvent(matching: .any, until: Date().addingTimeInterval(0.01), inMode: .default, dequeue: true) {
                NSApp.sendEvent(event)
            }
        }
        project.window?.makeFirstResponder(project.canvasHost.canvasView)
    }

    private func checkLiveDispatch(_ coordinator: ShellAppCoordinator, menu: NSMenu, path: String, otherPath: String) {
        coordinator.openProject(atPath: path, origin: .application)
        expect(coordinator.pump?.waitForPendingRequests(timeout: 10) == true, "프로젝트 열기 완료 event가 오지 않음")
        guard let window = coordinator.projectWindow(forPath: path), window.windowController.windowSession != nil else {
            expect(false, "프로젝트 창·창 session 없음")
            return
        }
        coordinator.openProject(atPath: otherPath, origin: .application)
        expect(coordinator.pump?.waitForPendingRequests(timeout: 10) == true, "둘째 창 열기 완료 event가 오지 않음")
        guard let other = coordinator.projectWindow(forPath: otherPath) else {
            expect(false, "둘째 창 없음")
            window.window?.close()
            return
        }
        other.window?.setFrameOrigin(NSPoint(x: 80, y: 80))
        window.window?.setFrameOrigin(NSPoint(x: 80, y: 80))
        focus(window)
        if nativeMenus && NSApp.keyWindow !== window.window {
            expect(false, "실제 key 창 활성화 실패: 로그인된 macOS 데스크톱에서 --native-menus를 다시 실행해야 함")
            window.window?.close()
            other.window?.close()
            _ = coordinator.pump?.waitForPendingRequests(timeout: 10)
            return
        }
        expect(window.selectNodes([Self.childNode]), "선택 요청 실패")
        expect(coordinator.pump?.waitForPendingRequests(timeout: 10) == true, "선택 완료 event가 오지 않음")
        for expectation in Self.expectations {
            guard let entry = item(matching: expectation.key, modifiers: expectation.modifiers, in: menu),
                let action = entry.action
            else {
                expect(false, "\(expectation.command.rawValue) 메뉴 항목 action 없음")
                continue
            }
            expect(entry.target == nil, "창 메뉴에 고정 target이 지정됨")
            if nativeMenus {
                let target = NSApp.target(forAction: action, to: nil, from: entry)
                expect((target as AnyObject?) === window.router, "활성 창 메뉴 대상이 창 router가 아님")
            }
            expect(window.router.validateMenuItem(entry), "\(expectation.command.rawValue) 메뉴 항목이 비활성")
            expect(send(entry, action: action, to: window), "메뉴 action 전달 실패")
            expect(coordinator.pump?.waitForPendingRequests(timeout: 10) == true, "\(expectation.command.rawValue) 완료 event가 오지 않음")
            let arrived: Bool
            switch (expectation.command, window.lastSessionCommandEvent) {
            case (.undo, .historyReplayed(_, _, false, true, _)?), (.redo, .historyReplayed(_, _, true, true, _)?),
                 (.saveNow, .windowSessionSaved(_, _, 1, false)?),
                 (.zoomIn, .windowViewChanged?), (.zoomToActualSize, .windowViewChanged?), (.zoomOut, .windowViewChanged?):
                arrived = true
            default:
                arrived = false
            }
            expect(arrived, "\(expectation.command.rawValue)가 창 session에 도착하지 않음: \(String(describing: window.lastSessionCommandEvent))")
        }
        expect(other.lastSessionCommandEvent == nil, "첫째 창 메뉴가 둘째 창으로 전달됨")
        focus(other)
        if nativeMenus { expect(NSApp.keyWindow === other.window, "둘째 창이 key 창이 아님") }
        if let entry = item(matching: "s", modifiers: [.command], in: menu), let action = entry.action {
            if nativeMenus {
                expect((NSApp.target(forAction: action, to: nil, from: entry) as AnyObject?) === other.router,
                       "창 전환 뒤 실제 메뉴 대상이 둘째 창 router가 아님")
            }
            expect(send(entry, action: action, to: other), "둘째 창 Save Now 전달 실패")
            expect(coordinator.pump?.waitForPendingRequests(timeout: 10) == true, "둘째 창 저장 완료 event 없음")
            if case .windowSessionSaved(_, let session, 1, false)? = other.lastSessionCommandEvent {
                expect(session == other.windowController.windowSession?.session, "둘째 창 저장의 session 불일치")
            } else {
                expect(false, "둘째 창 Save Now 완료 event 없음")
            }
        } else {
            expect(false, "Save Now 메뉴 없음")
        }
        window.window?.close()
        other.window?.close()
        _ = coordinator.pump?.waitForPendingRequests(timeout: 10)
    }
}
