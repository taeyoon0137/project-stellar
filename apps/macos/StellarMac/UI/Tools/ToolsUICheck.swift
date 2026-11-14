import AppKit

/// 도구·메뉴 영역 정적 검사(smoke 연결용, 통합 시 scripts/macos/smoke.sh에 연결).
///
/// Stub 데이터로 메인 메뉴·라우터·툴바를 화면 밖에서 만들고 다음을 확인합니다.
/// - 명령 메타데이터: 제목·말줄임표 규칙, SF Symbol 존재, 코어 ID Stub 대응
/// - 단축키: HIG 표준·시스템 단축키 충돌 감사(+ 감사기가 위반을 잡는지 음성 검사), 단일 키 도구 해석
/// - 메뉴: 최상위 순서, 모든 항목의 `ShellCommand` 대응·action·target 규칙, 아이콘 그룹 균일
/// - 라우팅: 응답자 체인, 메뉴 검증(제목·체크·활성), 텍스트 편집 중 억제, 빈·로딩 상태, 오래된 snapshot 폐기
/// - 툴바: 도구 그룹·줌 항목 접근성 레이블, snapshot 반영
/// - 레이아웃: Light·Dark·고대비 외형과 좁은 폭에서 모호한 Auto Layout 없음
/// - 성능: snapshot 반영·메뉴 검증의 메인 스레드 시간(아키텍처 §6.3 2ms)
enum ToolsUICheck {
    /// 검사 실패 수집기.
    private final class Recorder {
        private(set) var failures: [String] = []
        func expect(_ condition: Bool, _ message: @autoclosure () -> String) {
            if !condition { failures.append(message()) }
        }
    }

    /// 캔버스 자리에 두는 포커스 가능한 뷰(검사 전용).
    private final class FocusStandInView: NSView {
        override var acceptsFirstResponder: Bool { true }
    }

    static func run() -> Bool {
        let application = NSApplication.shared
        application.setActivationPolicy(.accessory)

        let recorder = Recorder()
        checkCommands(recorder)
        checkShortcuts(recorder)
        checkToolShortcuts(recorder)

        let appModel = ToolsMenuModelStub()
        let appRouter = ShellCommandRouter(model: appModel)
        let mainMenu = MainMenuBuilder.build(applicationTarget: appRouter)
        let menuItemCount = checkMainMenu(mainMenu, appRouter: appRouter, recorder)
        checkApplicationRouting(mainMenu, appRouter: appRouter, appModel: appModel, recorder)

        let timing = checkWindow(mainMenu, recorder)

        if recorder.failures.isEmpty {
            print(
                "tools-menus ok: \(ShellCommand.allCases.count) commands, \(menuItemCount) menu items, "
                    + "shortcut audit 0 issues, \(CanvasTool.allCases.count) tools, "
                    + "apply median \(timing.applyMedianMicros)µs (max \(timing.applyMaxMicros)µs), "
                    + "View menu validation median \(timing.validateMedianMicros)µs"
            )
            return true
        }
        for failure in recorder.failures {
            FileHandle.standardError.write(Data("tools-menus 실패: \(failure)\n".utf8))
        }
        return false
    }

    // MARK: 1. 명령 메타데이터

    private static func checkCommands(_ recorder: Recorder) {
        var coreIDs: Set<String> = []
        for command in ShellCommand.allCases {
            recorder.expect(!command.title.isEmpty, "\(command.rawValue) 제목 없음")
            recorder.expect(
                command.title.hasSuffix("…") == command.requiresMoreInput,
                "\(command.rawValue) 말줄임표 규칙 위반: '\(command.title)'"
            )
            recorder.expect(
                NSImage(systemSymbolName: command.symbolName, accessibilityDescription: nil) != nil,
                "\(command.rawValue) SF Symbol 없음: \(command.symbolName)"
            )
            recorder.expect(
                ShellCommand(menuItemIdentifier: command.menuItemIdentifier) == command,
                "\(command.rawValue) identifier 왕복 실패"
            )
            let coreID = ShellCommandCoreIDStub.coreCommandID(for: command)
            switch command.owner {
            case .shell:
                recorder.expect(coreID == nil, "셸 전용 \(command.rawValue)에 코어 ID가 있음")
            case .core:
                if let coreID {
                    recorder.expect(coreIDs.insert(coreID).inserted, "코어 ID 중복: \(coreID)")
                    // Stub 제한: 코어 ID 체계가 생기기 전까지 임시 접두사만 씁니다.
                    recorder.expect(
                        coreID.hasPrefix(ShellCommandCoreIDStub.provisionalPrefix),
                        "코어 ID Stub이 임시 접두사를 쓰지 않음: \(coreID)"
                    )
                } else {
                    recorder.expect(false, "코어 명령 \(command.rawValue)에 코어 ID 없음")
                }
            }
            if command.isSuppressedDuringTextEditing {
                recorder.expect(command.standardAction == nil, "\(command.rawValue) 표준 action은 억제 대상이 아님")
            }
        }
        for tool in CanvasTool.allCases {
            recorder.expect(tool.command.canvasTool == tool, "\(tool.rawValue) 도구↔명령 대응 불일치")
            // 도구 선택은 체크 표시가 붙는 모드 전환이므로 말줄임표를 붙이지 않습니다(HIG The menu bar).
            recorder.expect(!tool.command.title.hasSuffix("…"), "\(tool.rawValue) 도구 선택 제목에 말줄임표가 있음")
            recorder.expect(
                NSImage(systemSymbolName: tool.symbolName, accessibilityDescription: tool.label) != nil,
                "\(tool.rawValue) SF Symbol 없음: \(tool.symbolName)"
            )
        }
    }

