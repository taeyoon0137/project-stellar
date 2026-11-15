import AppKit

/// macOS 셸의 application delegate(II.5.3).
///
/// 코어 runtime은 `RuntimeHost`가 소유합니다. 시작 시 1회 생성하고,
/// 마지막 창이 닫혀 앱이 종료될 때 `applicationWillTerminate`에서 정확히 1회 파괴합니다.
///
/// 창 구성은 `ShellAppCoordinator`가 맡습니다: 메인 메뉴 설치, 창 복원 진입점 등록, 열린 프로젝트가 없을 때 시작 창,
/// 프로젝트를 열면 셸 프레임 프로젝트 창(`ShellWindowController`)입니다.
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let runtimeHost = RuntimeHost()
    private var coordinator: ShellAppCoordinator?
    #if DEBUG
    convenience init(checking coordinator: ShellAppCoordinator) {
        self.init()
        self.coordinator = coordinator
    }
    #endif
    /// 앱 업데이트(XVI.9.5, ADR-0017). 구성이 완전한 빌드에서만 Sparkle updater를 시작합니다.
    private var updateController: AppUpdateController?

    func applicationWillFinishLaunching(_ notification: Notification) {
        // 메뉴와 창 복원 진입점은 AppKit이 창을 복원하기 전(applicationDidFinishLaunching 전)에 준비합니다.
        MainActor.assumeIsolated {
            let coordinator = ShellAppCoordinator(runtimeHandle: runtimeHost.created ? runtimeHost.handle : nil)
            coordinator.installMainMenu(on: NSApplication.shared)
            coordinator.registerWindowRestoration()
            self.coordinator = coordinator
        }
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        // 시작 시 FFI로 코어 버전을 읽어 로그에 기록(II.5.2).
        NSLog("stellar core version: %@", runtimeHost.version() ?? "<unavailable>")

        // 복원된 프로젝트 창이 없으면 시작 창을 보여 줍니다.
        MainActor.assumeIsolated {
            coordinator?.showWelcomeIfNeeded()
        }

        // 업데이트는 background에서 확인만 하고, 다운로드·설치는 사용자가 승인한 뒤에만 합니다(XVI.9.5).
        let updates = AppUpdateController()
        NSLog("stellar update: %@", updates.summary)
        updateController = updates
    }

    /// 창 상태 복원은 secure coding만 씁니다(프로젝트 폴더 경로 문자열 하나).
    func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
        true
    }

    /// Finder·`open` 명령으로 받은 폴더를 프로젝트로 엽니다.
    func application(_ application: NSApplication, open urls: [URL]) {
        MainActor.assumeIsolated {
            for url in urls where url.isFileURL {
                coordinator?.openProject(atPath: url.path, origin: .application)
            }
        }
    }

    /// Dock 아이콘을 눌렀는데 보이는 창이 없으면 시작 창을 보여 줍니다.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag {
            MainActor.assumeIsolated {
                coordinator?.showWelcomeIfNeeded()
            }
        }
        return true
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        MainActor.assumeIsolated {
            coordinator?.prepareToTerminate(confirmAgent: {
                let alert = ShellAppCoordinator.makeAgentSessionCloseAlert()
                alert.messageText = String(
                    localized: "Quit Stellar and Stop the Agent Session?",
                    comment: "Alert title shown before quitting with an active agent session"
                )
                alert.buttons.first?.title = String(localized: "Quit", comment: "Destructive alert button")
                return alert.runModal() == .alertFirstButtonReturn
            }, reply: { sender.reply(toApplicationShouldTerminate: $0) }) ?? .terminateNow
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        // 종료 시 runtime을 정확히 1회 파괴(중복 호출은 RuntimeHost가 무시).
        runtimeHost.shutdown()
    }
}
