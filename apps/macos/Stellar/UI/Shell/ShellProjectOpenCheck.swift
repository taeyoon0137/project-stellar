import AppKit

/// Open Project 검사(`--project-open-check`, XII.6.2).
///
/// 실제 코어 runtime으로 임시 fixture 폴더를 열어 다음을 확인합니다.
/// - 메뉴 막대 Open Project…(⌘O) → 폴더 선택 → 코어 열기(열기 분류·안전 probe) → 프로젝트 창, 선택 취소 시 요청 없음
/// - 잘못된 폴더마다 코어 실패 코드와 셸 안내 원인이 정확히 대응: 폴더 없음, Git 저장소 아님, 폴더가 아닌 일반 파일,
///   `stellar.yaml` 없음, YAML 손상, 1 MiB 초과, 스키마 위반, 잘못된 project.id, 심볼릭 링크 매니페스트, 읽기 권한 없음,
///   상대·빈 경로
/// - 안내 문구: 원인마다 제목이 다르고 문제·해결 방법이 있으며 폴더 이름을 보여 줌, 코어 진단 메시지를 쓰지 않음
/// - 안내 위치: 메뉴에서 연 실패는 alert, 시작 창에서 연 실패는 시작 창 안내(`stellar.yaml` 없음·폴더 없음은 전용 안내),
///   복원 실패는 창의 실패 상태
/// - 모든 실패에서 프로젝트 창·코어 session이 생기지 않음
enum ShellProjectOpenCheck {
    static func run() -> Bool {
        MainActor.assumeIsolated {
            ShellProjectOpenCheckRunner().run()
        }
    }
}

@MainActor
private final class ShellProjectOpenCheckRunner {
    private var failures: [String] = []
    private var checks = 0
    private let fixtureRoot = URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
        .appendingPathComponent("StellarProjectOpenCheck-\(UUID().uuidString)", isDirectory: true)
    private var alerts: [(title: String, message: String)] = []
    private var nextChosenFolder: String?
    private var chooserCalls = 0

    private func expect(_ condition: Bool, _ message: @autoclosure () -> String) {
        checks += 1
        if !condition { failures.append(message()) }
    }

    func run() -> Bool {
        NSApplication.shared.setActivationPolicy(.accessory)
        defer { removeFixtures() }

        checkMappingTable()

        let host = RuntimeHost()
        guard host.created else {
            FileHandle.standardError.write(Data("project-open 실패: runtime create 실패\n".utf8))
            return false
        }
        defer { host.shutdown() }
        let coordinator = ShellAppCoordinator(runtimeHandle: host.handle)
        coordinator.presentsWindows = false
        let autosave = ShellCheckAutosave(prefix: "StellarProjectOpenCheck")
        defer { autosave.removeRecords() }
        coordinator.windowConfiguration = { autosave.configuration }
        coordinator.alertPresenter = { [weak self] title, message, _ in
            self?.alerts.append((title, message))
        }
        coordinator.folderChooser = { [weak self] _, completion in
            self?.chooserCalls += 1
            completion(self?.nextChosenFolder)
        }
        let previousMenu = NSApp.mainMenu
        let mainMenu = coordinator.installMainMenu(on: NSApp)
        defer { NSApp.mainMenu = previousMenu }

        autoreleasepool {
            checkMenuOpen(coordinator, mainMenu: mainMenu, host: host)
            checkCoreFailures(coordinator, host: host)
            checkWelcomeAndRestore(coordinator, host: host)
        }

        if failures.isEmpty {
            print("project-open ok: \(checks) checks (menu open via core, 12 invalid path fixtures, mapping, alert/welcome/restore)")
            return true
        }
        for failure in failures {
            FileHandle.standardError.write(Data("project-open 실패: \(failure)\n".utf8))
        }
        return false
    }

    // MARK: - fixture

    private enum Fixture: CaseIterable {
        case missing, plain, regularFile, gitOnly, corrupt, oversized, schemaInvalid, badID, symlink, unreadable, valid