    // MARK: 2. 단축키 감사

    private static func checkShortcuts(_ recorder: Recorder) {
        let issues = ShortcutPolicy.audit()
        recorder.expect(issues.isEmpty, "단축키 감사: \(issues.joined(separator: "; "))")

        // 감사기가 위반을 실제로 잡는지 음성 검사.
        typealias Binding = ShortcutPolicy.Binding
        let negatives: [(String, [Binding])] = [
            ("표준 재사용(⌘B)", [Binding(command: .duplicate, shortcut: .command("b"), isHiddenAlternate: false)]),
            ("시스템 단축키(⌘Space)", [Binding(command: .duplicate, shortcut: .command(" "), isHiddenAlternate: false)]),
            ("충돌", [
                Binding(command: .duplicate, shortcut: .command("k"), isHiddenAlternate: false),
                Binding(command: .frameSelection, shortcut: .command("k"), isHiddenAlternate: false),
            ]),
            ("Command 없음", [Binding(command: .duplicate, shortcut: KeyShortcut("k", []), isHiddenAlternate: false)]),
            ("Control 사용", [Binding(command: .duplicate, shortcut: KeyShortcut("k", [.control, .command]), isHiddenAlternate: false)]),
            ("파생(⇧⌘B ← ⌘B)", [Binding(command: .duplicate, shortcut: KeyShortcut("b", [.shift, .command]), isHiddenAlternate: false)]),
            ("정규화(⇧⌘= vs ⌘+)", [
                Binding(command: .zoomIn, shortcut: .command("+"), isHiddenAlternate: false),
                Binding(command: .duplicate, shortcut: KeyShortcut("=", [.shift, .command]), isHiddenAlternate: false),
            ]),
        ]
        for (name, bindings) in negatives {
            recorder.expect(!ShortcutPolicy.audit(bindings).isEmpty, "감사기가 위반을 놓침: \(name)")
        }
    }

    // MARK: 3. 단일 키 도구 단축키

