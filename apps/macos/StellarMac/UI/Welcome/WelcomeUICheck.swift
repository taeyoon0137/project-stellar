import AppKit

/// 시작 창 영역을 headless로 검증합니다(통합 시 `scripts/macos/smoke.sh`에 연결).
///
/// - 시작 창·Clone sheet·명령 설치 sheet를 Stub 데이터로 화면 밖에서 만들고 레이아웃을 강제한 뒤,
///   모호한 Auto Layout, 잘린 여러 줄 레이블, 직접 그린 반투명 표면이 없는지 확인합니다.
/// - 주요 접근성 레이블, 키보드 경로(Return·Delete), 상태별 표시(빈·읽는 중·오류·Git 없음·진행 중)를 확인합니다.
/// - `StellarCommandInstaller`를 메모리 파일 시스템(`StellarCommandFileSystemStub`)으로 검사합니다(XXIV.5.3~XXIV.5.5
///   fixture): 위치 결정, 대체 이름 규칙(설치 위치·PATH 앞쪽·순서 모르는 폴더), 설치·멱등·제거, 남의 파일 보존,
///   다시 연결, 확인과 실행 사이 경합, 동의 없는 파일 변경 0, UI 경로의 메인 스레드 파일 조회 0.
/// - 실제 파일 시스템 구현은 임시 폴더 안에서만 검사합니다(RENAME_SWAP 교체, symlink EEXIST, 경합 시 원래 항목 보존).
/// - Light, Dark, 대비 높이기(Light·Dark) 외형에서 같은 레이아웃 검사를 반복합니다.
/// 사용자 위치(~/.local/bin 등)에는 쓰지 않으며, 시스템 sheet·패널은 띄우지 않습니다.
enum WelcomeUICheck {
    static func run() -> Bool {
        let checker = Checker()
        let application = NSApplication.shared
        application.setActivationPolicy(.accessory)

        checkInstallLocation(checker)
        checkInstaller(checker)
        checkSearchPath(checker)
        checkInstallerOnDisk(checker)
        checkInstallerThreading(checker)
        checkFirstLaunchPolicy(checker)
        checkWelcomeWindow(checker)
        checkNoticeAlerts(checker)
        checkCloneSheet(checker)
        checkInstallSheet(checker)
        checkAccessibilityFeedback(checker)

        // 실제 앱 번들 상태는 환경마다 다르므로 보고만 합니다(XXIV.5.1 전 빌드에는 helper가 없음).
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

    /// 컨트롤에 지정된 색. 시스템 의미 색(catalog 색)이 아니면 하드코딩 색입니다(가이드 4.2).
    static func assignedColors(of view: NSView) -> [(String, NSColor)] {
        var colors: [(String, NSColor)] = []
        if let field = view as? NSTextField {
            if let color = field.textColor { colors.append(("textColor", color)) }
            if field.drawsBackground, let color = field.backgroundColor { colors.append(("backgroundColor", color)) }
        }
        if let image = view as? NSImageView, let color = image.contentTintColor { colors.append(("contentTintColor", color)) }
        if let button = view as? NSButton {
            if let color = button.contentTintColor { colors.append(("contentTintColor", color)) }
            if let color = button.bezelColor { colors.append(("bezelColor", color)) }
        }
        if let box = view as? NSBox, box.boxType == .custom {
            colors.append(("fillColor", box.fillColor))
            colors.append(("borderColor", box.borderColor))
        }
        return colors
    }

    /// 레이아웃 문제(모호한 제약, 세로로 잘린 여러 줄 레이블, 직접 넣은 반투명 표면, 하드코딩 색)를 찾습니다.
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
            for (name, color) in assignedColors(of: view) where color.type != .catalog {
                problems.append("\(context): 하드코딩 색 \(type(of: view)).\(name) \(color)")
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
        case .shadowed(let installation, _, _): return "shadowed(\(installation.commandName))"
        case .namesUnavailable: return "namesUnavailable"
        case .noWritableLocation: return "noWritableLocation"
        }
    }

    // MARK: 설치기(XXIV.5.3~XXIV.5.5 fixture, 판정 정의는 docs/design/ui/welcome.md 6장)

    static let stubHome = "/Users/stub"
    static let stubApp = "/Applications/Stellar.app"
    static let ownerBundleID = "xyz.taeyoon.stellar"
    static let stubHelper = stubApp + "/Contents/Helpers/stellar"
    static let localBin = stubHome + "/.local/bin"
    static let homeBin = stubHome + "/bin"

    /// helper가 있는 앱과 쓰기 불가 `/usr/local/bin`이 있는 표준 메모리 파일 시스템.
    static func makeFileSystem(helper: Bool = true) -> StellarCommandFileSystemStub {
        let fileSystem = StellarCommandFileSystemStub(homeDirectory: stubHome)
        fileSystem.addDirectory("/usr/local/bin", writable: false)
        fileSystem.addDirectory("/Applications", writable: false)
        if helper {
            fileSystem.addFile(stubHelper, executable: true)
        } else {
            fileSystem.addDirectory(stubApp + "/Contents/MacOS", writable: false)
        }
        fileSystem.bundleIdentifiers[stubApp] = ownerBundleID
        return fileSystem
    }

    static let defaultSearchPath = StellarCommandSearchPath(ordered: ["/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"])

    static func makeInstaller(
        _ fileSystem: StellarCommandFileSystem,
        app: String = stubApp,
        searchPath: StellarCommandSearchPath = defaultSearchPath
    ) -> StellarCommandInstaller {
        StellarCommandInstaller(
            appBundlePath: app, fileSystem: fileSystem, searchPath: searchPath, ownerBundleIdentifier: ownerBundleID)
    }

    static func readyPlan(_ installer: StellarCommandInstaller) -> StellarCommandInstallPlan? {
        if case .ready(let plan) = installer.status() { return plan }
        return nil
    }

    /// fixture 하나를 실행합니다. 안에서 `return`하면 그 fixture만 끝나고 다음 fixture는 계속 실행됩니다.
    static func fixture(_ body: () -> Void) {
        body()
    }

