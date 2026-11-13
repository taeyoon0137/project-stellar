import AppKit

/// 시작 창 영역을 headless로 검증합니다(통합 시 `scripts/macos/smoke.sh`에 연결).
///
/// - 시작 창·Clone sheet·명령 설치 sheet를 Stub 데이터로 화면 밖에서 만들고 레이아웃을 강제한 뒤,
///   모호한 Auto Layout, 잘린 여러 줄 레이블, 직접 그린 반투명 표면이 없는지 확인합니다.
/// - 주요 접근성 레이블, 키보드 경로(Return·Delete·⌘N·⌘O), 상태별 표시(빈·읽는 중·오류·Git 없음·진행 중)를 확인합니다.
/// - `StellarCommandInstaller`를 메모리 파일 시스템(`StellarCommandFileSystemStub`)으로 검사합니다:
///   설치·멱등·제거, 남의 파일 보존, 대체 이름, 오래된 링크 다시 연결, 경합, 권한 상승 없는 위치 선택.
/// - Light, Dark, 대비 높이기(Light·Dark) 외형에서 같은 레이아웃 검사를 반복합니다.
/// 실제 디스크에는 쓰지 않으며, 시스템 sheet·패널은 띄우지 않습니다.
enum WelcomeUICheck {
    static func run() -> Bool {
        let checker = Checker()
        let application = NSApplication.shared
        application.setActivationPolicy(.accessory)

        checkInstaller(checker)
        checkSearchPath(checker)
        checkFirstLaunchPolicy(checker)
        checkWelcomeWindow(checker)
        checkNoticeAlerts(checker)
        checkCloneSheet(checker)
        checkInstallSheet(checker)

        // 실제 앱 번들 상태는 환경마다 다르므로 보고만 합니다(초기 빌드에는 보조 실행 파일이 없음).
        let realStatus = StellarCommandInstaller.forRunningApp().status()
        checker.note("running app command status=\(describe(realStatus))")

        if checker.failures.isEmpty {
            print("welcome-ui ok: \(checker.passed) checks [\(checker.notes.joined(separator: "; "))]")
            return true
        }
        for failure in checker.failures {
            FileHandle.standardError.write(Data("welcome-ui 실패: \(failure)\n".utf8))
        }
        return false
    }

    // MARK: 공통

    final class Checker {
        private(set) var failures: [String] = []
        private(set) var notes: [String] = []
        private(set) var passed = 0

        func expect(_ condition: Bool, _ message: @autoclosure () -> String) {
            if condition { passed += 1 } else { failures.append(message()) }
        }

        func note(_ text: String) {
            notes.append(text)
        }
    }

    static let appearances: [NSAppearance.Name] = [
        .aqua, .darkAqua, .accessibilityHighContrastAqua, .accessibilityHighContrastDarkAqua,
    ]

    /// 레이아웃 문제(모호한 제약, 세로로 잘린 여러 줄 레이블, 직접 넣은 반투명 표면)를 찾습니다.
    static func layoutProblems(in root: NSView, context: String) -> [String] {
        root.layoutSubtreeIfNeeded()
        var problems: [String] = []
        func visit(_ view: NSView) {
            guard !view.isHidden else { return }
            if view.hasAmbiguousLayout {
                problems.append("\(context): 모호한 레이아웃 \(type(of: view)) \(view.frame)")
            }
            // NSScrollView가 스스로 넣는 표면(scroll edge effect 등)은 시스템 소유이므로 제외합니다.
            if view is NSVisualEffectView || view is NSGlassEffectView, !(view.superview is NSScrollView) {
                var chain: [String] = []
                var current: NSView? = view.superview
                while let ancestor = current, ancestor !== root {
                    chain.append("\(type(of: ancestor))")
                    current = ancestor.superview
                }
                problems.append("\(context): 직접 넣은 반투명 표면 \(type(of: view)) <- \(chain.joined(separator: " <- "))")
            }
            if let label = view as? NSTextField, !label.isEditable, label.cell?.wraps == true,
                label.frame.width > 0, let cell = label.cell
            {
                let needed = cell.cellSize(
                    forBounds: NSRect(x: 0, y: 0, width: label.frame.width, height: .greatestFiniteMagnitude)
                ).height
                if label.frame.height + 0.5 < needed {
                    problems.append("\(context): 잘린 레이블 '\(label.stringValue.prefix(24))'")
                }
            }
            for subview in view.subviews { visit(subview) }
        }
        visit(root)
        return problems
    }

    /// 모든 외형에서 레이아웃을 검사합니다.
    static func expectCleanLayout(_ checker: Checker, window: NSWindow, context: String) {
        guard let root = window.contentView else {
            checker.expect(false, "\(context): content view 없음")
            return
        }
        for name in appearances {
            window.appearance = NSAppearance(named: name)
            let problems = layoutProblems(in: root, context: "\(context)/\(name.rawValue)")
            checker.expect(problems.isEmpty, problems.joined(separator: "; "))
        }
        window.appearance = nil
    }

    /// view controller를 화면에 띄우지 않는 창에 붙입니다.
    static func offscreenWindow(for controller: NSViewController) -> NSWindow {
        let window = NSWindow(contentViewController: controller)
        window.isReleasedWhenClosed = false
        window.contentView?.layoutSubtreeIfNeeded()
        return window
    }

