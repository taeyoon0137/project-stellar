import Foundation

/// `stellar` 명령 설치기를 headless로 검증합니다(`Stellar --command-install-check`, `scripts/macos/smoke.sh`가 실행).
///
/// - `StellarCommandInstaller`를 메모리 파일 시스템(`StellarCommandFileSystemStub`)으로 검사합니다(XXIV.5.3~XXIV.5.5
///   fixture): 위치 결정, 대체 이름 규칙(설치 위치·PATH 앞쪽·순서 모르는 폴더), 설치·멱등·제거, 남의 파일 보존,
///   다시 연결, 확인과 실행 사이 경합, 동의 없는 파일 변경 0, 상태 확인의 메인 스레드 파일 조회 0.
/// - 실제 파일 시스템 구현은 임시 폴더 안에서만 검사합니다(RENAME_SWAP 교체, symlink EEXIST, 경합 시 원래 항목 보존).
/// 사용자 위치(~/.local/bin 등)에는 쓰지 않습니다. 설치 화면은 UI 재설계 때 다시 만듭니다(ADR-0098).
enum CommandInstallCheck {
    static func run() -> Bool {
        let checker = Checker()

        checkInstallLocation(checker)
        checkInstaller(checker)
        checkSearchPath(checker)
        checkInstallerOnDisk(checker)
        checkInstallerThreading(checker)

        // 실제 앱 번들 상태는 환경마다 다르므로 보고만 합니다(XXIV.5.1 전 빌드에는 helper가 없음).
        let realStatus = StellarCommandInstaller.forRunningApp().status()
        checker.note("running app command status=\(describe(realStatus))")

        if checker.failures.isEmpty {
            print("command-install ok: \(checker.passed) checks [\(checker.notes.joined(separator: "; "))]")
            return true
        }
        for failure in checker.failures {
            FileHandle.standardError.write(Data("command-install 실패: \(failure)\n".utf8))
        }
        return false
    }

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

    // MARK: 설치기(XXIV.5.3~XXIV.5.5 fixture)

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
            checker.expect(
                plan.linkPath == link && plan.action == .replaceOwnLink(previousDestination: old, previousAppExists: false),
                "R1 다시 연결 계획 불일치: \(plan.action)")
            checker.expect(installer.install(plan).isSuccess, "R1 다시 연결 실패")
            checker.expect(fileSystem.nodes[link] == .symbolicLink(destination: stubHelper), "R1 다시 연결 대상 불일치")
            // 제거는 다시 연결한 링크(이 앱 helper)만 합니다. 상대 경로 링크는 아직 없는 앱을 가리키므로 남고, 다음 확인에서
            // 현재 대상(링크 폴더 기준으로 푼 절대 경로)을 보여 주는 Update 동의 대상이 됩니다.
            let removed = installer.uninstall()
            checker.expect(removed.removedPaths == [link] && fileSystem.nodes[link] == nil, "R1 다시 연결한 링크만 제거하지 않음: \(removed)")
            if case .needsRepair(let next) = installer.status() {
                checker.expect(
                    next.linkPath == fallbackLink
                        && next.action == .replaceOwnLink(
                            previousDestination: stubHome + "/Apps/Stellar.app/Contents/Helpers/stellar", previousAppExists: false),
                    "R1 상대 경로 링크의 현재 대상을 풀지 않음: \(next.action)")
            } else {
                checker.expect(false, "R1 제거 뒤 남은 상대 경로 링크가 Update 대상이 아님: \(installer.status())")
            }
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

        // R6: Remove Command는 이 앱 helper를 가리키는 링크만 지웁니다. 번들이 없어 소유를 확인하지 못한(dangling)
        // 링크와 다른 Stellar 사본의 링크는 동의 화면에 보인 적이 없으므로 남깁니다.
        fixture {
            let fileSystem = makeFileSystem()
            fileSystem.addDirectory(localBin, writable: true)
            fileSystem.addDirectory(homeBin, writable: true)
            let gone = "/Applications/Gone.app/Contents/Helpers/stellar"
            let beta = "/Applications/Stellar Beta.app"
            fileSystem.addFile(beta + "/Contents/Helpers/stellar", executable: true)
            fileSystem.bundleIdentifiers[beta] = ownerBundleID
            fileSystem.addSymbolicLink(link, destination: stubHelper)
            fileSystem.addSymbolicLink(fallbackLink, destination: gone)
            fileSystem.addSymbolicLink(homeBin + "/stellar", destination: beta + "/Contents/Helpers/stellar")
            let installer = makeInstaller(fileSystem)
            checker.expect(
                installer.status() == .installed(StellarCommandInstallation(linkPath: link, directoryOnSearchPath: false)),
                "R6 설치 상태 불일치: \(installer.status())")
            let removed = installer.uninstall()
            checker.expect(removed == StellarCommandUninstallResult(removedPaths: [link], failures: []), "R6 제거 결과 불일치: \(removed)")
            checker.expect(
                fileSystem.nodes[fallbackLink] == .symbolicLink(destination: gone)
                    && fileSystem.nodes[homeBin + "/stellar"] == .symbolicLink(destination: beta + "/Contents/Helpers/stellar"),
                "R6 동의 화면에 보이지 않은 링크를 지움")
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
        let queue = DispatchQueue(label: "xyz.taeyoon.stellar.command-install-check.status")

        var evaluated: StellarCommandInstallStatus?
        StellarCommandStatusCheck.evaluate(on: queue, makeInstaller: { installer }) { _, status in evaluated = status }
        checker.expect(spin(until: { evaluated != nil }), "thread: 상태 확인 결과가 오지 않음")
        checker.expect(base.mutationCount == 0, "thread: 확인이 파일을 바꿈")
        checker.expect(audit.mainThreadOperations.isEmpty, "thread: 메인 스레드 파일 조회 \(audit.mainThreadOperations)")
        checker.expect(audit.backgroundOperationCount > 0, "thread: 검사 대상 호출이 없음")
        checker.note("main-thread fs calls \(audit.mainThreadOperations.count), background \(audit.backgroundOperationCount)")
    }
}

private extension Result {
    var isSuccess: Bool {
        if case .success = self { return true }
        return false
    }
}