    /// 위치 결정(XXIV.5.3) fixture L1~L8.
    static func checkInstallLocation(_ checker: Checker) {
        // L1: PATH에 홈 후보 없음, ~/.local/bin 없음 → ~/.local/bin을 만들어 설치, PATH 안내.
        fixture {
            let plan = readyPlan(makeInstaller(makeFileSystem()))
            checker.expect(
                plan?.linkPath == localBin + "/stellar" && plan?.createsDirectory == true
                    && plan?.directoryOnSearchPath == false && plan?.primaryNameConflict == nil,
                "L1 기본 위치 불일치: \(String(describing: plan))")
        }
        // L2: PATH에 있는 ~/.local/bin.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: true)
            let plan = readyPlan(makeInstaller(fileSystem, searchPath: StellarCommandSearchPath(ordered: [localBin, "/usr/bin"])))
            checker.expect(
                plan?.linkPath == localBin + "/stellar" && plan?.directoryOnSearchPath == true && plan?.createsDirectory == false,
                "L2 PATH의 ~/.local/bin 판정 실패: \(String(describing: plan))")
        }
        // L3: PATH에 있는 쪽(~/bin)을 PATH에 없는 첫 후보보다 먼저 고름.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(homeBin, writable: true)
            let plan = readyPlan(makeInstaller(fileSystem, searchPath: StellarCommandSearchPath(ordered: ["/usr/bin", homeBin])))
            checker.expect(plan?.linkPath == homeBin + "/stellar" && plan?.directoryOnSearchPath == true,
                "L3 PATH 우선 선택 실패: \(String(describing: plan))")
        }
        // L4: 쓸 수 있고 PATH에 있는 /usr/local/bin이 있어도 여러 사용자 공용 폴더는 고르지 않음(§37 최소 권한).
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory("/usr/local/bin", writable: true)
            let plan = readyPlan(makeInstaller(fileSystem))
            checker.expect(plan?.linkPath == localBin + "/stellar", "L4 공용 폴더를 고름: \(String(describing: plan))")
        }
        // L5: 홈에 쓸 수 없음 → 쓸 수 있는 위치 없음.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(stubHome, writable: false)
            let status = makeInstaller(fileSystem).status()
            checker.expect(status == .noWritableLocation(suggestedDirectory: localBin), "L5 위치 없음 판정 실패: \(status)")
        }
        // L6: ~/.local/bin은 있지만 쓸 수 없음 → ~/bin을 만들어 설치.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: false)
            let plan = readyPlan(makeInstaller(fileSystem))
            checker.expect(plan?.linkPath == homeBin + "/stellar" && plan?.createsDirectory == true,
                "L6 다음 후보 선택 실패: \(String(describing: plan))")
        }
        // L7: App Translocation·읽기 전용 볼륨.
        fixture {
            let translocated = "/private/var/folders/aa/T/AppTranslocation/1234/d/Stellar.app"
            let fileSystem = makeFileSystem()
            fileSystem.addFile(translocated + "/Contents/Helpers/stellar", executable: true)
            checker.expect(
                makeInstaller(fileSystem, app: translocated).status() == .unsupportedAppLocation(.translocated),
                "L7 translocation 판정 실패")
            let volume = "/Volumes/Stellar/Stellar.app"
            fileSystem.addFile(volume + "/Contents/Helpers/stellar", executable: true)
            fileSystem.readOnlyVolumePrefixes = ["/Volumes/Stellar"]
            checker.expect(
                makeInstaller(fileSystem, app: volume).status() == .unsupportedAppLocation(.readOnlyVolume),
                "L7 읽기 전용 볼륨 판정 실패")
        }
        // L8: helper(`Contents/Helpers/stellar`) 없음.
        fixture {
            let status = makeInstaller(makeFileSystem(helper: false)).status()
            checker.expect(status == .helperUnavailable(helperPath: stubHelper), "L8 helper 없음 상태 불일치: \(status)")
            checker.expect(StellarCommandInstaller.helperRelativePath == "Contents/Helpers/stellar", "helper 경로가 ADR-0005와 다름")
        }
    }

    /// 대체 이름 규칙·설치·제거(XXIV.5.4) fixture N1~N10, R1~R5, E1.
    static func checkInstaller(_ checker: Checker) {
        let link = localBin + "/stellar"
        let fallbackLink = localBin + "/stellar-cli"

        // I1: 새 설치 → 멱등 → 제거. 판정만으로는 파일을 바꾸지 않음(동의 없는 변경 0).
        fixture {
            let fileSystem = makeFileSystem()
            let installer = makeInstaller(fileSystem)
            guard let plan = readyPlan(installer) else {
                checker.expect(false, "I1 새 설치 상태가 ready가 아님: \(installer.status())")
                return
            }
            checker.expect(fileSystem.mutationCount == 0, "I1 판정이 파일을 바꿈")
            checker.expect(plan.action == .create && plan.destination == stubHelper, "I1 계획 불일치")
            checker.expect(
                installer.install(plan) == .success(StellarCommandInstallation(linkPath: link, directoryOnSearchPath: false)),
                "I1 설치 실패")
            checker.expect(fileSystem.nodes[link] == .symbolicLink(destination: stubHelper), "I1 링크 대상 불일치")
            checker.expect(fileSystem.nodes[localBin] == .directory(writable: true), "I1 ~/.local/bin 생성 안 됨")
            checker.expect(
                installer.status() == .installed(StellarCommandInstallation(linkPath: link, directoryOnSearchPath: false)),
                "I1 설치 후 상태 불일치")
            let before = fileSystem.mutationCount
            checker.expect(installer.install(plan).isSuccess && fileSystem.mutationCount == before, "I1 재설치가 멱등이 아님")
            let removed = installer.uninstall()
            checker.expect(removed == StellarCommandUninstallResult(removedPaths: [link], failures: []), "I1 제거 결과 불일치: \(removed)")
            checker.expect(fileSystem.nodes[link] == nil && installer.installedLinkPaths().isEmpty, "I1 되돌리기 후 설치 항목이 남음")
            checker.expect(fileSystem.nodes[localBin] != nil, "I1 제거가 사용자 폴더까지 지움")
            checker.expect(readyPlan(installer) != nil, "I1 제거 후 다시 설치 가능 상태가 아님")
        }

        // N1: 설치 위치에 남의 일반 파일 → 덮어쓰기 0, stellar-cli 설치.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: true)
            fileSystem.addFile(link, executable: true)
            let installer = makeInstaller(fileSystem)
            guard let plan = readyPlan(installer) else {
                checker.expect(false, "N1 상태가 ready(stellar-cli)가 아님: \(installer.status())")
                return
            }
            checker.expect(
                plan.linkPath == fallbackLink
                    && plan.primaryNameConflict == StellarCommandNameConflict(commandName: "stellar", path: link, position: .atInstallLocation),
                "N1 대체 이름 계획 불일치: \(plan)")
            checker.expect(installer.install(plan).isSuccess, "N1 대체 이름 설치 실패")
            checker.expect(fileSystem.nodes[link] == .file(executable: true), "N1 남의 파일이 바뀜")
            checker.expect(
                installer.status() == .installed(StellarCommandInstallation(linkPath: fallbackLink, directoryOnSearchPath: false)),
                "N1 설치 후 이름이 stellar-cli가 아님: \(installer.status())")
            let forced = StellarCommandInstallPlan(
                linkPath: link, destination: stubHelper, action: .create, createsDirectory: false,
                directoryOnSearchPath: false, primaryNameConflict: nil, hiddenProgramPaths: [])
            checker.expect(installer.install(forced) == .failure(.conflict(path: link)), "N1 남의 파일 덮어쓰기를 막지 못함")
            let removed = installer.uninstall()
            checker.expect(removed.removedPaths == [fallbackLink], "N1 대체 이름 제거 불일치: \(removed)")
            checker.expect(fileSystem.nodes[link] == .file(executable: true), "N1 제거가 남의 파일을 지움")
        }

        // N2: 확인한 PATH(/usr/local/bin)에 다른 `stellar`, 설치 폴더는 PATH에 없음 → 그 프로그램이 먼저 실행됨 → stellar-cli.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addFile("/usr/local/bin/stellar", executable: true)
            let installer = makeInstaller(fileSystem)
            let plan = readyPlan(installer)
            checker.expect(
                plan?.linkPath == fallbackLink
                    && plan?.primaryNameConflict
                        == StellarCommandNameConflict(commandName: "stellar", path: "/usr/local/bin/stellar", position: .earlierOnPath),
                "N2 PATH 앞쪽 가림 판정 실패: \(String(describing: plan))")
            if let plan {
                checker.expect(installer.install(plan).isSuccess && fileSystem.nodes[link] == nil, "N2 stellar를 설치함")
                checker.expect(fileSystem.nodes["/usr/local/bin/stellar"] == .file(executable: true), "N2 남의 프로그램이 바뀜")
            }
        }

        // N3: 설치 폴더가 PATH에 있고, 그보다 앞 폴더에 다른 `stellar` → stellar-cli.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: true)
            fileSystem.addDirectory("/opt/tools/bin", writable: false)
            fileSystem.addFile("/opt/tools/bin/stellar", executable: true)
            let installer = makeInstaller(
                fileSystem, searchPath: StellarCommandSearchPath(ordered: ["/opt/tools/bin", localBin, "/usr/bin"]))
            let plan = readyPlan(installer)
            checker.expect(
                plan?.linkPath == fallbackLink && plan?.primaryNameConflict?.position == .earlierOnPath
                    && plan?.directoryOnSearchPath == true,
                "N3 PATH 앞 폴더 가림 판정 실패: \(String(describing: plan))")
        }

        // N4: 다른 `stellar`가 PATH 뒤쪽에만 있음 → stellar 유지, 가려질 프로그램을 동의 화면에 알림.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: true)
            fileSystem.addDirectory("/opt/later/bin", writable: false)
            fileSystem.addFile("/opt/later/bin/stellar", executable: true)
            let installer = makeInstaller(
                fileSystem, searchPath: StellarCommandSearchPath(ordered: [localBin, "/usr/bin", "/opt/later/bin"]))
            let plan = readyPlan(installer)
            checker.expect(
                plan?.linkPath == link && plan?.primaryNameConflict == nil
                    && plan?.hiddenProgramPaths == ["/opt/later/bin/stellar"],
                "N4 PATH 뒤쪽 프로그램 처리 불일치: \(String(describing: plan))")
        }

        // N5: 순서를 모르는 폴더(Homebrew 접두사)에 다른 `stellar` → 먼저일 수 있으므로 stellar-cli.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory("/opt/homebrew/bin", writable: false)
            fileSystem.addFile("/opt/homebrew/bin/stellar", executable: true)
            let installer = makeInstaller(
                fileSystem,
                searchPath: StellarCommandSearchPath(ordered: defaultSearchPath.ordered, unordered: ["/opt/homebrew/bin"]))
            let plan = readyPlan(installer)
            checker.expect(
                plan?.linkPath == fallbackLink && plan?.primaryNameConflict?.position == .possiblyEarlierOnPath,
                "N5 순서 모르는 폴더 판정 실패: \(String(describing: plan))")
            // 실행 권한이 없는 같은 이름 파일은 셸이 고르지 않으므로 충돌이 아닙니다.
            fileSystem.addFile("/opt/homebrew/bin/stellar", executable: false)
            checker.expect(readyPlan(installer)?.linkPath == link, "N5 실행 불가 파일을 충돌로 봄")
        }

        // N6: 두 이름 모두 다른 프로그램(설치 위치의 남의 링크 + 폴더) → 설치 0, 알림 상태.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: true)
            fileSystem.addSymbolicLink(link, destination: "/opt/other/bin/stellar")
            fileSystem.addDirectory(fallbackLink, writable: true)
            let installer = makeInstaller(fileSystem)
            checker.expect(!installer.isStellarOwnedLink(atPath: link), "N6 남의 링크를 소유로 판정")
            let status = installer.status()
            checker.expect(
                status == .namesUnavailable([
                    StellarCommandNameConflict(commandName: "stellar", path: link, position: .atInstallLocation),
                    StellarCommandNameConflict(commandName: "stellar-cli", path: fallbackLink, position: .atInstallLocation),
                ]),
                "N6 두 이름 모두 막힌 상태 불일치: \(status)")
            checker.expect(installer.uninstall().removedPaths.isEmpty, "N6 남의 링크를 제거함")
            checker.expect(fileSystem.mutationCount == 0, "N6 파일을 바꿈")
            checker.expect(fileSystem.nodes[link] == .symbolicLink(destination: "/opt/other/bin/stellar"), "N6 남의 링크가 바뀜")
        }

        // N6b: stellar는 PATH 앞쪽, stellar-cli는 설치 위치에서 막힘 → 설치 0.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addFile("/usr/local/bin/stellar", executable: true)
            fileSystem.addDirectory(localBin, writable: true)
            fileSystem.addFile(fallbackLink, executable: true)
            let status = makeInstaller(fileSystem).status()
            if case .namesUnavailable(let conflicts) = status {
                checker.expect(
                    conflicts.map(\.position) == [.earlierOnPath, .atInstallLocation], "N6b 충돌 위치 불일치: \(conflicts)")
            } else {
                checker.expect(false, "N6b 상태가 namesUnavailable이 아님: \(status)")
            }
        }

        // N7: Stellar 자기 링크만 있음 → stellar 유지.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: true)
            fileSystem.addSymbolicLink(link, destination: stubHelper)
            let installer = makeInstaller(fileSystem)
            checker.expect(
                installer.status() == .installed(StellarCommandInstallation(linkPath: link, directoryOnSearchPath: false)),
                "N7 자기 링크 상태 불일치: \(installer.status())")
            // PATH 앞쪽의 자기 링크(같은 helper)는 다른 프로그램이 아니므로 새 설치도 stellar입니다.
            let elsewhere = makeFileSystem()
            elsewhere.addDirectory("/usr/local/bin", writable: false)
            elsewhere.addSymbolicLink("/usr/local/bin/stellar", destination: stubHelper)
            checker.expect(readyPlan(makeInstaller(elsewhere))?.linkPath == link, "N7 PATH의 자기 링크를 충돌로 봄")
        }

        // N8: 다른 앱의 `Contents/Helpers/stellar`를 가리키는 링크는 Stellar 소유가 아님.
        fixture {
            let fileSystem = makeFileSystem()
            let other = "/Applications/Other.app"
            fileSystem.addFile(other + "/Contents/Helpers/stellar", executable: true)
            fileSystem.bundleIdentifiers[other] = "com.example.other"
            fileSystem.addDirectory(localBin, writable: true)
            fileSystem.addSymbolicLink(link, destination: other + "/Contents/Helpers/stellar")
            let installer = makeInstaller(fileSystem)
            checker.expect(!installer.isStellarOwnedLink(atPath: link), "N8 다른 앱 helper 링크를 소유로 판정")
            checker.expect(readyPlan(installer)?.linkPath == fallbackLink, "N8 다른 앱 링크 자리에서 대체 이름을 고르지 않음")
            checker.expect(installer.uninstall().removedPaths.isEmpty, "N8 다른 앱 링크를 제거함")
        }

        // N9: 설치 뒤 다른 `stellar`가 PATH 앞쪽에 생김(상태 확인, XXIV.5.5) → shadowed + stellar-cli 제안.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: true)
            fileSystem.addSymbolicLink(link, destination: stubHelper)
            fileSystem.addFile("/usr/local/bin/stellar", executable: true)
            let installer = makeInstaller(fileSystem)
            let status = installer.status()
            guard case .shadowed(let installation, let conflict, let fallback?) = status else {
                checker.expect(false, "N9 상태가 shadowed+fallback이 아님: \(status)")
                return
            }
            checker.expect(
                installation.linkPath == link && conflict.path == "/usr/local/bin/stellar" && fallback.linkPath == fallbackLink,
                "N9 가림 판정 불일치")
            checker.expect(fileSystem.mutationCount == 0, "N9 상태 확인이 파일을 바꿈")
            checker.expect(installer.install(fallback).isSuccess, "N9 대체 이름 설치 실패")
            checker.expect(
                installer.status() == .installed(StellarCommandInstallation(linkPath: fallbackLink, directoryOnSearchPath: false)),
                "N9 대체 이름 설치 후 상태 불일치: \(installer.status())")
            let removed = installer.uninstall()
            checker.expect(Set(removed.removedPaths) == [link, fallbackLink] && installer.installedLinkPaths().isEmpty,
                "N9 되돌리기 후 설치 항목이 남음: \(removed)")
        }

        // N10: 가려졌고 stellar-cli도 다른 프로그램 → 대체 설치 없음(알림만).
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: true)
            fileSystem.addSymbolicLink(link, destination: stubHelper)
            fileSystem.addFile("/usr/local/bin/stellar", executable: true)
            fileSystem.addFile("/usr/local/bin/stellar-cli", executable: true)
            let status = makeInstaller(fileSystem).status()
            if case .shadowed(_, _, nil) = status {} else { checker.expect(false, "N10 상태 불일치: \(status)") }
        }

        // R1: 옮겨진 앱을 가리키는 소유 링크(dangling 절대 경로, 상대 경로) → 그 자리에서 다시 연결.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: true)
            let old = "/Users/stub/Downloads/Old Stellar.app/Contents/Helpers/stellar"
            fileSystem.addSymbolicLink(link, destination: old)
            fileSystem.addSymbolicLink(fallbackLink, destination: "../../Apps/Stellar.app/Contents/Helpers/stellar")
            let installer = makeInstaller(fileSystem)
            checker.expect(installer.isStellarOwnedLink(atPath: fallbackLink), "R1 상대 경로 소유 링크 판정 실패")
            guard case .needsRepair(let plan) = installer.status() else {
                checker.expect(false, "R1 오래된 링크 상태가 needsRepair가 아님: \(installer.status())")
                return
            }
            checker.expect(plan.linkPath == link && plan.action == .replaceOwnLink(previousDestination: old), "R1 다시 연결 계획 불일치")
            checker.expect(installer.install(plan).isSuccess, "R1 다시 연결 실패")
            checker.expect(fileSystem.nodes[link] == .symbolicLink(destination: stubHelper), "R1 다시 연결 대상 불일치")
            checker.expect(installer.uninstall().removedPaths.count == 2, "R1 소유 링크 2개를 모두 제거하지 않음")
        }

        // R2: 확인과 실행 사이에 설치 위치에 남의 파일이 생김.
        fixture {
            let fileSystem = makeFileSystem()
            let installer = makeInstaller(fileSystem)
            guard let plan = readyPlan(installer) else { return }
            fileSystem.addDirectory(localBin, writable: true)
            fileSystem.addFile(link, executable: false)
            checker.expect(installer.install(plan) == .failure(.conflict(path: link)), "R2 경합에서 충돌을 보고하지 않음")
            checker.expect(fileSystem.nodes[link] == .file(executable: false), "R2 경합에서 남의 파일이 바뀜")
        }

        // R3: 다시 연결 직전(소유 확인 뒤)에 링크가 남의 파일로 바뀜 → 바꾸지 않고 충돌.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: true)
            fileSystem.addSymbolicLink(link, destination: "/Users/stub/Old.app/Contents/Helpers/stellar")
            let installer = makeInstaller(fileSystem)
            guard case .needsRepair(let plan) = installer.status() else { return }
            fileSystem.beforeNextMutation = {
                fileSystem.removeNode(link)
                fileSystem.addFile(link, executable: true)
            }
            checker.expect(installer.install(plan) == .failure(.conflict(path: link)), "R3 교체 경합 충돌 미보고")
            checker.expect(fileSystem.nodes[link] == .file(executable: true), "R3 교체 경합에서 남의 파일을 덮음")
        }

        // R4: 제거 직전에 소유 링크가 남의 링크로 바뀜 → 지우지 않음.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: true)
            fileSystem.addSymbolicLink(link, destination: stubHelper)
            let installer = makeInstaller(fileSystem)
            fileSystem.beforeNextMutation = { fileSystem.addSymbolicLink(link, destination: "/opt/other/bin/stellar") }
            let removed = installer.uninstall()
            checker.expect(removed.removedPaths.isEmpty && removed.failures.isEmpty, "R4 제거 경합 결과 불일치: \(removed)")
            checker.expect(fileSystem.nodes[link] == .symbolicLink(destination: "/opt/other/bin/stellar"), "R4 남의 링크를 지움")
        }

        // R5: 확인 뒤 PATH 앞쪽에 같은 이름이 생김 → 설치 0.
        fixture {
            let fileSystem = makeFileSystem()
            let installer = makeInstaller(fileSystem)
            guard let plan = readyPlan(installer) else { return }
            fileSystem.addFile("/usr/local/bin/stellar", executable: true)
            let before = fileSystem.mutationCount
            checker.expect(
                installer.install(plan) == .failure(.conflict(path: "/usr/local/bin/stellar")) && fileSystem.mutationCount == before,
                "R5 확인 뒤 가림을 막지 못함")
        }

        // E1: 파일 시스템 오류 전달.
        fixture {
            let fileSystem = makeFileSystem()
            let installer = makeInstaller(fileSystem)
            guard let plan = readyPlan(installer) else { return }
            fileSystem.failNextLinkOperation = "No space left on device"
            checker.expect(
                installer.install(plan)
                    == .failure(.fileSystem(StellarCommandFileSystemError(path: link, message: "No space left on device"))),
                "E1 파일 시스템 오류 전달 실패")
        }

        checker.expect(
            StellarCommandInstaller.pathExportLine(for: localBin, homeDirectory: stubHome)
                == "export PATH=\"$HOME/.local/bin:$PATH\"",
            "PATH 안내 줄 불일치")
        checker.expect(
            StellarCommandInstaller.lexicallyNormalized("/a//b/./c/../d/") == "/a/b/d", "경로 정규화 불일치")
        checker.expect(
            StellarCommandInstaller.appBundlePath(ofHelper: "/A/Stellar.app/Contents/Helpers/stellar") == "/A/Stellar.app"
                && StellarCommandInstaller.appBundlePath(ofHelper: "/A/Stellar.app/Contents/Helpers/stellar-cli") == nil,
            "helper 경로 모양 판정 불일치")
    }

    static func checkSearchPath(_ checker: Checker) {
        let fileSystem = StellarCommandFileSystemStub(homeDirectory: stubHome)
        fileSystem.textFiles["/etc/paths"] = "/usr/local/bin\n/usr/bin\n/bin\n# comment\n"
        fileSystem.textFiles["/etc/paths.d/10-cryptex"] = "/System/Cryptexes/App/usr/bin\n"
        let path = StellarCommandSearchPath.system(fileSystem: fileSystem, environmentPath: "~/.local/bin:/usr/bin:relative")
        checker.expect(
            path.ordered == ["/usr/local/bin", "/usr/bin", "/bin", "/System/Cryptexes/App/usr/bin", stubHome + "/.local/bin"],
            "PATH 구성 불일치: \(path.ordered)")
        checker.expect(
            path.unordered == [homeBin, "/opt/homebrew/bin", "/opt/local/bin"], "순서 모르는 폴더 불일치: \(path.unordered)")
    }

    /// 실제 파일 시스템 구현(`LocalStellarCommandFileSystem`)을 임시 폴더 안에서만 검사합니다.
    /// 사용자 위치(~/.local/bin 등)에는 쓰지 않습니다. RENAME_SWAP 교체, symlink EEXIST, 경합 시 원래 항목 보존을 봅니다.
    static func checkInstallerOnDisk(_ checker: Checker) {
        let manager = FileManager.default
        let root = (NSTemporaryDirectory() as NSString).appendingPathComponent("stellar-welcome-check-\(UUID().uuidString)")
        defer { try? manager.removeItem(atPath: root) }
        let home = root + "/home"
        let app = root + "/Apps/Stellar.app"
        let helper = app + "/Contents/Helpers/stellar"
        do {
            try manager.createDirectory(atPath: home, withIntermediateDirectories: true)
            try manager.createDirectory(atPath: app + "/Contents/Helpers", withIntermediateDirectories: true)
            try Data("#!/bin/sh\n".utf8).write(to: URL(fileURLWithPath: helper))
            try manager.setAttributes([.posixPermissions: 0o755], ofItemAtPath: helper)
            let plist = ["CFBundleIdentifier": ownerBundleID] as NSDictionary
            guard plist.write(toFile: app + "/Contents/Info.plist", atomically: true) else {
                checker.expect(false, "disk: Info.plist 쓰기 실패")
                return
            }
        } catch {
            checker.expect(false, "disk: 준비 실패 \(error)")
            return
        }
        let fileSystem = LocalStellarCommandFileSystem(homeDirectory: home)
        // 실제 시스템 폴더 내용이 판정에 섞이지 않도록 PATH는 임시 폴더만 씁니다.
        let installer = StellarCommandInstaller(
            appBundlePath: app, fileSystem: fileSystem,
            searchPath: StellarCommandSearchPath(ordered: [root + "/sys/bin"]), ownerBundleIdentifier: ownerBundleID)
        let bin = home + "/.local/bin"
        let link = bin + "/stellar"
        func leftovers() -> [String] {
            ((try? manager.contentsOfDirectory(atPath: bin)) ?? []).filter { $0.hasPrefix(".stellar-link-") }
        }

        guard case .ready(let plan) = installer.status(), plan.linkPath == link, plan.createsDirectory else {
            checker.expect(false, "disk: 새 설치 상태 불일치 \(installer.status())")
            return
        }
        checker.expect(!manager.fileExists(atPath: bin), "disk: 판정이 폴더를 만듦")
        checker.expect(installer.install(plan).isSuccess, "disk: 설치 실패")
        checker.expect(fileSystem.entry(atPath: link) == .symbolicLink(destination: helper), "disk: 링크 대상 불일치")
        checker.expect(installer.status() == .installed(StellarCommandInstallation(linkPath: link, directoryOnSearchPath: false)),
            "disk: 설치 후 상태 불일치")

        // 옮겨진 사본을 가리키는 소유 링크 → RENAME_SWAP으로 다시 연결.
        let moved = root + "/Moved/Stellar.app/Contents/Helpers/stellar"
        _ = installer.uninstall()
        do { try manager.createSymbolicLink(atPath: link, withDestinationPath: moved) } catch {
            checker.expect(false, "disk: 오래된 링크 준비 실패")
        }
        if case .needsRepair(let repair) = installer.status() {
            checker.expect(installer.install(repair).isSuccess, "disk: 다시 연결 실패")
            checker.expect(fileSystem.entry(atPath: link) == .symbolicLink(destination: helper), "disk: 다시 연결 대상 불일치")
        } else {
            checker.expect(false, "disk: needsRepair가 아님 \(installer.status())")
        }
        checker.expect(leftovers().isEmpty, "disk: 임시 링크가 남음 \(leftovers())")

        // 확인한 링크가 아닌 항목은 교체·삭제하지 않음(경합 흉내: 기대 대상이 다름).
        let foreign = bin + "/other-tool"
        try? Data("x".utf8).write(to: URL(fileURLWithPath: foreign))
        var replaceRefused = false
        do {
            try fileSystem.replaceSymbolicLink(atPath: foreign, expectedDestination: helper, newDestination: helper)
        } catch let error as StellarCommandFileSystemError {
            replaceRefused = error.isConflict
        } catch {}
        checker.expect(replaceRefused && fileSystem.entry(atPath: foreign) == .regularFile, "disk: 남의 파일을 교체함")
        var removeRefused = false
        do {
            try fileSystem.removeSymbolicLink(atPath: link, expectedDestination: moved)
        } catch let error as StellarCommandFileSystemError {
            removeRefused = error.isConflict
        } catch {}
        checker.expect(removeRefused && fileSystem.entry(atPath: link) == .symbolicLink(destination: helper), "disk: 기대와 다른 링크를 지움")
        checker.expect(leftovers().isEmpty, "disk: 거부 후 임시 항목이 남음 \(leftovers())")
        var createRefused = false
        do { try fileSystem.createSymbolicLink(atPath: foreign, destination: helper) } catch { createRefused = true }
        checker.expect(createRefused && fileSystem.entry(atPath: foreign) == .regularFile, "disk: symlink가 기존 파일을 덮음")

        // 설치 위치의 남의 파일 → stellar-cli, 되돌리기 후 설치 항목 0.
        _ = installer.uninstall()
        try? Data("other".utf8).write(to: URL(fileURLWithPath: link))
        if let fallback = readyPlan(installer), fallback.linkPath == bin + "/stellar-cli" {
            checker.expect(installer.install(fallback).isSuccess, "disk: 대체 이름 설치 실패")
        } else {
            checker.expect(false, "disk: 대체 이름 계획 불일치 \(installer.status())")
        }
        let removed = installer.uninstall()
        checker.expect(removed.removedPaths == [bin + "/stellar-cli"] && installer.installedLinkPaths().isEmpty,
            "disk: 되돌리기 결과 불일치 \(removed)")
        checker.expect(fileSystem.entry(atPath: link) == .regularFile, "disk: 남의 파일이 바뀜")
        checker.note("disk installer ok")
    }

    /// 상태 확인·설치·제거의 UI 경로가 메인 스레드에서 파일 시스템을 호출하지 않는지(XXIV.5.3·XXIV.5.5).
    static func checkInstallerThreading(_ checker: Checker) {
        let base = makeFileSystem()
        let audit = StellarCommandFileSystemThreadAudit(base: base)
        let installer = makeInstaller(audit)
        let queue = DispatchQueue(label: "xyz.taeyoon.stellar.welcome-ui-check.status")

        var evaluated: StellarCommandInstallStatus?
        StellarCommandStatusCheck.evaluate(on: queue, makeInstaller: { installer }) { _, status in evaluated = status }
        checker.expect(spin(until: { evaluated != nil }), "thread: 상태 확인 결과가 오지 않음")

        // 첫 실행 제안 경로: 창이 보이지 않으면 띄우지 않지만, 판정은 메인 스레드 밖에서 끝나야 합니다.
        let suite = "xyz.taeyoon.stellar.welcome-ui-check.thread.\(UUID().uuidString)"
        if let defaults = UserDefaults(suiteName: suite), let fixture = makeWelcome(.empty) {
            var offered: (StellarCommandInstallStatus, WelcomeCommandInstallSheetController?)?
            fixture.controller.presentCommandInstallOfferIfNeeded(
                makeInstaller: { installer }, policy: WelcomeFirstLaunchPolicy(defaults: defaults)
            ) { offered = ($0, $1) }
            checker.expect(spin(until: { offered != nil }), "thread: 첫 실행 판정 결과가 오지 않음")
            checker.expect(offered?.1 == nil, "thread: 보이지 않는 창에 sheet를 띄움")
            defaults.removePersistentDomain(forName: suite)
            fixture.window.close()
        }

        // 관리 sheet: 확인 → Install → Remove → 경합 후 Check Again.
        let sheet = WelcomeCommandInstallSheetController(installer: installer, status: nil)
        let window = offscreenWindow(for: sheet)
        defer { window.close() }
        func waitStatus() -> Bool {
            spin(until: {
                if case .status = sheet.phase { return true }
                return false
            })
        }
        checker.expect(waitStatus(), "thread: 확인 단계가 끝나지 않음")
        checker.expect(base.mutationCount == 0, "thread: 확인이 파일을 바꿈")
        sheet.primaryButton.performClick(nil)
        checker.expect(spin(until: {
            if case .installed = sheet.phase { return true }
            return false
        }), "thread: 설치가 끝나지 않음")
        sheet.destructiveButton.performClick(nil)
        checker.expect(spin(until: {
            if case .removed = sheet.phase { return true }
            return false
        }), "thread: 제거가 끝나지 않음")
        if let plan = readyPlan(makeInstaller(base)) {
            sheet.show(.installFailed(.conflict(path: plan.linkPath), plan: plan))
            sheet.primaryButton.performClick(nil)
            checker.expect(waitStatus(), "thread: Check Again이 상태로 돌아오지 않음")
        }
        checker.expect(audit.mainThreadOperations.isEmpty, "thread: 메인 스레드 파일 조회 \(audit.mainThreadOperations)")
        checker.expect(audit.backgroundOperationCount > 0, "thread: 검사 대상 호출이 없음")
        checker.note("main-thread fs calls \(audit.mainThreadOperations.count), background \(audit.backgroundOperationCount)")
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
            linkPath: localBin + "/stellar", destination: stubHelper, action: .create, createsDirectory: true,
            directoryOnSearchPath: false, primaryNameConflict: nil, hiddenProgramPaths: [])
        let conflict = StellarCommandNameConflict(commandName: "stellar", path: "/x", position: .atInstallLocation)
        let installation = StellarCommandInstallation(linkPath: localBin + "/stellar", directoryOnSearchPath: false)
        checker.expect(policy.shouldOfferCommandInstall(for: .ready(plan)), "첫 실행 제안이 꺼져 있음")
        checker.expect(policy.shouldOfferCommandInstall(for: .namesUnavailable([conflict])), "두 이름이 막혔을 때 알리지 않음")
        checker.expect(
            policy.shouldOfferCommandInstall(for: .shadowed(installation, conflict: conflict, fallback: nil)), "가려진 설치를 알리지 않음")
        checker.expect(
            !policy.shouldOfferCommandInstall(for: .helperUnavailable(helperPath: "/x")), "helper 없는 빌드에서 제안함")
        checker.expect(!policy.shouldOfferCommandInstall(for: .installed(installation)), "이미 설치됐는데 제안함")
        checker.expect(
            !policy.shouldOfferCommandInstall(for: .unsupportedAppLocation(.translocated)), "임시 위치에서 제안함")
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
        // ⌘N·⌘O는 File 메뉴가 소유하므로 버튼에 같은 단축키를 걸지 않습니다(메뉴와 중복 방지).
        checker.expect(
            [welcome.newProjectButton, welcome.openProjectButton, welcome.cloneProjectButton].allSatisfy(\.keyEquivalent.isEmpty),
            "동작 버튼에 메뉴와 겹치는 단축키가 있음")
        checker.expect(
            menuValidation(welcome) == WelcomeCommandAvailability(newProject: true, openProject: true, cloneProject: true),
            "메뉴 활성 판정 불일치(sample): \(menuValidation(welcome))")
        checker.expect(welcome.gitNoticeView.isHidden && welcome.activityView.isHidden, "안내·진행 표시가 보임")
        checker.expect(welcome.versionLabel.stringValue.contains("1.2.3"), "버전 표시 불일치")

        let first = table.view(atColumn: 0, row: 0, makeIfNecessary: true) as? WelcomeRecentProjectCellView
        let missing = table.view(atColumn: 0, row: 3, makeIfNecessary: true) as? WelcomeRecentProjectCellView
        checker.expect(first?.accessibilityLabel()?.contains("Design System") == true, "행 접근성 레이블 없음")
        checker.expect(
            first?.accessibilityChildren()?.isEmpty == true && first?.nameLabel.isAccessibilityElement() == false
                && first?.pathLabel.isAccessibilityElement() == false,
            "행 하위 레이블이 접근성 트리에 남아 중복으로 읽힘")
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
            checker.expect(
                menuValidation(welcome) == WelcomeCommandAvailability(newProject: false, openProject: true, cloneProject: false),
                "메뉴 활성 판정 불일치(Git 없음): \(menuValidation(welcome))")
            expectCleanLayout(checker, window: window, context: "welcome/gitMissing")
            checker.expect(actionsColumnFits(welcome), "동작 열이 잘림(gitMissing)")
            notice.checkAgainButton.performClick(nil)
            checker.expect(stub.sentCommands.last == .recheckGit, "다시 검사 명령 없음")

            // 가장 긴 경우: Git 없음 + 진행 중 + 쓸 수 없는 Git 설명.
            stub.replaceSnapshot {
                $0.git = .unusable(.versionTooOld(found: "2.20.1", minimum: "2.40"))
                $0.activity = WelcomeActivity(
                    id: 3, kind: .opening, projectName: "A Project With A Long Name", fractionCompleted: nil, isCancellable: true)
            }
            welcome.renderNow()
            checker.expect(notice.commandRow.isHidden, "쓸 수 없는 Git에 설치 명령을 보여 줌")
            checker.expect(
                notice.messageLabel.stringValue == WelcomeStrings.gitProblem(.versionTooOld(found: "2.20.1", minimum: "2.40")),
                "Git 문제 문구를 셸이 만들지 않음")
            checker.expect(
                menuValidation(welcome) == WelcomeCommandAvailability(newProject: false, openProject: false, cloneProject: false),
                "메뉴 활성 판정 불일치(진행 중)")
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
            // 끝난 작업의 늦은 취소(다른 식별자)는 현재 작업을 취소하지 않습니다.
            stub.send(.cancelActivity(id: 999))
            welcome.renderNow()
            checker.expect(!activity.isHidden, "다른 작업 식별자의 취소가 현재 작업을 취소함")
            activity.cancelButton.performClick(nil)
            checker.expect(stub.sentCommands.last == .cancelActivity(id: 7), "취소 명령에 작업 식별자가 없음")
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
            .operationFailed(.networkUnavailable),
            .operationFailed(.internalError(code: "E_CLONE_OPEN")),
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
        let failed = WelcomeViewController.makeAlert(for: WelcomeNotice(id: 10, kind: .operationFailed(.remoteNotFound)))
        checker.expect(failed.informativeText == WelcomeStrings.failureReason(.remoteNotFound), "실패 문구를 셸이 만들지 않음")

        // 새 프로젝트 저장 패널: 같은 이름은 바꾸기 확인 전에(userEnteredFilename, okFlag true) 막습니다.
        let parent = URL(fileURLWithPath: "/Users/stub/Projects", isDirectory: true)
        checker.expect(
            WelcomeViewController.folderNameIsTaken("design-system", in: parent, fileExists: { $0 == "/Users/stub/Projects/design-system" })
                && !WelcomeViewController.folderNameIsTaken("new-project", in: parent, fileExists: { _ in false })
                && !WelcomeViewController.folderNameIsTaken("x", in: nil, fileExists: { _ in true }),
            "같은 이름 판정 불일치")
        if let fixture = makeWelcome(.empty) {
            checker.expect(
                fixture.welcome.panel(NSObject(), userEnteredFilename: "typing", confirmed: false) == "typing",
                "입력 중(okFlag false)에 파일 이름을 막음")
            fixture.window.close()
        }
    }

    /// 메뉴 항목 활성 판정(`validateMenuItem`)을 세 selector로 읽습니다.
    static func menuValidation(_ welcome: WelcomeViewController) -> WelcomeCommandAvailability {
        func enabled(_ action: Selector) -> Bool {
            welcome.validateMenuItem(NSMenuItem(title: "", action: action, keyEquivalent: ""))
        }
        return WelcomeCommandAvailability(
            newProject: enabled(#selector(WelcomeViewController.newStellarProject(_:))),
            openProject: enabled(#selector(WelcomeViewController.openStellarProject(_:))),
            cloneProject: enabled(#selector(WelcomeViewController.cloneStellarProject(_:))))
    }

    /// 접근성 announcement와 복사 피드백(가이드 8.3, 8.1).
    static func checkAccessibilityFeedback(_ checker: Checker) {
        var announced: [String] = []
        WelcomeAccessibility.announcementObserver = { announced.append($0) }
        defer { WelcomeAccessibility.announcementObserver = nil }

        // 최근 목록 상태 문구 변화 → announcement.
        if let fixture = makeWelcome(.loading) {
            fixture.welcome.recentsView.apply(.failed(.notEnoughSpace))
            checker.expect(announced.last == WelcomeStrings.recentsFailed, "목록 상태 변화를 알리지 않음: \(announced)")
            checker.expect(
                fixture.welcome.recentsView.stateMessageLabel.stringValue == WelcomeStrings.failureReason(.notEnoughSpace),
                "목록 실패 문구를 셸이 만들지 않음")
            fixture.welcome.recentsView.apply(.loaded([]))
            fixture.window.close()
        }

        // 설치 sheet 단계 변화 → announcement(처음 표시는 sheet가 읽히므로 제외).
        let installer = makeInstaller(makeFileSystem())
        let sheet = WelcomeCommandInstallSheetController(
            installer: installer, status: .helperUnavailable(helperPath: installer.helperPath))
        let window = offscreenWindow(for: sheet)
        let before = announced.count
        sheet.show(.working(removing: true))
        checker.expect(announced.count == before + 1 && announced.last == WelcomeStrings.removing, "sheet 단계 변화를 알리지 않음")
        window.close()

        // 복사 버튼: 클립보드(검사용 이름 pasteboard)에 쓰고, 체크 표시·도움말·알림으로 피드백한 뒤 되돌아옵니다.
        let pasteboard = NSPasteboard(name: NSPasteboard.Name("xyz.taeyoon.stellar.welcome-ui-check.\(UUID().uuidString)"))
        defer { pasteboard.releaseGlobally() }
        let button = WelcomeCopyButton(accessibilityLabel: WelcomeStrings.copyCommand)
        button.pasteboard = pasteboard
        button.textToCopy = "export PATH=\"$HOME/.local/bin:$PATH\""
        button.performClick(nil)
        checker.expect(pasteboard.string(forType: .string) == button.textToCopy, "복사한 내용 불일치")
        checker.expect(
            button.isShowingCopiedFeedback && button.toolTip == WelcomeStrings.copied && announced.last == WelcomeStrings.copied,
            "복사 피드백 없음")
        checker.expect(button.accessibilityLabel() == WelcomeStrings.copyCommand, "복사 버튼 접근성 레이블이 바뀜")
        checker.expect(
            spin(until: { !button.isShowingCopiedFeedback }, timeout: WelcomeCopyButton.feedbackDuration + 2)
                && button.toolTip == WelcomeStrings.copyCommand,
            "복사 피드백이 되돌아오지 않음")
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

    // MARK: 명령 설치 sheet(XXIV.5.6 화면, 실제 표시·VoiceOver는 통합 후 확인)

    static func checkInstallSheet(_ checker: Checker) {
        let fileSystem = makeFileSystem()
        let installer = makeInstaller(fileSystem)
        guard let plan = readyPlan(installer) else {
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
        checker.expect(
            sheet.targetValueLabel.stringValue == stubHelper, "링크 대상 표시 불일치: \(sheet.targetValueLabel.stringValue)")
        checker.expect(!sheet.pathLineRow.isHidden && sheet.pathLineLabel.stringValue.contains("$HOME/.local/bin"), "PATH 안내 없음")
        checker.expect(sheet.pathCopyButton.accessibilityLabel() != nil && sheet.pathCopyButton.toolTip != nil, "PATH 복사 버튼 레이블 없음")
        checker.expect(fileSystem.nodes[plan.linkPath] == nil && fileSystem.mutationCount == 0, "동의 전에 설치함")

        // 동의 → 설치(작업 큐) → 결과.
        sheet.primaryButton.performClick(nil)
        let installed = spin(until: {
            if case .installed = sheet.phase { return true }
            return false
        })
        checker.expect(installed, "설치 결과 단계로 가지 않음: \(sheet.phase)")
        checker.expect(fileSystem.nodes[plan.linkPath] == .symbolicLink(destination: installer.helperPath), "sheet 설치 결과 불일치")
        checker.expect(sheet.titleLabel.stringValue == WelcomeStrings.installedTitle("stellar"), "설치한 이름이 제목에 없음")
        checker.expect(
            !sheet.destructiveButton.isHidden && sheet.destructiveButton.hasDestructiveAction
                && sheet.destructiveButton.keyEquivalent.isEmpty,
            "제거 버튼이 파괴적 표시·비기본이 아님")
        checker.expect(sheet.primaryButton.title == WelcomeStrings.done, "설치 후 Done 없음")
        expectCleanLayout(checker, window: window, context: "install/installed")

        // 제거(되돌리기) → 설치 항목 0.
        sheet.destructiveButton.performClick(nil)
        let removed = spin(until: {
            if case .removed = sheet.phase { return true }
            return false
        })
        checker.expect(removed, "제거 결과 단계로 가지 않음: \(sheet.phase)")
        checker.expect(fileSystem.nodes[plan.linkPath] == nil && installer.installedLinkPaths().isEmpty, "sheet 되돌리기 후 설치 항목이 남음")
        checker.expect(sheet.titleLabel.stringValue == WelcomeStrings.removedTitle, "제거 제목 불일치")
        sheet.primaryButton.performClick(nil)
        checker.expect(responses == [.removed], "sheet 응답 불일치: \(responses)")

        // Not Now: 파일 변경 0.
        do {
            let declineFileSystem = makeFileSystem()
            let declineInstaller = makeInstaller(declineFileSystem)
            if let declinePlan = readyPlan(declineInstaller) {
                let decline = WelcomeCommandInstallSheetController(installer: declineInstaller, status: .ready(declinePlan))
                var answer: WelcomeCommandInstallSheetController.Response?
                decline.onResponse = { answer = $0 }
                let declineWindow = offscreenWindow(for: decline)
                decline.cancelOperation(nil)
                checker.expect(answer == .declined && declineFileSystem.mutationCount == 0, "Not Now(Escape)가 파일을 바꾸거나 답하지 않음")
                declineWindow.close()
            }
        }

        // 대체 이름: 다른 프로그램이 stellar를 씀 → 제목·버튼·결과에 stellar-cli.
        do {
            let fallbackFileSystem = makeFileSystem()
            fallbackFileSystem.addFile("/usr/local/bin/stellar", executable: true)
            let fallbackInstaller = makeInstaller(fallbackFileSystem)
            if let fallbackPlan = readyPlan(fallbackInstaller) {
                let fallbackSheet = WelcomeCommandInstallSheetController(installer: fallbackInstaller, status: .ready(fallbackPlan))
                let fallbackWindow = offscreenWindow(for: fallbackSheet)
                checker.expect(
                    fallbackSheet.titleLabel.stringValue == WelcomeStrings.fallbackInstallTitle("stellar-cli")
                        && fallbackSheet.primaryButton.title == WelcomeStrings.installAs("stellar-cli")
                        && fallbackSheet.messageLabel.stringValue.contains("/usr/local/bin/stellar"),
                    "대체 이름 제안 화면 불일치: \(fallbackSheet.titleLabel.stringValue)")
                checker.expect(fallbackSheet.linkValueLabel.stringValue == "~/.local/bin/stellar-cli", "대체 이름 링크 표시 불일치")
                expectCleanLayout(checker, window: fallbackWindow, context: "install/fallback")
                fallbackSheet.primaryButton.performClick(nil)
                checker.expect(spin(until: {
                    if case .installed = fallbackSheet.phase { return true }
                    return false
                }), "대체 이름 설치가 끝나지 않음")
                checker.expect(
                    fallbackSheet.titleLabel.stringValue == WelcomeStrings.installedTitle("stellar-cli")
                        && fallbackFileSystem.nodes[localBin + "/stellar"] == nil,
                    "설치한 이름(stellar-cli)을 보여 주지 않음")
                fallbackWindow.close()
            }
        }

        // 그 밖의 단계: 모두 제목과 닫을 방법이 있고 레이아웃이 깨지지 않아야 합니다.
        let installation = StellarCommandInstallation(linkPath: plan.linkPath, directoryOnSearchPath: false)
        let conflict = StellarCommandNameConflict(commandName: "stellar", path: "/usr/local/bin/stellar", position: .earlierOnPath)
        var fallbackPlan = plan
        fallbackPlan.linkPath = localBin + "/stellar-cli"
        fallbackPlan.primaryNameConflict = conflict
        var hidingPlan = plan
        hidingPlan.hiddenProgramPaths = ["/opt/later/bin/stellar"]
        let otherPhases: [(String, WelcomeCommandInstallSheetController.Phase)] = [
            ("helperUnavailable", .status(.helperUnavailable(helperPath: installer.helperPath))),
            ("translocated", .status(.unsupportedAppLocation(.translocated))),
            ("readyFallback", .status(.ready(fallbackPlan))),
            ("readyHiding", .status(.ready(hidingPlan))),
            ("namesUnavailable", .status(.namesUnavailable([
                conflict,
                StellarCommandNameConflict(commandName: "stellar-cli", path: localBin + "/stellar-cli", position: .atInstallLocation),
            ]))),
            ("shadowedFallback", .status(.shadowed(installation, conflict: conflict, fallback: fallbackPlan))),
            ("shadowed", .status(.shadowed(installation, conflict: conflict, fallback: nil))),
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
        sheet.show(.status(.namesUnavailable([conflict])))
        checker.expect(
            sheet.primaryButton.title == WelcomeStrings.close && sheet.secondaryButton.isHidden && sheet.destructiveButton.isHidden
                && sheet.detailRows.isHidden,
            "두 이름이 막힌 알림에 설치 동작이 있음")
        sheet.show(.status(.shadowed(installation, conflict: conflict, fallback: nil)))
        checker.expect(
            sheet.primaryButton.title == WelcomeStrings.close && !sheet.destructiveButton.isHidden,
            "가려진 설치(대체 불가) 화면 불일치")
        sheet.show(.status(.ready(hidingPlan)))
        checker.expect(sheet.noteLabel.stringValue.contains("/opt/later/bin/stellar"), "가려질 프로그램 안내 없음")
        sheet.show(.status(.installed(StellarCommandInstallation(linkPath: plan.linkPath, directoryOnSearchPath: true))))
        checker.expect(sheet.pathLineRow.isHidden, "PATH에 있는 폴더에 PATH 안내를 보여 줌")
        sheet.show(.installFailed(.conflict(path: plan.linkPath), plan: plan))
        checker.expect(sheet.primaryButton.title == WelcomeStrings.checkAgainButton, "충돌 뒤 같은 계획을 다시 실행하려 함")

        sheet.show(.working(removing: false))
        checker.expect(
            !sheet.spinner.isHidden && sheet.primaryButton.isHidden && sheet.secondaryButton.isHidden,
            "작업 중 표시 불일치")
        expectCleanLayout(checker, window: window, context: "install/working")
        sheet.cancelOperation(nil)
        checker.expect(responses == [.removed], "작업 중 Escape가 sheet를 닫음")
        sheet.show(.checking)
        expectCleanLayout(checker, window: window, context: "install/checking")
        sheet.show(.status(.helperUnavailable(helperPath: installer.helperPath)))
    }
}

private extension Result {
    var isSuccess: Bool {
        if case .success = self { return true }
        return false
    }
}