    static func keyEvent(_ keyCode: UInt16, characters: String, window: NSWindow) -> NSEvent? {
        NSEvent.keyEvent(
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
        )
    }

    /// 메인 큐 작업이 끝날 때까지 런 루프를 돌립니다.
    static func spin(until condition: () -> Bool, timeout: TimeInterval = 3) -> Bool {
        let deadline = Date(timeIntervalSinceNow: timeout)
        while !condition() {
            if Date() > deadline { return false }
            RunLoop.main.run(mode: .default, before: Date(timeIntervalSinceNow: 0.01))
        }
        return true
    }

    static func describe(_ status: StellarCommandInstallStatus) -> String {
        switch status {
        case .helperUnavailable: return "helperUnavailable"
        case .unsupportedAppLocation(let location): return "unsupportedAppLocation(\(location))"
        case .ready(let plan): return "ready(\(plan.commandName))"
        case .installed(let installation): return "installed(\(installation.commandName))"
        case .needsRepair: return "needsRepair"
        case .blocked: return "blocked"
        case .noWritableLocation: return "noWritableLocation"
        }
    }

    // MARK: 설치기

    static let stubHome = "/Users/stub"
    static let stubApp = "/Applications/Stellar.app"

    /// 보조 실행 파일이 있는 앱과 쓰기 불가 `/usr/local/bin`이 있는 표준 메모리 파일 시스템.
    static func makeFileSystem(helper: Bool = true) -> StellarCommandFileSystemStub {
        let fileSystem = StellarCommandFileSystemStub(homeDirectory: stubHome)
        fileSystem.addDirectory("/usr/local/bin", writable: false)
        fileSystem.addDirectory("/Applications", writable: false)
        if helper {
            fileSystem.addFile(stubApp + "/Contents/Helpers/stellar-cli", executable: true)
        }
        return fileSystem
    }

    static let defaultSearchPath = ["/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"]

    static func checkInstaller(_ checker: Checker) {
        let helper = stubApp + "/Contents/Helpers/stellar-cli"
        let localBin = stubHome + "/.local/bin"
        let link = localBin + "/stellar"

        // 보조 실행 파일 없음.
        do {
            let installer = StellarCommandInstaller(
                appBundlePath: stubApp, fileSystem: makeFileSystem(helper: false), searchPath: defaultSearchPath)
            checker.expect(
                installer.status() == .helperUnavailable(helperPath: helper),
                "helper 없음 상태 불일치: \(installer.status())")
        }

        // 임시·읽기 전용 위치.
        do {
            let translocated = "/private/var/folders/aa/T/AppTranslocation/1234/d/Stellar.app"
            let fileSystem = makeFileSystem()
            fileSystem.addFile(translocated + "/Contents/Helpers/stellar-cli", executable: true)
            let installer = StellarCommandInstaller(
                appBundlePath: translocated, fileSystem: fileSystem, searchPath: defaultSearchPath)
            checker.expect(
                installer.status() == .unsupportedAppLocation(.translocated), "translocation 판정 실패")

            let volume = "/Volumes/Stellar/Stellar.app"
            fileSystem.addFile(volume + "/Contents/Helpers/stellar-cli", executable: true)
            fileSystem.readOnlyVolumePrefixes = ["/Volumes/Stellar"]
            let dmg = StellarCommandInstaller(appBundlePath: volume, fileSystem: fileSystem, searchPath: defaultSearchPath)
            checker.expect(dmg.status() == .unsupportedAppLocation(.readOnlyVolume), "읽기 전용 볼륨 판정 실패")
        }

        // 새 설치 → 멱등 → 제거.
        do {
            let fileSystem = makeFileSystem()
            let installer = StellarCommandInstaller(
                appBundlePath: stubApp, fileSystem: fileSystem, searchPath: defaultSearchPath)
            guard case .ready(let plan) = installer.status() else {
                checker.expect(false, "새 설치 상태가 ready가 아님: \(installer.status())")
                return
            }
            checker.expect(plan.linkPath == link, "기본 링크 위치 불일치: \(plan.linkPath)")
            checker.expect(plan.createsDirectory && !plan.directoryOnSearchPath, "폴더 생성·PATH 판정 불일치")
            checker.expect(plan.action == .create, "새 설치 action 불일치")
            checker.expect(
                installer.install(plan) == .success(StellarCommandInstallation(linkPath: link, directoryOnSearchPath: false)),
                "설치 실패")
            checker.expect(
                fileSystem.nodes[link] == .symbolicLink(destination: helper), "링크 대상 불일치: \(String(describing: fileSystem.nodes[link]))")
            checker.expect(fileSystem.nodes[localBin] == .directory(writable: true), "~/.local/bin 생성 안 됨")
            checker.expect(
                installer.status() == .installed(StellarCommandInstallation(linkPath: link, directoryOnSearchPath: false)),
                "설치 후 상태 불일치")
            checker.expect(installer.install(plan).isSuccess, "같은 계획 재설치가 멱등이 아님")
            let removed = installer.uninstall()
            checker.expect(removed == StellarCommandUninstallResult(removedPaths: [link], failures: []), "제거 결과 불일치: \(removed)")
            checker.expect(fileSystem.nodes[link] == nil, "제거 후 링크가 남음")
            checker.expect(fileSystem.nodes[localBin] != nil, "제거가 사용자 폴더까지 지움")
            if case .ready = installer.status() {} else { checker.expect(false, "제거 후 다시 설치 가능 상태가 아님") }
        }

        // 남의 일반 파일: 덮어쓰지 않고 대체 이름 제안.
        do {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: true)
            fileSystem.addFile(link, executable: true)
            let installer = StellarCommandInstaller(
                appBundlePath: stubApp, fileSystem: fileSystem, searchPath: defaultSearchPath)
            guard case .blocked(let conflict, let fallback?) = installer.status() else {
                checker.expect(false, "남의 파일 상태가 blocked+fallback이 아님: \(installer.status())")
                return
            }
            checker.expect(conflict == link && fallback.commandName == "stellar-cli", "대체 이름 계획 불일치")
            checker.expect(installer.install(fallback).isSuccess, "대체 이름 설치 실패")
            checker.expect(fileSystem.nodes[link] == .file(executable: true), "남의 파일이 바뀜")
            // 기본 이름 계획을 강제로 실행해도 남의 파일은 그대로입니다.
            let forced = StellarCommandInstallPlan(
                linkPath: link, destination: helper, action: .create, createsDirectory: false, directoryOnSearchPath: false)
            checker.expect(installer.install(forced) == .failure(.conflict(path: link)), "남의 파일 덮어쓰기를 막지 못함")
            let removed = installer.uninstall()
            checker.expect(removed.removedPaths == [localBin + "/stellar-cli"], "대체 이름 제거 불일치: \(removed)")
            checker.expect(fileSystem.nodes[link] == .file(executable: true), "제거가 남의 파일을 지움")
        }