    private static func checkToolShortcuts(_ recorder: Recorder) {
        let keys = CanvasTool.allCases.compactMap(\.singleKey)
        recorder.expect(Set(keys).count == keys.count, "도구 단일 키 중복: \(keys)")
        for key in keys {
            recorder.expect(key.isASCII && key.isLetter && key.isLowercase, "도구 단일 키가 ASCII 소문자가 아님: \(key)")
        }

        func resolve(_ characters: String?, _ keyCode: UInt16, _ flags: NSEvent.ModifierFlags = [], repeat isRepeat: Bool = false) -> CanvasTool? {
            CanvasToolShortcut.tool(characters: characters, keyCode: keyCode, modifierFlags: flags, isARepeat: isRepeat)
        }
        for tool in CanvasTool.allCases {
            guard let key = tool.singleKey else { continue }
            recorder.expect(resolve(String(key), 0xFF) == tool, "단일 키 \(key) → \(tool.rawValue) 해석 실패")
        }
        recorder.expect(resolve("R", 0x0F, [.capsLock]) == .rectangle, "Caps Lock 대문자 R 해석 실패")
        recorder.expect(resolve("r", 0x0F, [.command]) == nil, "⌘R이 도구로 해석됨")
        recorder.expect(resolve("r", 0x0F, [.shift]) == nil, "⇧R이 도구로 해석됨")
        recorder.expect(resolve("r", 0x0F, [.option]) == nil, "⌥R이 도구로 해석됨")
        recorder.expect(resolve("r", 0x0F, [.control]) == nil, "⌃R이 도구로 해석됨")
        recorder.expect(resolve("r", 0x0F, repeat: true) == nil, "키 반복이 도구로 해석됨")
        recorder.expect(resolve("ㄱ", 0x0F) == .rectangle, "한글 입력 소스(ㄱ, R 위치) 해석 실패")
        recorder.expect(resolve("ㅍ", 0x23) == .pen, "한글 입력 소스(ㅍ, P 위치) 해석 실패")
        recorder.expect(resolve("1", 0x12) == nil, "숫자 키가 도구로 해석됨")
        recorder.expect(resolve("q", 0x0C) == nil, "할당 없는 Q가 도구로 해석됨")

        if let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: 0,
            context: nil, characters: "t", charactersIgnoringModifiers: "t", isARepeat: false, keyCode: 0x11
        ) {
            recorder.expect(CanvasToolShortcut.tool(for: event) == .text, "NSEvent T → Text 해석 실패")
        } else {
            recorder.expect(false, "검사용 key event 생성 실패")
        }
    }

    // MARK: 4. 메인 메뉴

    private static func checkMainMenu(
        _ mainMenu: MainMenuBuilder.MainMenu,
        appRouter: ShellCommandRouter,
        _ recorder: Recorder
    ) -> Int {
        let titles = mainMenu.menu.items.map(\.title)
        recorder.expect(titles == MainMenuBuilder.topLevelTitles, "최상위 메뉴 순서 불일치: \(titles)")
        recorder.expect(mainMenu.windowsMenu.title == "Window", "windowsMenu가 Window 메뉴가 아님")
        recorder.expect(mainMenu.helpMenu.title == "Help", "helpMenu가 Help 메뉴가 아님")

        var visibleCommands: [ShellCommand] = []
        var itemCount = 0
        let customAction = #selector(ShellCommandRouter.performShellCommand(_:))

        func walk(_ menu: NSMenu, path: String) {
            var previous: NSMenuItem?
            var group: [NSMenuItem] = []
            func checkGroup() {
                let visible = group.filter { !$0.isHidden }
                let withImage = visible.filter { $0.image != nil }.count
                recorder.expect(
                    withImage == 0 || withImage == visible.count,
                    "\(path) 아이콘 그룹 불균일: \(visible.map { "\($0.title)\($0.image == nil ? "" : "*")" })"
                )
                group.removeAll()
            }
            for item in menu.items {
                if item.isSeparatorItem {
                    checkGroup()
                    previous = item
                    continue
                }
                group.append(item)
                itemCount += 1
                recorder.expect(!item.title.isEmpty, "\(path) 제목 없는 항목")
                if let submenu = item.submenu {
                    // 하위 메뉴를 붙이면 AppKit이 submenuAction:을 지정합니다. 그 밖의 action은 두지 않습니다.
                    recorder.expect(
                        item.action == nil || item.action == #selector(NSMenu.submenuAction(_:)),
                        "\(path)/\(item.title) 컨테이너에 명령 action이 있음"
                    )
                    recorder.expect(ShellCommand.resolve(from: item) == nil, "\(path)/\(item.title) 컨테이너가 명령으로 해석됨")
                    walk(submenu, path: "\(path)/\(item.title)")
                    previous = item
                    continue
                }
                guard let command = ShellCommand.resolve(from: item),
                      let identifier = item.identifier,
                      ShellCommand(menuItemIdentifier: identifier) == command
                else {
                    recorder.expect(false, "\(path)/\(item.title) ShellCommand 대응 없음")
                    previous = item
                    continue
                }
                if !item.isHidden { visibleCommands.append(command) }
                recorder.expect(item.title == command.title, "\(command.rawValue) 제목 불일치: \(item.title)")
                recorder.expect(
                    item.action == (command.standardAction ?? customAction),
                    "\(command.rawValue) action 불일치: \(String(describing: item.action))"
                )
                if command.standardAction != nil {
                    recorder.expect(item.target == nil, "\(command.rawValue) 표준 action에 target이 있음")
                } else if command.scope == .application {
                    recorder.expect(item.target === appRouter, "\(command.rawValue) 앱 수준 target이 아님")
                } else {
                    recorder.expect(item.target == nil, "\(command.rawValue) 창 수준인데 target이 있음")
                }
                // key equivalent: 표시 항목은 shortcut, 숨김 항목은 보조 shortcut.
                let expected: [KeyShortcut] = item.isHidden
                    ? command.hiddenAlternateShortcuts
                    : (command.shortcut.map { [$0] } ?? [])
                if expected.isEmpty {
                    recorder.expect(item.keyEquivalent.isEmpty, "\(command.rawValue) 예상 밖 key equivalent '\(item.keyEquivalent)'")
                } else {
                    let matches = expected.contains {
                        $0.key == item.keyEquivalent
                            && $0.modifiers.eventFlags == item.keyEquivalentModifierMask.intersection([.command, .shift, .option, .control])
                    }
                    recorder.expect(matches, "\(command.rawValue) key equivalent 불일치")
                }
                if !item.keyEquivalent.isEmpty {
                    recorder.expect(
                        item.keyEquivalentModifierMask.contains(.command),
                        "\(command.rawValue) Command 없는 key equivalent(단일 키는 캔버스 keyDown 전용)"
                    )
                }
                if item.isHidden {
                    recorder.expect(item.allowsKeyEquivalentWhenHidden, "\(command.rawValue) 숨김 보조 단축키가 동작하지 않음")
                }
                if item.isAlternate {
                    let base = previous
                    recorder.expect(
                        base?.keyEquivalent == item.keyEquivalent
                            && base?.keyEquivalentModifierMask != item.keyEquivalentModifierMask,
                        "\(command.rawValue) 동적 항목이 앞 항목과 같은 키·다른 수식키가 아님"
                    )
                }
                if let tool = command.canvasTool, let key = tool.singleKey {
                    recorder.expect(
                        item.toolTip?.contains(String(key).uppercased()) == true,
                        "\(command.rawValue) 도움말 태그에 단일 키 없음"
                    )
                }
                previous = item
            }
            checkGroup()
        }
        walk(mainMenu.menu, path: "Main")

        let missing = Set(ShellCommand.allCases).subtracting(visibleCommands)
        recorder.expect(missing.isEmpty, "메뉴에 없는 명령: \(missing.map(\.rawValue).sorted())")
        let duplicated = Dictionary(grouping: visibleCommands, by: { $0 }).filter { $0.value.count > 1 }.keys
        recorder.expect(duplicated.isEmpty, "메뉴에 두 번 이상 보이는 명령: \(duplicated.map(\.rawValue).sorted())")

        let servicesItem = mainMenu.menu.items.first?.submenu?.items.first { $0.title == "Services" }
        recorder.expect(servicesItem?.submenu === mainMenu.servicesMenu, "Services 하위 메뉴 연결 없음")
        return itemCount
    }

    /// 메뉴에서 명령의 첫 표시 항목을 찾습니다.
    private static func menuItem(for command: ShellCommand, in menu: NSMenu) -> NSMenuItem? {
        for item in menu.items {
            if let submenu = item.submenu, let found = menuItem(for: command, in: submenu) { return found }
            if !item.isHidden, let identifier = item.identifier, ShellCommand(menuItemIdentifier: identifier) == command {
                return item
            }
        }
        return nil
    }

    // MARK: 5. 앱 수준 라우팅

    private static func checkApplicationRouting(
        _ mainMenu: MainMenuBuilder.MainMenu,
        appRouter: ShellCommandRouter,
        appModel: ToolsMenuModelStub,
        _ recorder: Recorder
    ) {
        guard let newProject = menuItem(for: .newProject, in: mainMenu.menu) else {
            recorder.expect(false, "New Project 항목 없음")
            return
        }
        recorder.expect(appRouter.validateMenuItem(newProject), "창 없이 New Project 비활성")
        _ = NSApp.sendAction(newProject.action!, to: newProject.target, from: newProject)
        recorder.expect(appModel.sentCommands == [.newProject], "New Project 명령 전달 실패: \(appModel.sentCommands)")
    }

    // MARK: 6. 창: 라우팅·검증·툴바·레이아웃·성능

    private struct Timing {
        var applyMedianMicros = 0
        var applyMaxMicros = 0
        var validateMedianMicros = 0
    }

    private static func checkWindow(_ mainMenu: MainMenuBuilder.MainMenu, _ recorder: Recorder) -> Timing {
        var timing = Timing()
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1280, height: 800),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.isReleasedWhenClosed = false
        window.title = "Tools Check"
        defer {
            window.orderOut(nil)
            window.close()
        }

        // 콘텐츠: 네이티브 텍스트 필드(텍스트 편집 억제 확인)와 캔버스 자리 포커스 뷰.
        let content = NSView()
        let textField = NativeTextControls.makeLayerNameField("Layer")
        let focusView = FocusStandInView()
        for view in [textField, focusView] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            content.addSubview(view)
        }
        NSLayoutConstraint.activate([
            textField.topAnchor.constraint(equalTo: content.safeAreaLayoutGuide.topAnchor, constant: 12),
            textField.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 12),
            textField.widthAnchor.constraint(equalToConstant: 200),
            focusView.topAnchor.constraint(equalTo: textField.bottomAnchor, constant: 12),
            focusView.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            focusView.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            focusView.bottomAnchor.constraint(equalTo: content.bottomAnchor),
        ])
        window.contentView = content

        let model = ToolsMenuModelStub()
        let router = ShellCommandRouter(model: model)
        recorder.expect(router.install(in: window), "router 설치 실패")
        recorder.expect(content.nextResponder === router && router.nextResponder === window, "응답자 체인 위치 불일치")
        recorder.expect(router.install(in: window), "router 재설치 실패")
        recorder.expect(router.nextResponder === window, "router 재설치가 체인을 바꿈")

        let controller = ToolsToolbarController(model: model, router: router)
        let toolbar = controller.makeToolbar(
            identifier: NSToolbar.Identifier("xyz.taeyoon.stellar.check.toolbar"),
            autosavesConfiguration: false
        )
        window.toolbar = toolbar
        let identifiers = toolbar.items.map(\.itemIdentifier)
        recorder.expect(
            identifiers.contains(ToolsToolbarController.toolsItemIdentifier)
                && identifiers.contains(ToolsToolbarController.zoomItemIdentifier),
            "툴바에 도구·줌 항목 없음: \(identifiers.map(\.rawValue))"
        )

        checkRouterWithContentViewController(recorder)
        checkObservers(model: model, controller: controller, recorder)
        checkPaletteCopies(controller: controller, toolbar: toolbar, recorder)

        withExtendedLifetime(controller) {
            checkToolbarItems(controller, router: router, recorder)
            checkRouting(
                window: window, mainMenu: mainMenu, model: model, router: router,
                controller: controller, textField: textField, focusView: focusView, recorder
            )
            checkLayout(window: window, recorder)
            timing = measure(mainMenu: mainMenu, model: model, router: router, controller: controller, recorder)
        }
        return timing
    }

    /// content view controller가 있는 창(셸 프레임 창): router는 controller와 창 사이에 들어가고 체인이 순환하지 않습니다.
    private static func checkRouterWithContentViewController(_ recorder: Recorder) {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 400, height: 300),
            styleMask: [.titled, .closable], backing: .buffered, defer: true
        )
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let controller = NSViewController()
        controller.view = NSView()
        window.contentViewController = controller
        let router = ShellCommandRouter(model: ToolsMenuModelStub())
        recorder.expect(router.install(in: window), "content view controller 창에 router 설치 실패")
        recorder.expect(router.install(in: window), "content view controller 창에 router 재설치 실패")
        var chain: [NSResponder] = []
        var cursor: NSResponder? = window.contentView
        while let responder = cursor, chain.count < 16 {
            chain.append(responder)
            cursor = responder.nextResponder
        }
        recorder.expect(chain.count < 16, "router 설치 후 응답자 체인이 순환함")
        recorder.expect(chain.filter { $0 === router }.count == 1, "router가 체인에 한 번 있지 않음")
        recorder.expect(controller.nextResponder === router && router.nextResponder === window, "router가 controller와 창 사이에 없음")
    }

    /// snapshot 구독이 여럿이어도 서로 덮어쓰지 않고, 토큰을 취소하면 그 구독만 끝납니다.
    private static func checkObservers(
        model: ToolsMenuModelStub,
        controller: ToolsToolbarController,
        _ recorder: Recorder
    ) {
        let before = model.observers.count
        var received: [UInt64] = []
        var second: ToolsMenuObservation? = model.addSnapshotObserver { snapshot in
            received.append(snapshot.generation)
        }
        recorder.expect(model.observers.count == before + 1, "두 번째 구독이 추가되지 않음")
        model.send(.zoomOut)
        recorder.expect(received == [model.snapshot.generation], "두 번째 구독자가 snapshot을 받지 못함: \(received)")
        recorder.expect(
            controller.zoomItem.title == ZoomFormat.percentString(model.snapshot.zoomScale),
            "두 번째 구독이 툴바 구독을 덮어씀"
        )
        second?.cancel()
        second = nil
        model.send(.zoomIn)
        recorder.expect(received.count == 1, "취소한 구독자가 계속 snapshot을 받음")
        recorder.expect(model.observers.count == before, "취소한 구독이 남아 있음")
        recorder.expect(controller.zoomItem.title == ZoomFormat.percentString(1.0), "줌 복귀 실패: \(controller.zoomItem.title)")
    }

    /// 사용자 구성 palette 사본: 실제 항목과 다른 인스턴스이고 명령을 보내지 않습니다.
    private static func checkPaletteCopies(
        controller: ToolsToolbarController,
        toolbar: NSToolbar,
        _ recorder: Recorder
    ) {
        autoreleasepool {
            let tools = controller.toolbar(
                toolbar, itemForItemIdentifier: ToolsToolbarController.toolsItemIdentifier, willBeInsertedIntoToolbar: false
            ) as? NSToolbarItemGroup
            let zoom = controller.toolbar(
                toolbar, itemForItemIdentifier: ToolsToolbarController.zoomItemIdentifier, willBeInsertedIntoToolbar: false
            ) as? NSMenuToolbarItem
            recorder.expect(tools != nil && tools !== controller.toolsGroup, "palette 도구 그룹이 실제 항목과 같음")
            recorder.expect(zoom != nil && zoom !== controller.zoomItem, "palette 줌 항목이 실제 항목과 같음")
            recorder.expect(tools?.target == nil && tools?.action == nil, "palette 도구 그룹이 명령을 보낼 수 있음")
            // NSMenuToolbarItem은 메뉴 항목 action을 내부 pop-up action으로 감싸므로 target으로 판정합니다.
            let liveZoomItems = zoom?.menu.items.filter { $0.target === controller.router } ?? []
            recorder.expect(liveZoomItems.isEmpty, "palette 줌 메뉴가 router로 명령을 보낼 수 있음: \(liveZoomItems.map(\.title))")
            recorder.expect(tools?.subitems.count == CanvasTool.allCases.count, "palette 도구 그룹 항목 수 불일치")
        }
        let real = toolbar.items.first { $0.itemIdentifier == ToolsToolbarController.toolsItemIdentifier }
        recorder.expect(real === controller.toolsGroup, "palette 뒤 실제 도구 그룹이 바뀜")
    }

    private static func checkToolbarItems(
        _ controller: ToolsToolbarController,
        router: ShellCommandRouter,
        _ recorder: Recorder
    ) {
        let group = controller.toolsGroup
        let tools = CanvasTool.allCases
        recorder.expect(group.label == "Tools" && !(group.toolTip ?? "").isEmpty, "도구 그룹 레이블·도움말 없음")
        recorder.expect(group.selectionMode == .selectOne, "도구 그룹이 단일 선택이 아님")
        recorder.expect(group.controlRepresentation == .expanded, "도구 그룹이 펼친 표현이 아님(한 번 클릭 전환)")
        recorder.expect(
            group.visibilityPriority.rawValue > controller.zoomItem.visibilityPriority.rawValue,
            "줌 항목이 도구 그룹보다 먼저 overflow되지 않음"
        )
        recorder.expect(group.subitems.count == tools.count, "도구 그룹 항목 수 \(group.subitems.count)")
        for (subitem, tool) in zip(group.subitems, tools) {
            recorder.expect(subitem.label == tool.label, "\(tool.rawValue) 툴바 레이블 불일치")
            recorder.expect(subitem.toolTip == tool.helpTag, "\(tool.rawValue) 툴바 도움말 불일치")
            recorder.expect(
                subitem.image?.accessibilityDescription == tool.label,
                "\(tool.rawValue) 툴바 아이콘 접근성 설명 없음"
            )
        }
        // 표준 구성요소만(커스텀 뷰 없음): 접근성 표시 옵션·외형 대응을 시스템이 처리합니다.
        recorder.expect(group.view == nil && controller.zoomItem.view == nil, "툴바 항목에 커스텀 뷰가 있음")

        let zoom = controller.zoomItem
        recorder.expect(zoom.label == "Zoom" && !(zoom.toolTip ?? "").isEmpty, "줌 항목 레이블·도움말 없음")
        recorder.expect(zoom.title == ZoomFormat.percentString(1.0), "줌 제목이 100%가 아님: \(zoom.title)")
        let zoomCommands = zoom.menu.items.compactMap { ShellCommand.resolve(from: $0) }
        recorder.expect(zoomCommands == ShellCommand.zoomCommands, "줌 메뉴 명령 불일치: \(zoomCommands.map(\.rawValue))")
        for item in zoom.menu.items where !item.isSeparatorItem {
            recorder.expect(item.target === router, "줌 메뉴 항목 target이 router가 아님")
            recorder.expect(item.keyEquivalent.isEmpty, "줌 툴바 메뉴에 key equivalent가 있음(View 메뉴와 중복)")
        }
    }

    private static func checkRouting(
        window: NSWindow,
        mainMenu: MainMenuBuilder.MainMenu,
        model: ToolsMenuModelStub,
        router: ShellCommandRouter,
        controller: ToolsToolbarController,
        textField: NSTextField,
        focusView: NSView,
        _ recorder: Recorder
    ) {
        func item(_ command: ShellCommand) -> NSMenuItem {
            menuItem(for: command, in: mainMenu.menu) ?? MainMenuBuilder.makeItem(for: command, target: nil)
        }
        let tools = CanvasTool.allCases
        let group = controller.toolsGroup

        // 캔버스 포커스: 표준 편집 action과 셸 명령이 router로 갑니다.
        recorder.expect(window.makeFirstResponder(focusView), "캔버스 자리 first responder 실패")
        recorder.expect(!router.isTextEditing, "캔버스 포커스인데 텍스트 편집으로 판정")
        recorder.expect(focusView.tryToPerform(#selector(NSText.copy(_:)), with: item(.copy)), "copy: 응답자 없음")
        recorder.expect(model.sentCommands.last == .copy, "copy:가 router로 가지 않음")
        recorder.expect(focusView.tryToPerform(#selector(ShellCommandRouter.undo(_:)), with: item(.undo)), "undo: 응답자 없음")
        recorder.expect(model.sentCommands.last == .undo, "undo:가 창보다 먼저 router로 가지 않음")
        let custom = #selector(ShellCommandRouter.performShellCommand(_:))
        recorder.expect(focusView.tryToPerform(custom, with: item(.zoomIn)), "performShellCommand: 응답자 없음")
        recorder.expect(model.sentCommands.last == .zoomIn && model.snapshot.zoomScale == 2.0, "Zoom In 전달 실패")
        recorder.expect(controller.zoomItem.title == ZoomFormat.percentString(2.0), "줌 제목 갱신 실패: \(controller.zoomItem.title)")

        // 메뉴 검증: 제목·체크·활성.
        let undoItem = item(.undo)
        recorder.expect(router.validateMenuItem(undoItem) && undoItem.title == "Undo Move", "Undo 제목·활성 불일치: \(undoItem.title)")
        recorder.expect(!router.validateMenuItem(item(.redo)), "Redo가 활성(Stub은 불가)")
        let rulers = item(.toggleRulers)
        recorder.expect(!router.validateMenuItem(rulers) && rulers.title == "Show Rulers", "Rulers placeholder가 활성이거나 제목 불일치")
        let moveItem = item(.selectMoveTool)
        _ = router.validateMenuItem(moveItem)
        recorder.expect(moveItem.state == .on, "Move Tool 체크 없음")
        _ = focusView.tryToPerform(custom, with: item(.toggleLock))
        let lockItem = item(.toggleLock)
        _ = router.validateMenuItem(lockItem)
        recorder.expect(lockItem.title == "Unlock", "Lock 토글 제목 불일치: \(lockItem.title)")

        // 사용자가 도구 그룹을 누름 → 명령 → snapshot → 선택·메뉴 체크.
        guard let pen = tools.firstIndex(of: .pen), let rectangle = tools.firstIndex(of: .rectangle),
              let image = tools.firstIndex(of: .image), let move = tools.firstIndex(of: .move)
        else { return }
        recorder.expect(group.selectedIndex == move, "초기 도구 선택이 Move가 아님: \(group.selectedIndex)")
        group.selectedIndex = pen
        controller.toolSelectionChanged(group)
        recorder.expect(model.sentCommands.last == .selectPenTool && model.snapshot.activeTool == .pen, "도구 그룹 → Pen 명령 실패")
        let penItem = item(.selectPenTool)
        _ = router.validateMenuItem(penItem)
        _ = router.validateMenuItem(moveItem)
        recorder.expect(penItem.state == .on && moveItem.state == .off, "Pen 체크 갱신 실패")

        // 코어(Stub) 쪽 변경 → 툴바 선택 반영.
        model.send(.selectRectangleTool)
        recorder.expect(group.selectedIndex == rectangle, "snapshot 도구 → 툴바 선택 반영 실패")

        // 오래된 snapshot 폐기.
        var stale = model.snapshot
        stale.generation = 0
        stale.activeTool = .hand
        controller.apply(stale)
        recorder.expect(group.selectedIndex == rectangle, "오래된 snapshot이 반영됨")

        // 비활성 도구: 항목 비활성, 눌러도 거부되고 선택이 되돌아갑니다.
        var noImage = model.snapshot
        noImage.generation += 1
        noImage.enabledCommands.remove(.selectImageTool)
        model.receive(noImage)
        recorder.expect(!group.subitems[image].isEnabled && group.isEnabled, "Image 도구 비활성 반영 실패")
        let sentBefore = model.sentCommands.count
        group.selectedIndex = image
        controller.toolSelectionChanged(group)
        recorder.expect(model.sentCommands.count == sentBefore, "비활성 도구 명령이 전달됨")
        recorder.expect(group.selectedIndex == rectangle, "거부된 도구 선택이 되돌아가지 않음")

        // 로딩·빈 상태: 실행 가능한 명령이 없으면 도구·줌이 비활성이고 메뉴는 숨지 않고 비활성입니다.
        var loading = model.snapshot
        loading.generation += 1
        loading.enabledCommands = []
        model.receive(loading)
        recorder.expect(!group.isEnabled && !controller.zoomItem.isEnabled, "로딩 상태에서 도구·줌이 활성")
        let duplicateItem = item(.duplicate)
        recorder.expect(!router.validateMenuItem(duplicateItem) && !duplicateItem.isHidden, "로딩 상태 메뉴가 활성이거나 숨겨짐")
        var restored = ToolsMenuModelStub.sampleSnapshot
        restored.generation = model.snapshot.generation + 1
        restored.activeTool = .rectangle
        model.receive(restored)
        recorder.expect(group.isEnabled && controller.zoomItem.isEnabled, "상태 복원 실패")

        // 텍스트 편집 중: 객체 명령 억제, 보기 명령은 유지, 표준 편집 action은 field editor가 받음.
        recorder.expect(window.makeFirstResponder(textField), "텍스트 필드 first responder 실패")
        recorder.expect(router.isTextEditing, "텍스트 편집 판정 실패")
        recorder.expect(!router.validateMenuItem(item(.duplicate)), "텍스트 편집 중 Duplicate가 활성")
        recorder.expect(!router.validateMenuItem(item(.group)), "텍스트 편집 중 Group이 활성")
        recorder.expect(router.validateMenuItem(item(.zoomIn)), "텍스트 편집 중 Zoom In이 비활성")
        let countBefore = model.sentCommands.count
        if let editor = window.firstResponder {
            _ = editor.tryToPerform(#selector(NSText.copy(_:)), with: item(.copy))
            _ = editor.tryToPerform(#selector(NSText.selectAll(_:)), with: item(.selectAll))
        }
        recorder.expect(model.sentCommands.count == countBefore, "텍스트 편집 중 표준 편집 action이 router로 감")
        recorder.expect(!router.dispatch(.duplicate), "텍스트 편집 중 Duplicate 직접 전달이 허용됨")
        checkTextEditingUndo(window: window, router: router, model: model, undoItem: item(.undo), redoItem: item(.redo), recorder)
        recorder.expect(window.makeFirstResponder(focusView), "포커스 복귀 실패")
    }

    /// 텍스트 편집 중 ⌘Z·⇧⌘Z는 창 undo manager가 텍스트 입력을 되돌립니다(디자인 가이드 5.2, 8.2).
    /// field editor는 `undo:`를 구현하지 않으므로 router가 응답자 체인을 창으로 넘기는지 확인합니다.
    private static func checkTextEditingUndo(
        window: NSWindow,
        router: ShellCommandRouter,
        model: ToolsMenuModelStub,
        undoItem: NSMenuItem,
        redoItem: NSMenuItem,
        _ recorder: Recorder
    ) {
        guard let editor = window.firstResponder as? NSTextView, let undoManager = editor.undoManager else {
            recorder.expect(false, "텍스트 편집 중 field editor·undo manager 없음")
            return
        }
        let undo = #selector(ShellCommandRouter.undo(_:))
        let redo = #selector(ShellCommandRouter.redo(_:))
        recorder.expect(!editor.responds(to: undo), "field editor가 undo:를 직접 구현함(검사 전제 바뀜)")
        recorder.expect(!router.responds(to: undo) && !router.responds(to: redo), "텍스트 편집 중 router가 undo:·redo:를 가로챔")

        // 응답자 체인에서 undo:를 처리할 첫 대상은 창입니다.
        var cursor: NSResponder? = editor
        var handler: NSResponder?
        while let responder = cursor {
            if responder.responds(to: undo) {
                handler = responder
                break
            }
            cursor = responder.nextResponder
        }
        recorder.expect(handler === window, "텍스트 편집 중 undo: 처리 대상이 창이 아님: \(handler.map { String(describing: type(of: $0)) } ?? "없음")")

        undoManager.removeAllActions()
        let original = editor.string
        editor.insertText("Renamed", replacementRange: NSRange(location: 0, length: (original as NSString).length))
        editor.breakUndoCoalescing()
        let typed = editor.string
        recorder.expect(typed == "Renamed", "field editor 입력 실패: '\(typed)'")
        let sentBefore = model.sentCommands
        recorder.expect(window.validateUserInterfaceItem(undoItem), "텍스트 편집 중 창 Undo가 비활성")
        recorder.expect(undoItem.title != "Undo Move", "텍스트 편집 중 Undo 제목이 캔버스 명령 이름임: \(undoItem.title)")
        recorder.expect(router.validateMenuItem(undoItem), "텍스트 편집 중 router 직접 검증이 창 결과와 다름")
        recorder.expect(editor.tryToPerform(undo, with: undoItem), "텍스트 편집 중 undo: 응답자 없음")
        recorder.expect(editor.string != typed, "텍스트 편집 중 ⌘Z가 입력을 되돌리지 않음: '\(editor.string)'")
        recorder.expect(editor.tryToPerform(redo, with: redoItem), "텍스트 편집 중 redo: 응답자 없음")
        recorder.expect(editor.string == typed, "텍스트 편집 중 ⇧⌘Z가 입력을 다시 적용하지 않음: '\(editor.string)'")
        // router를 직접 불러도 창으로 넘깁니다.
        router.undo(undoItem)
        recorder.expect(editor.string != typed, "router.undo 직접 호출이 창으로 넘어가지 않음")
        recorder.expect(model.sentCommands == sentBefore, "텍스트 편집 중 Undo·Redo가 코어 명령으로 감: \(model.sentCommands.suffix(3))")
        undoManager.removeAllActions()
    }

    private static func checkLayout(window: NSWindow, _ recorder: Recorder) {
        // 화면 밖에 두고 그려 툴바 뷰를 실체화합니다(활성화하지 않음).
        window.setFrame(NSRect(x: -20000, y: -20000, width: 1280, height: 800), display: false)
        window.orderBack(nil)

        func ambiguousViews(in view: NSView) -> [String] {
            var found = view.hasAmbiguousLayout ? ["\(type(of: view))"] : []
            for subview in view.subviews { found += ambiguousViews(in: subview) }
            return found
        }
        func containsLabel(_ label: String, in view: NSView) -> Bool {
            if view.accessibilityLabel() == label { return true }
            return view.subviews.contains { containsLabel(label, in: $0) }
        }

        let appearances: [NSAppearance.Name] = [
            .aqua, .darkAqua, .accessibilityHighContrastAqua, .accessibilityHighContrastDarkAqua,
        ]
        for name in appearances {
            guard let appearance = NSAppearance(named: name) else {
                recorder.expect(false, "외형 \(name.rawValue) 없음")
                continue
            }
            window.appearance = appearance
            window.layoutIfNeeded()
            window.displayIfNeeded()
            guard let frameView = window.contentView?.superview else {
                recorder.expect(false, "창 frame view 없음")
                return
            }
            let ambiguous = ambiguousViews(in: frameView)
            recorder.expect(ambiguous.isEmpty, "\(name.rawValue) 모호한 Auto Layout: \(ambiguous)")
            recorder.expect(containsLabel("Zoom", in: frameView), "\(name.rawValue) 줌 툴바 항목 접근성 레이블 없음")
        }

        // 일반 크기(1280pt)에서는 기본 항목이 overflow되지 않아야 합니다(HIG Toolbars).
        let visible = window.toolbar?.visibleItems?.map(\.itemIdentifier) ?? []
        recorder.expect(
            visible.contains(ToolsToolbarController.toolsItemIdentifier)
                && visible.contains(ToolsToolbarController.zoomItemIdentifier),
            "1280pt에서 도구·줌 항목이 overflow됨: \(visible.map(\.rawValue))"
        )

        // 좁은 폭: 도구 그룹이 접혀도 레이아웃이 모호하지 않아야 합니다.
        window.appearance = NSAppearance(named: .aqua)
        window.setFrame(NSRect(x: -20000, y: -20000, width: 560, height: 600), display: false)
        window.layoutIfNeeded()
        window.displayIfNeeded()
        if let frameView = window.contentView?.superview {
            let ambiguous = ambiguousViews(in: frameView)
            recorder.expect(ambiguous.isEmpty, "좁은 폭 모호한 Auto Layout: \(ambiguous)")
        }
        window.setFrame(NSRect(x: -20000, y: -20000, width: 1280, height: 800), display: false)
    }

    private static func measure(
        mainMenu: MainMenuBuilder.MainMenu,
        model: ToolsMenuModelStub,
        router: ShellCommandRouter,
        controller: ToolsToolbarController,
        _ recorder: Recorder
    ) -> Timing {
        func micros(_ block: () -> Void) -> Int {
            let start = DispatchTime.now().uptimeNanoseconds
            block()
            return Int((DispatchTime.now().uptimeNanoseconds - start) / 1_000)
        }
        func median(_ values: [Int]) -> Int {
            let sorted = values.sorted()
            return sorted.isEmpty ? 0 : sorted[sorted.count / 2]
        }

        var snapshot = model.snapshot
        var applySamples: [Int] = []
        for index in 0..<60 {
            snapshot.generation += 1
            snapshot.activeTool = CanvasTool.allCases[index % CanvasTool.allCases.count]
            snapshot.zoomScale = index.isMultiple(of: 2) ? 1.0 : 1.5
            let next = snapshot
            applySamples.append(micros { controller.apply(next) })
        }

        var validateSamples: [Int] = []
        if let viewMenu = mainMenu.menu.items.first(where: { $0.title == "View" })?.submenu {
            let items = viewMenu.items.filter { !$0.isSeparatorItem && $0.target == nil && $0.action == #selector(ShellCommandRouter.performShellCommand(_:)) }
            for _ in 0..<30 {
                validateSamples.append(micros { for item in items { _ = router.validateMenuItem(item) } })
            }
        }

        var timing = Timing()
        timing.applyMedianMicros = median(applySamples)
        timing.applyMaxMicros = applySamples.max() ?? 0
        timing.validateMedianMicros = median(validateSamples)
        recorder.expect(timing.applyMedianMicros < 2_000, "snapshot 반영 median \(timing.applyMedianMicros)µs ≥ 2ms")
        recorder.expect(timing.validateMedianMicros < 2_000, "View 메뉴 검증 median \(timing.validateMedianMicros)µs ≥ 2ms")
        return timing
    }
}