        var folderName: String {
            switch self {
            case .missing: return "Missing Folder"
            case .plain: return "Plain Folder"
            case .regularFile: return "Plain File"
            case .gitOnly: return "Repository Without Manifest"
            case .corrupt: return "Corrupt Manifest"
            case .oversized: return "Oversized Manifest"
            case .schemaInvalid: return "Schema Invalid"
            case .badID: return "Bad Project ID"
            case .symlink: return "Linked Manifest"
            case .unreadable: return "Unreadable Manifest"
            case .valid: return "Open Check Project"
            }
        }

        /// 기대 안내 원인(valid는 nil = 열림).
        var expected: ShellProjectOpenFailure? {
            switch self {
            case .missing: return .folderMissing
            case .plain, .regularFile: return .notARepository
            case .gitOnly: return .notAStellarProject
            case .corrupt, .oversized: return .manifestCorrupt
            case .schemaInvalid: return .manifestInvalid
            case .badID: return .invalidProjectID
            case .symlink: return .untrustedSymlink
            case .unreadable: return .manifestUnreadable
            case .valid: return nil
            }
        }
    }

    private static func manifest(name: String, id: String = "0195c530-3acd-7bc1-8414-c213f61cb965") -> String {
        "schema_version: \"1\"\nproject:\n  id: \"\(id)\"\n  name: \(name)\ntarget_profiles: []\ndependencies: {}\n"
    }

    private func make(_ fixture: Fixture) -> String {
        let manager = FileManager.default
        let folder = fixtureRoot.appendingPathComponent(fixture.folderName, isDirectory: true)
        guard fixture != .missing else { return folder.path }
        if fixture == .regularFile {
            // Finder·`open` 명령으로 받은 일반 파일: 폴더는 아니지만 경로에 항목이 있습니다("폴더 없음"이 아님).
            let file = fixtureRoot.appendingPathComponent(fixture.folderName, isDirectory: false)
            try? manager.createDirectory(at: fixtureRoot, withIntermediateDirectories: true)
            try? "not a project\n".write(to: file, atomically: true, encoding: .utf8)
            return file.path
        }
        try? manager.createDirectory(at: folder, withIntermediateDirectories: true)
        if fixture != .plain {
            try? manager.createDirectory(at: folder.appendingPathComponent(".git", isDirectory: true), withIntermediateDirectories: true)
        }
        let manifestURL = folder.appendingPathComponent("stellar.yaml")
        func write(_ text: String) { try? text.write(to: manifestURL, atomically: true, encoding: .utf8) }
        switch fixture {
        case .missing, .plain, .regularFile, .gitOnly:
            break
        case .corrupt:
            write("schema_version: \"1\"\nproject: [unclosed\n")
        case .oversized:
            // loader 상한(1 MiB)을 넘는 매니페스트는 끝까지 읽지 않고 손상으로 거부합니다.
            write(Self.manifest(name: fixture.folderName) + "# " + String(repeating: "x", count: 1_100_000) + "\n")
        case .schemaInvalid:
            write("schema_version: \"1\"\n")
        case .badID:
            write(Self.manifest(name: fixture.folderName, id: "not-a-uuid"))
        case .symlink:
            let target = fixtureRoot.appendingPathComponent("linked-target.yaml")
            try? Self.manifest(name: fixture.folderName).write(to: target, atomically: true, encoding: .utf8)
            try? manager.createSymbolicLink(at: manifestURL, withDestinationURL: target)
        case .unreadable:
            write(Self.manifest(name: fixture.folderName))
            try? manager.setAttributes([.posixPermissions: 0o000], ofItemAtPath: manifestURL.path)
        case .valid:
            write(Self.manifest(name: fixture.folderName))
        }
        return folder.path
    }