        // 남의 심볼릭 링크와 디렉터리: 소유로 보지 않음.
        do {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: true)
            fileSystem.addSymbolicLink(link, destination: "/opt/other/bin/stellar")
            fileSystem.addDirectory(localBin + "/stellar-cli", writable: true)
            let installer = StellarCommandInstaller(
                appBundlePath: stubApp, fileSystem: fileSystem, searchPath: defaultSearchPath)
            checker.expect(!installer.isStellarOwnedLink(atPath: link), "남의 링크를 소유로 판정")
            checker.expect(
                installer.status() == .blocked(conflictPath: link, fallback: nil), "대체 이름도 막힌 상태 불일치")
            checker.expect(installer.uninstall().removedPaths.isEmpty, "남의 링크를 제거함")
            checker.expect(fileSystem.nodes[link] == .symbolicLink(destination: "/opt/other/bin/stellar"), "남의 링크가 바뀜")
        }

        // 옮겨진 앱을 가리키는 Stellar 소유 링크(dangling, 상대 경로 포함): 다시 연결.
        do {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: true)
            let old = "/Users/stub/Downloads/Old Stellar.app/Contents/Helpers/stellar-cli"
            fileSystem.addSymbolicLink(link, destination: old)
            fileSystem.addSymbolicLink(localBin + "/stellar-cli", destination: "../../Apps/Stellar.app/Contents/Helpers/stellar-cli")
            let installer = StellarCommandInstaller(
                appBundlePath: stubApp, fileSystem: fileSystem, searchPath: defaultSearchPath)
            checker.expect(installer.isStellarOwnedLink(atPath: localBin + "/stellar-cli"), "상대 경로 소유 링크 판정 실패")
            guard case .needsRepair(let plan) = installer.status() else {
                checker.expect(false, "오래된 링크 상태가 needsRepair가 아님: \(installer.status())")
                return
            }
            checker.expect(plan.linkPath == link && plan.action == .replaceOwnLink(previousDestination: old), "다시 연결 계획 불일치")
            checker.expect(installer.install(plan).isSuccess, "다시 연결 실패")
            checker.expect(fileSystem.nodes[link] == .symbolicLink(destination: helper), "다시 연결 대상 불일치")
            checker.expect(installer.uninstall().removedPaths.count == 2, "소유 링크 2개를 모두 제거하지 않음")
        }

        // 확인과 실행 사이에 남의 파일이 생긴 경합.
        do {
            let fileSystem = makeFileSystem()
            let installer = StellarCommandInstaller(
                appBundlePath: stubApp, fileSystem: fileSystem, searchPath: defaultSearchPath)
            guard case .ready(let plan) = installer.status() else {
                checker.expect(false, "경합 준비 상태 불일치")
                return
            }
            fileSystem.addDirectory(localBin, writable: true)
            fileSystem.addFile(link, executable: false)
            checker.expect(installer.install(plan) == .failure(.conflict(path: link)), "경합에서 충돌을 보고하지 않음")
            checker.expect(fileSystem.nodes[link] == .file(executable: false), "경합에서 남의 파일이 바뀜")
        }

        // 위치 선택: PATH에 있는 쓰기 가능 폴더 우선, 권한 상승 필요한 폴더 제외.
        do {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: true)
            let onPath = StellarCommandInstaller(
                appBundlePath: stubApp, fileSystem: fileSystem, searchPath: defaultSearchPath + [localBin])
            if case .ready(let plan) = onPath.status() {
                checker.expect(plan.directoryOnSearchPath && !plan.createsDirectory, "PATH의 ~/.local/bin 판정 실패")
            } else {
                checker.expect(false, "PATH의 ~/.local/bin 상태 불일치")
            }

            let usrLocal = makeFileSystem()
            usrLocal.addDirectory("/usr/local/bin", writable: true)
            let preferUsrLocal = StellarCommandInstaller(
                appBundlePath: stubApp, fileSystem: usrLocal, searchPath: defaultSearchPath)
            if case .ready(let plan) = preferUsrLocal.status() {
                checker.expect(plan.linkPath == "/usr/local/bin/stellar", "PATH에 있는 쓰기 가능 폴더 우선 실패: \(plan.linkPath)")
            } else {
                checker.expect(false, "/usr/local/bin 선택 상태 불일치")
            }

            let locked = makeFileSystem()
            locked.addDirectory(stubHome, writable: false)
            let noLocation = StellarCommandInstaller(
                appBundlePath: stubApp, fileSystem: locked, searchPath: defaultSearchPath)
            checker.expect(
                noLocation.status() == .noWritableLocation(suggestedDirectory: localBin), "쓸 수 있는 위치 없음 판정 실패")
        }

        // 파일 시스템 오류 전달.
        do {
            let fileSystem = makeFileSystem()
            let installer = StellarCommandInstaller(
                appBundlePath: stubApp, fileSystem: fileSystem, searchPath: defaultSearchPath)
            guard case .ready(let plan) = installer.status() else { return }
            fileSystem.failNextLinkOperation = "No space left on device"
            checker.expect(
                installer.install(plan)
                    == .failure(.fileSystem(StellarCommandFileSystemError(path: link, message: "No space left on device"))),
                "파일 시스템 오류 전달 실패")
        }

        checker.expect(
            StellarCommandInstaller.pathExportLine(for: localBin, homeDirectory: stubHome)
                == "export PATH=\"$HOME/.local/bin:$PATH\"",
            "PATH 안내 줄 불일치")
        checker.expect(
            StellarCommandInstaller.lexicallyNormalized("/a//b/./c/../d/") == "/a/b/d", "경로 정규화 불일치")
    }

    static func checkSearchPath(_ checker: Checker) {
        let fileSystem = StellarCommandFileSystemStub(homeDirectory: stubHome)
        fileSystem.textFiles["/etc/paths"] = "/usr/local/bin\n/usr/bin\n/bin\n# comment\n"
        fileSystem.textFiles["/etc/paths.d/10-cryptex"] = "/System/Cryptexes/App/usr/bin\n"
        let path = StellarCommandSearchPath.system(fileSystem: fileSystem, environmentPath: "~/.local/bin:/usr/bin:relative")
        checker.expect(
            path == ["/usr/local/bin", "/usr/bin", "/bin", "/System/Cryptexes/App/usr/bin", stubHome + "/.local/bin"],
            "PATH 구성 불일치: \(path)")
    }

    static func checkFirstLaunchPolicy(_ checker: Checker) {
        let suite = "xyz.taeyoon.stellar.welcome-ui-check.\(UUID().uuidString)"
        guard let defaults = UserDefaults(suiteName: suite) else {
            checker.expect(false, "검사용 UserDefaults 생성 실패")
            return
        }
        defer { defaults.removePersistentDomain(forName: suite) }
        let policy = WelcomeFirstLaunchPolicy(defaults: defaults)
        let plan = StellarCommandInstallPlan(
            linkPath: "/Users/stub/.local/bin/stellar", destination: "/x.app/Contents/Helpers/stellar-cli",
            action: .create, createsDirectory: true, directoryOnSearchPath: false)
        checker.expect(policy.shouldOfferCommandInstall(for: .ready(plan)), "첫 실행 제안이 꺼져 있음")
        checker.expect(
            !policy.shouldOfferCommandInstall(for: .helperUnavailable(helperPath: "/x")), "helper 없는 빌드에서 제안함")
        checker.expect(
            !policy.shouldOfferCommandInstall(for: .blocked(conflictPath: "/x", fallback: nil)), "막다른 상태에서 제안함")
        policy.recordCommandInstallOffered()
        checker.expect(!policy.shouldOfferCommandInstall(for: .ready(plan)), "답한 뒤에도 다시 제안함")
    }

    // MARK: 시작 창

    struct WelcomeFixture {
        let controller: WelcomeWindowController
        let welcome: WelcomeViewController
        let stub: WelcomeDataSourceStub
        let window: NSWindow
    }

    static func makeWelcome(_ scenario: WelcomeDataSourceStub.Scenario) -> WelcomeFixture? {
        // 경로 줄임(~)을 확인하려고 실제 홈 아래 가상 경로를 씁니다. 그 경로는 열거나 읽지 않습니다.
        let stub = WelcomeDataSourceStub(scenario: scenario, homeDirectory: NSHomeDirectory())
        let controller = WelcomeWindowController(dataSource: stub, appVersion: "1.2.3")
        guard let window = controller.window else { return nil }
        window.isReleasedWhenClosed = false
        window.contentView?.layoutSubtreeIfNeeded()
        return WelcomeFixture(controller: controller, welcome: controller.welcomeViewController, stub: stub, window: window)
    }

    /// 왼쪽 동작 열이 safe area 안에 잘리지 않고 들어가는지.
    static func actionsColumnFits(_ controller: WelcomeViewController) -> Bool {
        let root = controller.view
        root.layoutSubtreeIfNeeded()
        let safe = root.safeAreaRect.insetBy(dx: 0, dy: 0)
        let frame = controller.actionsColumn.frame
        return safe.contains(frame) && frame.height + 0.5 >= controller.actionsColumn.fittingSize.height
    }

    static func checkWelcomeWindow(_ checker: Checker) {
        // 표본: 목록, 키보드, 문맥 동작.
        guard let fixture = makeWelcome(.sample) else {
            checker.expect(false, "시작 창 생성 실패")
            return
        }
        let welcome = fixture.welcome, stub = fixture.stub, window = fixture.window
        checker.expect(window.title == WelcomeStrings.windowTitle, "창 제목 불일치")
        checker.expect(window.styleMask.contains(.titled) && window.styleMask.contains(.closable), "표준 창 스타일 아님")
        checker.expect(window.contentView?.frame.size == WelcomeMetrics.windowContentSize, "창 크기 불일치")
        expectCleanLayout(checker, window: window, context: "welcome/sample")
        checker.expect(actionsColumnFits(welcome), "동작 열이 잘림(sample)")
        let table = welcome.recentsView.tableView
        checker.expect(window.initialFirstResponder === table, "초기 포커스가 최근 목록이 아님")
        checker.expect(!welcome.recentsView.scrollView.isHidden && welcome.recentsView.stateStack.isHidden, "목록 표시 상태 불일치")
        checker.expect(table.numberOfRows == 4, "최근 행 수 불일치: \(table.numberOfRows)")
        checker.expect(table.accessibilityLabel() == WelcomeStrings.recentProjects, "목록 접근성 레이블 없음")
        checker.expect(table.acceptsFirstResponder, "목록이 키보드 포커스를 받지 않음")
        for button in [welcome.newProjectButton, welcome.openProjectButton, welcome.cloneProjectButton] {
            checker.expect(!button.title.isEmpty && button.isEnabled, "동작 버튼 '\(button.title)' 상태 불일치")
            checker.expect(button.image != nil, "동작 버튼 '\(button.title)' SF Symbol 없음")
        }
        checker.expect(
            welcome.newProjectButton.keyEquivalent == "n" && welcome.newProjectButton.keyEquivalentModifierMask == .command,
            "새 프로젝트 단축키 불일치")
        checker.expect(
            welcome.openProjectButton.keyEquivalent == "o" && welcome.openProjectButton.keyEquivalentModifierMask == .command,
            "열기 단축키 불일치")
        checker.expect(welcome.gitNoticeView.isHidden && welcome.activityView.isHidden, "안내·진행 표시가 보임")
        checker.expect(welcome.versionLabel.stringValue.contains("1.2.3"), "버전 표시 불일치")

        let first = table.view(atColumn: 0, row: 0, makeIfNecessary: true) as? WelcomeRecentProjectCellView
        let missing = table.view(atColumn: 0, row: 3, makeIfNecessary: true) as? WelcomeRecentProjectCellView
        checker.expect(first?.accessibilityLabel()?.contains("Design System") == true, "행 접근성 레이블 없음")
        checker.expect(first?.pathLabel.stringValue.hasPrefix("~/") == true, "경로가 ~로 줄지 않음: \(first?.pathLabel.stringValue ?? "")")
        checker.expect(
            missing?.accessibilityLabel()?.contains(WelcomeStrings.notFound) == true, "없는 프로젝트 상태가 레이블에 없음")
        checker.expect((first?.accessibilityCustomActions()?.count ?? 0) == 3, "행 접근성 사용자 동작 수 불일치")
        checker.expect((missing?.accessibilityCustomActions()?.count ?? 0) == 2, "없는 행 접근성 사용자 동작 수 불일치")
        if let first { checker.expect(layoutProblems(in: first, context: "cell").isEmpty, "행 셀 레이아웃 문제") }

        // Return으로 열기.
        table.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        if let returnKey = keyEvent(36, characters: "\r", window: window) {
            table.keyDown(with: returnKey)
        }
        checker.expect(stub.sentCommands.last == .openRecent(id: "stub-1"), "Return이 최근 프로젝트를 열지 않음")

        // Delete로 목록에서 빼기(없는 프로젝트 행).
        table.selectRowIndexes(IndexSet(integer: 3), byExtendingSelection: false)
        if let deleteKey = keyEvent(51, characters: "\u{7f}", window: window) {
            table.keyDown(with: deleteKey)
        }
        checker.expect(stub.sentCommands.last == .removeRecent(id: "stub-4"), "Delete가 목록에서 빼지 않음")
        checker.expect(spin(until: { table.numberOfRows == 3 }), "snapshot 변경이 다음 런 루프에서 반영되지 않음")

        // 문맥 메뉴 항목 활성 상태.
        if let menu = table.menu {
            table.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
            welcome.recentsView.menuNeedsUpdate(menu)
            checker.expect(menu.items.filter { !$0.isSeparatorItem }.allSatisfy(\.isEnabled), "문맥 메뉴 비활성")
        } else {
            checker.expect(false, "문맥 메뉴 없음")
        }

        // snapshot 반영 시간(보고만 함, 아키텍처 §6.3 UI 스레드 2ms 상한 참고).
        stub.replaceSnapshot { $0.git = .notFound }
        let start = DispatchTime.now().uptimeNanoseconds
        welcome.renderNow()
        let micros = Double(DispatchTime.now().uptimeNanoseconds - start) / 1000
        checker.note(String(format: "render %.0fus", micros))
        checker.expect(!welcome.gitNoticeView.isHidden, "Git 안내가 반영되지 않음")
        window.close()

        // 빈 상태.
        if let fixture = makeWelcome(.empty) {
            let welcome = fixture.welcome, window = fixture.window
            let recents = welcome.recentsView
            checker.expect(recents.scrollView.isHidden && !recents.stateStack.isHidden, "빈 상태 표시 불일치")
            checker.expect(recents.stateTitleLabel.stringValue == WelcomeStrings.noRecentProjects, "빈 상태 제목 불일치")
            checker.expect(!recents.stateMessageLabel.isHidden && recents.retryButton.isHidden, "빈 상태 구성 불일치")
            checker.expect(window.initialFirstResponder !== recents.tableView, "빈 목록에 초기 포커스를 줌")
            expectCleanLayout(checker, window: window, context: "welcome/empty")
            window.close()
        }

        // 읽는 중.
        if let fixture = makeWelcome(.loading) {
            let welcome = fixture.welcome, window = fixture.window
            let recents = welcome.recentsView
            checker.expect(!recents.stateSpinner.isHidden, "읽는 중 표시 없음")
            checker.expect(recents.stateTitleLabel.stringValue == WelcomeStrings.loadingRecentProjects, "읽는 중 제목 불일치")
            checker.expect(welcome.gitNoticeView.isHidden, "Git 탐색 중에 안내를 띄움")
            checker.expect(welcome.newProjectButton.isEnabled, "Git 탐색 중에 새 프로젝트를 막음")
            expectCleanLayout(checker, window: window, context: "welcome/loading")
            recents.apply(.loaded([]))
            window.close()
        }

        // 오류 → 다시 시도.
        if let fixture = makeWelcome(.failed) {
            let welcome = fixture.welcome, stub = fixture.stub, window = fixture.window
            let recents = welcome.recentsView
            checker.expect(!recents.retryButton.isHidden, "오류 상태에 다시 시도 버튼 없음")
            checker.expect(recents.stateTitleLabel.stringValue == WelcomeStrings.recentsFailed, "오류 제목 불일치")
            expectCleanLayout(checker, window: window, context: "welcome/failed")
            recents.retryButton.performClick(nil)
            checker.expect(stub.sentCommands.last == .reloadRecentProjects, "다시 시도 명령 없음")
            welcome.renderNow()
            checker.expect(!recents.scrollView.isHidden && recents.tableView.numberOfRows == 4, "다시 시도 후 목록 없음")
            window.close()
        }

        // Git 없음.
        if let fixture = makeWelcome(.gitMissing) {
            let welcome = fixture.welcome, stub = fixture.stub, window = fixture.window
            let notice = welcome.gitNoticeView
            checker.expect(!notice.isHidden, "Git 없음 안내가 숨겨짐")
            checker.expect(notice.titleLabel.stringValue == WelcomeStrings.gitNotFoundTitle, "Git 안내 제목 불일치")
            checker.expect(notice.accessibilityLabel() == WelcomeStrings.gitNotFoundTitle, "Git 안내 접근성 레이블 없음")
            checker.expect(notice.commandLabel.stringValue == "xcode-select --install", "설치 안내 명령 불일치")
            checker.expect(notice.commandLabel.isSelectable, "설치 안내 명령을 선택할 수 없음")
            checker.expect(
                notice.copyButton.accessibilityLabel() == WelcomeStrings.copyCommand && notice.copyButton.toolTip != nil,
                "아이콘 버튼 접근성 레이블·도움말 없음")
            checker.expect(
                !welcome.newProjectButton.isEnabled && !welcome.cloneProjectButton.isEnabled
                    && welcome.openProjectButton.isEnabled,
                "Git 없음 버튼 활성 상태 불일치")
            checker.expect(welcome.newProjectButton.toolTip == WelcomeStrings.requiresGit, "비활성 이유 도움말 없음")
            expectCleanLayout(checker, window: window, context: "welcome/gitMissing")
            checker.expect(actionsColumnFits(welcome), "동작 열이 잘림(gitMissing)")
            notice.checkAgainButton.performClick(nil)
            checker.expect(stub.sentCommands.last == .recheckGit, "다시 검사 명령 없음")

            // 가장 긴 경우: Git 없음 + 진행 중 + 쓸 수 없는 Git 설명.
            stub.replaceSnapshot {
                $0.git = .unusable(detail: "Git 2.20 is too old. Stellar needs a newer version of Git to read project status.")
                $0.activity = WelcomeActivity(kind: .opening, projectName: "A Project With A Long Name", fractionCompleted: nil, isCancellable: true)
            }
            welcome.renderNow()
            checker.expect(notice.commandRow.isHidden, "쓸 수 없는 Git에 설치 명령을 보여 줌")
            checker.expect(!welcome.openProjectButton.isEnabled, "진행 중에 열기를 막지 않음")
            expectCleanLayout(checker, window: window, context: "welcome/gitUnusable+activity")
            checker.expect(actionsColumnFits(welcome), "동작 열이 잘림(최대 내용)")
            welcome.activityView.stop()
            window.close()
        }

        // Clone 진행 중 → 취소.
        if let fixture = makeWelcome(.cloning) {
            let welcome = fixture.welcome, stub = fixture.stub, window = fixture.window
            let activity = welcome.activityView
            checker.expect(!activity.isHidden && !activity.cancelButton.isHidden, "진행 표시 없음")
            checker.expect(
                !activity.progressIndicator.isIndeterminate && abs(activity.progressIndicator.doubleValue - 0.4) < 0.001,
                "진행률 불일치")
            checker.expect(activity.accessibilityLabel()?.contains("design-system") == true, "진행 접근성 레이블 없음")
            checker.expect(
                ![welcome.newProjectButton, welcome.openProjectButton, welcome.cloneProjectButton].contains(where: \.isEnabled),
                "진행 중 동작 버튼이 켜져 있음")
            expectCleanLayout(checker, window: window, context: "welcome/cloning")
            activity.cancelButton.performClick(nil)
            checker.expect(stub.sentCommands.last == .cancelActivity, "취소 명령 없음")
            welcome.renderNow()
            checker.expect(activity.isHidden && welcome.openProjectButton.isEnabled, "취소 후 상태 불일치")
            window.close()
        }
    }

    static func checkNoticeAlerts(_ checker: Checker) {
        let kinds: [WelcomeNotice.Kind] = [
            .notStellarProject(path: "/Users/stub/Projects/plain-repo"),
            .projectMissing(path: "/Users/stub/Projects/gone"),
            .folderAlreadyExists(path: "/Users/stub/Projects/design-system"),
            .gitIdentityRequired,
            .operationFailed(detail: "Clone failed."),
        ]
        for (index, kind) in kinds.enumerated() {
            let alert = WelcomeViewController.makeAlert(for: WelcomeNotice(id: UInt64(index), kind: kind))
            checker.expect(
                !alert.messageText.isEmpty && !alert.informativeText.isEmpty && alert.buttons.count == 1,
                "안내 alert 구성 불일치: \(kind)")
        }
        let notStellar = WelcomeViewController.makeAlert(
            for: WelcomeNotice(id: 9, kind: .notStellarProject(path: "/Users/stub/Projects/plain-repo")))
        checker.expect(notStellar.informativeText.contains("stellar.yaml"), "stellar.yaml 안내 없음")
    }

    // MARK: Clone sheet

    static func checkCloneSheet(_ checker: Checker) {
        let sheet = WelcomeCloneSheetController(defaultParentDirectory: stubHome + "/Documents")
        let window = offscreenWindow(for: sheet)
        defer { window.close() }
        expectCleanLayout(checker, window: window, context: "clone")
        checker.expect(!sheet.cloneButton.isEnabled, "빈 입력에서 Clone이 켜짐")
        checker.expect(sheet.cloneButton.keyEquivalent == "\r" && sheet.cancelButton.keyEquivalent == "\u{1b}", "Clone sheet 키 불일치")
        checker.expect(
            sheet.urlField.accessibilityLabel() == WelcomeStrings.repositoryURL
                && sheet.nameField.accessibilityLabel() == WelcomeStrings.folderName,
            "Clone 입력 접근성 레이블 없음")
        checker.expect(sheet.urlField.isEditable && sheet.nameField.isEditable, "Clone 입력이 네이티브 편집 필드가 아님")

        sheet.simulateInput(url: "https://example.com/team/design-system.git")
        checker.expect(sheet.nameField.stringValue == "design-system", "URL에서 폴더 이름을 만들지 못함")
        let expected = WelcomeCloneRequest(
            remoteURL: "https://example.com/team/design-system.git",
            parentDirectory: stubHome + "/Documents",
            folderName: "design-system")
        checker.expect(sheet.request == expected && sheet.cloneButton.isEnabled, "Clone 요청 불일치")
        var received: WelcomeCloneRequest?
        sheet.onClone = { received = $0 }
        sheet.cloneButton.performClick(nil)
        checker.expect(received == expected, "Clone 명령이 전달되지 않음")

        sheet.simulateInput(url: "not a url")
        checker.expect(!sheet.cloneButton.isEnabled, "공백 있는 URL을 허용함")
        checker.expect(
            WelcomeCloneSheetController.suggestedFolderName(forRemoteURL: "git@example.com:team/ds.git") == "ds"
                && WelcomeCloneSheetController.suggestedFolderName(forRemoteURL: "https://example.com/x/") == "x"
                && WelcomeCloneSheetController.suggestedFolderName(forRemoteURL: "https://example.com/.git").isEmpty,
            "폴더 이름 추천 불일치")
        checker.expect(
            !WelcomeCloneSheetController.isValidFolderName(".hidden")
                && !WelcomeCloneSheetController.isValidFolderName("a/b")
                && WelcomeCloneSheetController.isValidFolderName("Design System"),
            "폴더 이름 규칙 불일치")
    }

    // MARK: 명령 설치 sheet

    static func checkInstallSheet(_ checker: Checker) {
        let fileSystem = makeFileSystem()
        let installer = StellarCommandInstaller(appBundlePath: stubApp, fileSystem: fileSystem, searchPath: defaultSearchPath)
        guard case .ready(let plan) = installer.status() else {
            checker.expect(false, "설치 sheet 준비 상태 불일치")
            return
        }
        let sheet = WelcomeCommandInstallSheetController(installer: installer, status: .ready(plan))
        var responses: [WelcomeCommandInstallSheetController.Response] = []
        sheet.onResponse = { responses.append($0) }
        let window = offscreenWindow(for: sheet)
        defer { window.close() }

        expectCleanLayout(checker, window: window, context: "install/ready")
        checker.expect(sheet.titleLabel.stringValue == WelcomeStrings.commandInstallTitle, "설치 제안 제목 불일치")
        checker.expect(
            sheet.primaryButton.title == WelcomeStrings.install && sheet.primaryButton.keyEquivalent == "\r",
            "Install 기본 버튼 불일치")
        checker.expect(
            sheet.secondaryButton.title == WelcomeStrings.notNow && sheet.secondaryButton.keyEquivalent == "\u{1b}",
            "Not Now 버튼 불일치")
        checker.expect(sheet.destructiveButton.isHidden, "설치 전 제거 버튼이 보임")
        checker.expect(sheet.linkValueLabel.stringValue == "~/.local/bin/stellar", "링크 위치 표시 불일치: \(sheet.linkValueLabel.stringValue)")
        checker.expect(!sheet.pathLineRow.isHidden && sheet.pathLineLabel.stringValue.contains("$HOME/.local/bin"), "PATH 안내 없음")
        checker.expect(sheet.pathCopyButton.accessibilityLabel() != nil && sheet.pathCopyButton.toolTip != nil, "PATH 복사 버튼 레이블 없음")
        checker.expect(fileSystem.nodes[plan.linkPath] == nil, "동의 전에 설치함")

        // 동의 → 설치(작업 큐) → 결과.
        sheet.primaryButton.performClick(nil)
        let installed = spin(until: {
            if case .installed = sheet.phase { return true }
            return false
        })
        checker.expect(installed, "설치 결과 단계로 가지 않음: \(sheet.phase)")
        checker.expect(fileSystem.nodes[plan.linkPath] == .symbolicLink(destination: installer.helperPath), "sheet 설치 결과 불일치")
        checker.expect(
            !sheet.destructiveButton.isHidden && sheet.destructiveButton.hasDestructiveAction
                && sheet.destructiveButton.keyEquivalent.isEmpty,
            "제거 버튼이 파괴적 표시·비기본이 아님")
        checker.expect(sheet.primaryButton.title == WelcomeStrings.done, "설치 후 Done 없음")
        expectCleanLayout(checker, window: window, context: "install/installed")

        // 제거.
        sheet.destructiveButton.performClick(nil)
        let removed = spin(until: {
            if case .removed = sheet.phase { return true }
            return false
        })
        checker.expect(removed, "제거 결과 단계로 가지 않음: \(sheet.phase)")
        checker.expect(fileSystem.nodes[plan.linkPath] == nil, "sheet 제거 결과 불일치")
        checker.expect(sheet.titleLabel.stringValue == WelcomeStrings.removedTitle, "제거 제목 불일치")
        sheet.primaryButton.performClick(nil)
        checker.expect(responses == [.removed], "sheet 응답 불일치: \(responses)")

        // 그 밖의 단계: 모두 제목과 닫을 방법이 있고 레이아웃이 깨지지 않아야 합니다.
        let otherPhases: [(String, WelcomeCommandInstallSheetController.Phase)] = [
            ("helperUnavailable", .status(.helperUnavailable(helperPath: installer.helperPath))),
            ("translocated", .status(.unsupportedAppLocation(.translocated))),
            ("blockedFallback", .status(.blocked(conflictPath: plan.linkPath, fallback: plan))),
            ("blocked", .status(.blocked(conflictPath: plan.linkPath, fallback: nil))),
            ("needsRepair", .status(.needsRepair(plan))),
            ("noLocation", .status(.noWritableLocation(suggestedDirectory: plan.directory))),
            ("installedManage", .status(.installed(StellarCommandInstallation(linkPath: plan.linkPath, directoryOnSearchPath: true)))),
            ("conflict", .installFailed(.conflict(path: plan.linkPath), plan: plan)),
            ("fsError", .installFailed(.fileSystem(StellarCommandFileSystemError(path: plan.linkPath, message: "Permission denied")), plan: plan)),
            ("removeFailed", .removed(StellarCommandUninstallResult(removedPaths: [], failures: [StellarCommandFileSystemError(path: plan.linkPath, message: "Permission denied")]))),
        ]
        for (name, phase) in otherPhases {
            sheet.show(phase)
            checker.expect(!sheet.titleLabel.stringValue.isEmpty, "install/\(name): 제목 없음")
            checker.expect(!sheet.primaryButton.isHidden && sheet.primaryButton.keyEquivalent == "\r", "install/\(name): 기본 버튼 없음")
            expectCleanLayout(checker, window: window, context: "install/\(name)")
        }
        sheet.show(.status(.installed(StellarCommandInstallation(linkPath: plan.linkPath, directoryOnSearchPath: true))))
        checker.expect(sheet.pathLineRow.isHidden, "PATH에 있는 폴더에 PATH 안내를 보여 줌")
        sheet.show(.status(.blocked(conflictPath: plan.linkPath, fallback: plan)))
        checker.expect(sheet.primaryButton.title == WelcomeStrings.installAs("stellar"), "대체 이름 버튼 제목 불일치")

        sheet.show(.working(removing: false))
        checker.expect(
            !sheet.spinner.isHidden && sheet.primaryButton.isHidden && sheet.secondaryButton.isHidden,
            "작업 중 표시 불일치")
        expectCleanLayout(checker, window: window, context: "install/working")
        sheet.cancelOperation(nil)
        checker.expect(responses == [.removed], "작업 중 Escape가 sheet를 닫음")
        sheet.show(.status(.helperUnavailable(helperPath: installer.helperPath)))
    }
}

private extension Result {
    var isSuccess: Bool {
        if case .success = self { return true }
        return false
    }
}