    private func removeFixtures() {
        let unreadable = fixtureRoot.appendingPathComponent(Fixture.unreadable.folderName).appendingPathComponent("stellar.yaml")
        try? FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: unreadable.path)
        try? FileManager.default.removeItem(at: fixtureRoot)
    }

    private func projectCount(_ host: RuntimeHost) -> UInt64 {
        CoreRuntime.state(handle: host.handle)?.project_count ?? .max
    }

    // MARK: - 대응표

    private func checkMappingTable() {
        let codes: [(Int32, ShellProjectOpenFailure)] = [
            (STELLAR_FAILURE_NOT_A_REPOSITORY, .notARepository),
            (STELLAR_FAILURE_NOT_A_STELLAR_PROJECT, .notAStellarProject),
            (STELLAR_FAILURE_MANIFEST_UNREADABLE, .manifestUnreadable),
            (STELLAR_FAILURE_MANIFEST_CORRUPT, .manifestCorrupt),
            (STELLAR_FAILURE_MANIFEST_INVALID, .manifestInvalid),
            (STELLAR_FAILURE_INVALID_PROJECT_ID, .invalidProjectID),
            (STELLAR_FAILURE_UNTRUSTED_SYMLINK, .untrustedSymlink),
            (STELLAR_FAILURE_PATH_UNRESOLVED, .pathUnresolved),
            (STELLAR_FAILURE_WINDOW_SESSION_LOAD_FAILED, .designUnavailable),
        ]
        for (code, expected) in codes {
            expect(ShellProjectOpenFailure(coreFailure: UInt32(code)) == expected, "실패 코드 \(code) 대응 불일치")
        }
        expect(
            ShellProjectOpenFailure(coreFailure: UInt32(STELLAR_FAILURE_NOT_A_REPOSITORY), itemExists: false) == .folderMissing,
            "폴더가 없을 때 NOT_A_REPOSITORY가 '폴더 없음'이 아님"
        )
        expect(ShellProjectOpenFailure(coreFailure: 999) == .unknown(code: 999), "모르는 실패 코드가 unknown이 아님")
        // 프로젝트 열기와 무관한 문서 session 실패 코드도 unknown(일반 안내)입니다.
        expect(ShellProjectOpenFailure(coreFailure: UInt32(STELLAR_FAILURE_UNKNOWN_PROJECT)) == .unknown(code: UInt32(STELLAR_FAILURE_UNKNOWN_PROJECT)),
               "열기와 무관한 코드가 특정 안내로 대응됨")
        let statuses: [(StellarStatus, ShellProjectOpenFailure)] = [
            (STELLAR_INVALID_ARGUMENT, .invalidPath), (STELLAR_INVALID_UTF8, .invalidPath),
            (STELLAR_QUEUE_FULL, .busy), (STELLAR_INVALID_HANDLE, .runtimeUnavailable), (STELLAR_INTERNAL, .runtimeUnavailable),
        ]
        for (status, expected) in statuses {
            expect(ShellProjectOpenFailure(callStatus: status) == expected, "호출 상태 \(status.rawValue) 대응 불일치")
        }

        // 문구: 원인마다 제목이 다르고, 폴더 관련 원인은 폴더 이름을 보여 줍니다.
        let all: [ShellProjectOpenFailure] = codes.map(\.1) + [
            .folderMissing, .invalidPath, .busy, .runtimeUnavailable, .unknown(code: 999),
        ]
        let path = "/Volumes/Data/Projects/Example Folder"
        let titles = all.map(\.title)
        expect(Set(titles).count == titles.count, "안내 제목이 원인마다 다르지 않음: \(titles)")
        for failure in all {
            let message = failure.message(path: path)
            expect(!failure.title.isEmpty && !message.isEmpty, "\(failure) 안내 문구 없음")
            expect(!message.contains("/Volumes/Data/Projects"), "\(failure) 안내가 전체 경로를 노출함")
            switch failure {
            case .invalidPath, .busy, .runtimeUnavailable:
                break
            default:
                expect(message.contains("Example Folder"), "\(failure) 안내에 폴더 이름이 없음")
            }
        }
        expect(ShellProjectOpenFailure.notAStellarProject.welcomeNotice(path: path) == .notStellarProject(path: path),
               "'stellar.yaml 없음' 시작 창 안내 종류 불일치")
        expect(ShellProjectOpenFailure.folderMissing.welcomeNotice(path: path) == .projectMissing(path: path),
               "'폴더 없음' 시작 창 안내 종류 불일치")
        if case .operationFailed(let reason) = ShellProjectOpenFailure.manifestCorrupt.welcomeNotice(path: path) {
            expect(WelcomeStrings.failureReason(reason).hasPrefix(ShellProjectOpenFailure.manifestCorrupt.title),
                   "일반 시작 창 안내에 원인 제목이 없음")
        } else {
            expect(false, "손상 매니페스트 시작 창 안내가 operationFailed가 아님")
        }
    }

    // MARK: - 메뉴 Open Project

    private func checkMenuOpen(_ coordinator: ShellAppCoordinator, mainMenu: MainMenuBuilder.MainMenu, host: RuntimeHost) {
        guard let pump = coordinator.pump,
            let openItem = Self.item(.openProject, in: mainMenu.menu), let action = openItem.action
        else {
            expect(false, "Open Project 메뉴 항목 또는 펌프 없음")
            return
        }
        expect(openItem.keyEquivalent == "o" && openItem.keyEquivalentModifierMask == .command, "Open Project 단축키가 ⌘O가 아님")
        expect(openItem.title == "Open Project…", "Open Project 제목에 말줄임표 없음")

        // 선택 취소: 요청 없음.
        nextChosenFolder = nil
        NSApp.sendAction(action, to: openItem.target, from: openItem)
        expect(chooserCalls == 1, "⌘O가 폴더 선택을 열지 않음")
        expect(pump.pendingRequestCount == 0 && coordinator.projectWindows.isEmpty, "폴더 선택 취소 후 열기 요청이 감")

        // 폴더 선택 → 코어 열기 → 창.
        nextChosenFolder = make(.valid)
        NSApp.sendAction(action, to: openItem.target, from: openItem)
        expect(pump.pendingRequestCount == 1, "폴더를 골랐는데 코어 열기 요청이 없음")
        expect(coordinator.projectWindows.isEmpty, "코어 probe 전에 창을 만듦")
        expect(pump.waitForPendingRequests(timeout: 10), "열기 완료 event가 오지 않음")
        expect(coordinator.projectWindows.count == 1, "열기 성공 후 창이 없음")
        expect(coordinator.projectWindows.first?.dataSource.session?.name == Fixture.valid.folderName, "열린 session 이름 불일치")
        expect(projectCount(host) == 1, "코어 프로젝트 session 수가 1이 아님")
        expect(alerts.isEmpty, "성공한 열기에서 alert가 뜸")
        coordinator.projectWindows.first?.window?.close()
        _ = pump.waitForPendingRequests(timeout: 10)
        expect(projectCount(host) == 0, "창을 닫은 뒤 코어 session이 남음")
    }

    // MARK: - 잘못된 폴더

    private func checkCoreFailures(_ coordinator: ShellAppCoordinator, host: RuntimeHost) {
        guard let pump = coordinator.pump else { return }
        for fixture in Fixture.allCases where fixture != .valid {
            guard let expected = fixture.expected else { continue }
            let path = make(fixture)

            // 코어 실패 코드(FFI 경로 그대로) → 안내 원인.
            var outcome: ShellProjectOpenFailure?
            let result = pump.openProject(path: path) { event in
                if case .requestFailed(_, let code, _) = event {
                    outcome = ShellAppCoordinator.failure(coreCode: code, path: path)
                }
            }
            expect((try? result.get()) != nil, "\(fixture.folderName): 열기 요청이 접수되지 않음")
            expect(pump.waitForPendingRequests(timeout: 10), "\(fixture.folderName): 완료 event가 오지 않음")
            expect(outcome == expected, "\(fixture.folderName): 안내 원인 \(String(describing: outcome)) ≠ 기대 \(expected)")

            // 메뉴에서 연 실패 → alert(원인 제목·폴더 이름), 창·session 없음.
            let alertsBefore = alerts.count
            coordinator.openProject(atPath: path, origin: .application)
            _ = pump.waitForPendingRequests(timeout: 10)
            expect(alerts.count == alertsBefore + 1, "\(fixture.folderName): 실패 alert가 없음")
            if let alert = alerts.last, alerts.count == alertsBefore + 1 {
                expect(alert.title == expected.title, "\(fixture.folderName): alert 제목 불일치 '\(alert.title)'")
                expect(alert.message.contains(fixture.folderName), "\(fixture.folderName): alert에 폴더 이름 없음")
                expect(!alert.message.contains(fixtureRoot.path), "\(fixture.folderName): alert가 전체 경로를 노출함")
            }
            expect(coordinator.projectWindows.isEmpty, "\(fixture.folderName): 실패했는데 창이 생김")
            expect(projectCount(host) == 0, "\(fixture.folderName): 실패했는데 코어 session이 생김")
        }

        // 상대·빈 경로는 코어가 요청을 받지 않습니다(INVALID_ARGUMENT → 경로 안내).
        for raw in ["relative/project", ""] {
            let result = CoreRuntime.openProject(handle: host.handle, path: raw)
            if case .failure(let error) = result {
                expect(ShellProjectOpenFailure(callStatus: error.status) == .invalidPath, "'\(raw)' 경로 거부가 경로 안내가 아님")
            } else {
                expect(false, "'\(raw)' 경로 열기 요청이 접수됨")
            }
        }
        expect(pump.pendingRequestCount == 0, "검사 끝에 대기 요청이 남음")
    }

    // MARK: - 시작 창·복원

    private func checkWelcomeAndRestore(_ coordinator: ShellAppCoordinator, host: RuntimeHost) {
        guard let pump = coordinator.pump else { return }
        coordinator.showWelcomeIfNeeded()
        let cases: [(Fixture, (WelcomeNotice.Kind?) -> Bool)] = [
            (.gitOnly, { if case .notStellarProject? = $0 { return true }; return false }),
            (.missing, { if case .projectMissing? = $0 { return true }; return false }),
            (.corrupt, {
                if case .operationFailed(let reason)? = $0 {
                    return WelcomeStrings.failureReason(reason).hasPrefix(ShellProjectOpenFailure.manifestCorrupt.title)
                }
                return false
            }),
        ]
        for (fixture, matches) in cases {
            let alertsBefore = alerts.count
            coordinator.welcomeData.send(.openProject(directory: make(fixture)))
            _ = pump.waitForPendingRequests(timeout: 10)
            let notice = coordinator.welcomeData.currentSnapshot().notice
            expect(matches(notice?.kind), "\(fixture.folderName): 시작 창 안내 불일치 \(String(describing: notice?.kind))")
            expect(alerts.count == alertsBefore, "\(fixture.folderName): 시작 창에서 연 실패가 alert로 뜸")
            expect(coordinator.welcomeData.currentSnapshot().activity == nil, "\(fixture.folderName): 실패 후 진행 표시가 남음")
            if let id = notice?.id {
                coordinator.welcomeData.send(.dismissNotice(id: id))
            }
        }

        // 복원: 없어진 폴더는 창의 실패 상태로 원인 제목·해결 방법을 보여 줍니다.
        let missing = make(.missing)
        let archiver = NSKeyedArchiver(requiringSecureCoding: true)
        archiver.encode(missing as NSString, forKey: ShellAppCoordinator.restorationRootPathKey)
        archiver.finishEncoding()
        if let state = try? NSKeyedUnarchiver(forReadingFrom: archiver.encodedData),
            coordinator.restoreProjectWindow(from: state) != nil, let window = coordinator.projectWindows.first
        {
            _ = pump.waitForPendingRequests(timeout: 10)
            if case .failed(let title, let message, _) = window.dataSource.currentSnapshot.status {
                expect(title == ShellProjectOpenFailure.folderMissing.title, "복원 실패 제목 불일치: \(title)")
                expect(message == ShellProjectOpenFailure.folderMissing.message(path: missing), "복원 실패 안내 불일치")
            } else {
                expect(false, "없어진 폴더 복원이 실패 상태가 아님")
            }
            window.window?.close()
        } else {
            expect(false, "복원 창을 만들지 않음")
        }
        expect(projectCount(host) == 0, "검사 끝에 코어 session이 남음")
    }

    private static func item(_ command: ShellCommand, in menu: NSMenu) -> NSMenuItem? {
        for entry in menu.items {
            if let identifier = entry.identifier, ShellCommand(menuItemIdentifier: identifier) == command, !entry.isHidden {
                return entry
            }
            if let submenu = entry.submenu, let found = item(command, in: submenu) {
                return found
            }
        }
        return nil
    }
}
