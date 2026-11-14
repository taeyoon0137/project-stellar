import Foundation

/// 시작 창 영역의 화면 문구. 지역화 추출 대상이 되도록 `String(localized:)`로 둡니다.
///
/// 메뉴·버튼 제목은 title-style 대문자, 추가 입력이 필요한 명령은 말줄임표를 붙입니다(HIG The menu bar, Writing).
enum WelcomeStrings {
    /// 프로젝트 매니페스트 파일 이름(아키텍처 §25.2).
    static let manifestFileName = "stellar.yaml"

    // MARK: 창

    static let windowTitle = String(localized: "Welcome", comment: "Welcome window title")
    static let appName = String(localized: "Stellar", comment: "App name on the welcome window")
    static func version(_ version: String) -> String {
        String(localized: "Version \(version)", comment: "App version on the welcome window")
    }

    // MARK: 동작

    static let newProject = String(localized: "New Project…", comment: "Welcome action")
    static let openProject = String(localized: "Open Project…", comment: "Welcome action")
    static let cloneProject = String(localized: "Clone Project…", comment: "Welcome action")
    static let requiresGit = String(
        localized: "Requires Git. Install Git to use this command.",
        comment: "Tooltip for disabled actions when Git is missing"
    )

    // MARK: 최근 프로젝트

    static let recentProjects = String(localized: "Recent Projects", comment: "Accessibility label of the recents list")
    static let noRecentProjects = String(localized: "No Recent Projects", comment: "Empty state title")
    static let noRecentProjectsMessage = String(
        localized: "Projects you create, open, or clone appear here.",
        comment: "Empty state message"
    )
    static let loadingRecentProjects = String(localized: "Loading Recent Projects…", comment: "Loading state")
    static let recentsFailed = String(localized: "Couldn’t Load Recent Projects", comment: "Error state title")
    static let tryAgain = String(localized: "Try Again", comment: "Retry button")
    static let open = String(localized: "Open", comment: "Context menu item")
    static let showInFinder = String(localized: "Show in Finder", comment: "Context menu item")
    static let removeFromRecents = String(localized: "Remove from Recents", comment: "Context menu item")
    static let notFound = String(localized: "Not Found", comment: "Recent project whose folder is missing")
    static func missingProjectPath(_ path: String) -> String {
        String(localized: "Not found: \(path)", comment: "Path line of a missing recent project")
    }

    // MARK: Git 안내(아키텍처 §24.1: 안내만, 자동 설치 없음)

    static let gitNotFoundTitle = String(localized: "Git Isn’t Installed", comment: "Git notice title")
    static let gitNotFoundMessage = String(
        localized: "Stellar uses the Git on your Mac to create and clone projects, and won’t install it for you. To install Apple’s command line tools, which include Git, run this command in Terminal:",
        comment: "Git notice message"
    )
    static let gitUnusableTitle = String(localized: "Git Can’t Be Used", comment: "Git notice title")
    static let gitInstallCommand = "xcode-select --install"
    static let copyCommand = String(localized: "Copy Command", comment: "Accessibility label of a copy button")
    static let checkAgain = String(localized: "Check Again", comment: "Git notice button")
    static let locateGit = String(localized: "Locate Git…", comment: "Git notice button")
    static let locateGitMessage = String(
        localized: "Choose the Git program Stellar should use.",
        comment: "Open panel message"
    )

    // MARK: 진행 중 작업

    static func activity(_ kind: WelcomeActivity.Kind, name: String) -> String {
        switch kind {
        case .creating: return String(localized: "Creating “\(name)”…", comment: "Activity")
        case .opening: return String(localized: "Opening “\(name)”…", comment: "Activity")
        case .cloning: return String(localized: "Cloning “\(name)”…", comment: "Activity")
        }
    }

    static let cancel = String(localized: "Cancel", comment: "Button")

    // MARK: 패널

    static let newProjectPanelTitle = String(localized: "New Project", comment: "Save panel title")
    static let create = String(localized: "Create", comment: "Save panel prompt")
    static let projectName = String(localized: "Project Name:", comment: "Save panel name label")
    static let untitledProject = String(localized: "Untitled Project", comment: "Default new project name")
    static let openPanelMessage = String(localized: "Choose a Stellar project folder.", comment: "Open panel message")
    static func folderExists(_ name: String) -> String {
        String(
            localized: "A folder named “\(name)” already exists here. Choose a different name or location.",
            comment: "Save panel validation"
        )
    }

    // MARK: Clone sheet

    static let cloneTitle = String(localized: "Clone Project", comment: "Clone sheet title")
    static let cloneMessage = String(
        localized: "Clone an existing Stellar project. Stellar can open a cloned repository only if it contains a \(manifestFileName) file.",
        comment: "Clone sheet message"
    )
    static let repositoryURL = String(localized: "Repository URL:", comment: "Clone sheet field label")
    static let repositoryPlaceholder = String(
        localized: "https://example.com/team/project.git",
        comment: "Clone sheet URL placeholder"
    )
    static let folderName = String(localized: "Name:", comment: "Clone sheet field label")
    static let location = String(localized: "Location:", comment: "Clone sheet field label")
    static let choose = String(localized: "Choose…", comment: "Button")
    static let clone = String(localized: "Clone", comment: "Button")
    static let chooseLocationMessage = String(
        localized: "Choose the folder to clone into.",
        comment: "Open panel message"
    )

    // MARK: 안내 alert

    static let ok = String(localized: "OK", comment: "Button")
    static let notStellarProjectTitle = String(
        localized: "This Folder Isn’t a Stellar Project",
        comment: "Alert title"
    )
    static func notStellarProjectMessage(_ name: String) -> String {
        String(
            localized: "“\(name)” doesn’t contain \(manifestFileName). Stellar opens Stellar projects only and doesn’t convert existing repositories.",
            comment: "Alert message"
        )
    }
    static let projectMissingTitle = String(localized: "Project Not Found", comment: "Alert title")
    static func projectMissingMessage(_ path: String) -> String {
        String(
            localized: "The folder “\(path)” no longer exists. It may have been moved, renamed, or deleted.",
            comment: "Alert message"
        )
    }
    static let folderExistsTitle = String(localized: "A Folder with This Name Already Exists", comment: "Alert title")
    static func folderExistsMessage(_ path: String) -> String {
        String(localized: "“\(path)” already exists. Choose a different name or location.", comment: "Alert message")
    }
    static let gitIdentityTitle = String(localized: "Set Your Git Name and Email", comment: "Alert title")
    static let gitIdentityMessage = String(
        localized: "Git needs your name and email before Stellar can make the first commit. Set user.name and user.email with git config, then try again.",
        comment: "Alert message"
    )
    static let operationFailedTitle = String(localized: "The Operation Couldn’t Be Completed", comment: "Alert title")

    // MARK: 이유 코드 → 문구(코어는 문구가 아니라 코드를 보냄, 지역화 대비)

    static func failureReason(_ reason: WelcomeFailureReason) -> String {
        switch reason {
        case .permissionDenied(let path):
            return String(
                localized: "Stellar doesn’t have permission to use “\(path)”.",
                comment: "Failure reason: permission denied"
            )
        case .notEnoughSpace:
            return String(localized: "There isn’t enough free space on the disk.", comment: "Failure reason")
        case .networkUnavailable:
            return String(
                localized: "Stellar couldn’t connect to the remote repository. Check your network connection.",
                comment: "Failure reason"
            )
        case .authenticationFailed:
            return String(
                localized: "The remote repository requires you to sign in. Set up your Git credentials, then try again.",
                comment: "Failure reason"
            )
        case .remoteNotFound:
            return String(localized: "The remote repository wasn’t found. Check the URL.", comment: "Failure reason")
        case .gitCommandFailed(let status):
            return String(localized: "Git stopped with exit status \(status).", comment: "Failure reason")
        case .internalError(let code):
            return String(localized: "An unexpected error occurred. (Error code: \(code))", comment: "Failure reason")
        }
    }

    static func gitProblem(_ problem: WelcomeGitProblem) -> String {
        switch problem {
        case .versionTooOld(let found, let minimum):
            return String(
                localized: "Git \(found) is too old. Stellar needs Git \(minimum) or later.",
                comment: "Git problem"
            )
        case .notExecutable(let path):
            return String(localized: "“\(path)” isn’t a program Stellar can run.", comment: "Git problem")
        case .launchFailed:
            return String(localized: "Git couldn’t be started.", comment: "Git problem")
        }
    }

    // MARK: 접근성 알림·피드백

    static let copied = String(localized: "Copied", comment: "Feedback after copying a command")

    // MARK: `stellar` 명령 설치 sheet(ADR-0005 §2·§7, 계획 XXIV.5.6)

    /// 앱 메뉴 항목(관리 진입점). 추가 입력이 필요하므로 말줄임표를 붙입니다(HIG The menu bar).
    static let setUpCommandMenuItem = String(localized: "Set Up the stellar Command…", comment: "App menu item")
    static let checkingTitle = String(localized: "Checking the stellar Command…", comment: "Progress")
    static let commandInstallTitle = String(localized: "Install the stellar Command?", comment: "Sheet title")
    static let commandInstallMessage = String(
        localized: "Use Stellar from Terminal with the stellar command. Stellar adds a link to the command-line tool inside the app. No administrator password is needed, and you can remove the command at any time.",
        comment: "Sheet message"
    )
    static func fallbackInstallTitle(_ name: String) -> String {
        String(localized: "Install the Command as \(name)?", comment: "Sheet title")
    }
    static func fallbackInstallMessage(_ name: String) -> String {
        String(
            localized: "Stellar won’t change the other program, so it can install the command as \(name) instead. No administrator password is needed, and you can remove the command at any time.",
            comment: "Sheet message"
        )
    }
    /// 다른 프로그램이 이름을 쓰는 이유 한 문장(ADR-0005 §2).
    static func nameConflict(_ conflict: StellarCommandNameConflict, path: String) -> String {
        let name = conflict.commandName
        switch conflict.position {
        case .atInstallLocation:
            return String(
                localized: "Another program already uses the name \(name) at \(path).",
                comment: "Sheet message: name taken at the install location"
            )
        case .earlierOnPath:
            return String(
                localized: "Another program named \(name) at \(path) would run instead, because it comes first on your PATH.",
                comment: "Sheet message: name shadowed earlier on PATH"
            )
        case .possiblyEarlierOnPath:
            return String(
                localized: "Another program named \(name) is at \(path), which may come first on your PATH.",
                comment: "Sheet message: name possibly shadowed on PATH"
            )
        }
    }
    static func hidesPrograms(_ name: String, paths: String) -> String {
        String(
            localized: "After you install it, \(name) runs Stellar instead of \(paths).",
            comment: "Sheet note: the link will come first on PATH"
        )
    }
    static let commandLink = String(localized: "Link:", comment: "Sheet detail label")
    static let commandTarget = String(localized: "Points to:", comment: "Sheet detail label")
    static func createsFolder(_ path: String) -> String {
        String(localized: "Stellar will create the folder \(path).", comment: "Sheet detail")
    }
    static func pathUnconfirmed(_ path: String) -> String {
        String(
            localized: "Stellar couldn’t confirm that \(path) is on your PATH. If Terminal can’t find the command, add this line to your shell profile, such as ~/.zshrc:",
            comment: "Sheet PATH hint"
        )
    }
    static let notNow = String(localized: "Not Now", comment: "Button")
    static let install = String(localized: "Install", comment: "Button")
    static func installAs(_ name: String) -> String {
        String(localized: "Install as \(name)", comment: "Button")
    }
    static let update = String(localized: "Update", comment: "Button")
    static let done = String(localized: "Done", comment: "Button")
    static let close = String(localized: "Close", comment: "Button")
    static let checkAgainButton = String(localized: "Check Again", comment: "Button")
    static let removeCommand = String(localized: "Remove Command", comment: "Button")
    static let installing = String(localized: "Installing…", comment: "Progress")
    static let removing = String(localized: "Removing…", comment: "Progress")

    static func installedTitle(_ name: String) -> String {
        String(localized: "The \(name) Command Is Installed", comment: "Sheet title")
    }
    static func installedMessage(_ name: String) -> String {
        String(
            localized: "You can now run \(name) in a new Terminal window. You can remove the command at any time.",
            comment: "Sheet message"
        )
    }
    static func shadowedTitle(_ name: String) -> String {
        String(localized: "Another Program Runs Instead of \(name)", comment: "Sheet title")
    }
    static func shadowedFallbackMessage(_ name: String) -> String {
        String(localized: "Stellar can also install the command as \(name).", comment: "Sheet message")
    }
    static let shadowedNoFallbackMessage = String(
        localized: "The name stellar-cli is also in use, so Stellar didn’t install another name. Rename or remove one of the other programs, or remove this command.",
        comment: "Sheet message"
    )
    static let removedTitle = String(localized: "The Command Was Removed", comment: "Sheet title")
    static func removedMessage(_ paths: String) -> String {
        String(
            localized: "Stellar removed \(paths). Your projects and folders weren’t changed.",
            comment: "Sheet message"
        )
    }
    static let nothingToRemove = String(
        localized: "Stellar didn’t find a command link it created.",
        comment: "Sheet message"
    )
    static func repairTitle(_ name: String) -> String {
        String(localized: "Update the \(name) Command?", comment: "Sheet title")
    }
    static func repairMessage(_ name: String) -> String {
        String(
            localized: "The \(name) command links to another copy of Stellar, or to a copy that was moved. Update it to use this copy?",
            comment: "Sheet message"
        )
    }
    /// 링크 대상의 앱 번들이 있고 번들 식별자로 Stellar 사본임을 확인했을 때.
    static func repairOtherCopyMessage(_ name: String, appPath: String) -> String {
        String(
            localized: "The \(name) command links to another copy of Stellar at \(appPath). Update it to use this copy?",
            comment: "Sheet message: the link points to another verified copy of Stellar"
        )
    }
    /// 링크 대상의 앱 번들이 없어 어느 앱이 만든 링크인지 확인하지 못했을 때(다른 앱이 남긴 링크일 수 있음).
    static func repairMissingMessage(_ name: String, path: String) -> String {
        String(
            localized: "The \(name) command links to \(path), which no longer exists. Update it to use this copy of Stellar?",
            comment: "Sheet message: the link points to an app that no longer exists"
        )
    }
    static let namesUnavailableTitle = String(localized: "The stellar Command Wasn’t Installed", comment: "Sheet title")
    static let namesUnavailableMessage = String(
        localized: "Other programs already use both stellar and stellar-cli, so Stellar didn’t install anything. To use the command, rename or remove one of these programs, then choose Set Up the stellar Command again.",
        comment: "Sheet message"
    )
    static let helperUnavailableTitle = String(localized: "The stellar Command Isn’t Included", comment: "Sheet title")
    static let helperUnavailableMessage = String(
        localized: "This build of Stellar doesn’t include the stellar command, so it can’t be installed.",
        comment: "Sheet message"
    )
    static let locationTitle = String(localized: "Move Stellar to Your Applications Folder", comment: "Sheet title")
    static let locationMessage = String(
        localized: "Stellar is running from a temporary or read-only location. Move Stellar to your Applications folder, open it again, and then install the command.",
        comment: "Sheet message"
    )
    static let noLocationTitle = String(localized: "No Folder Available", comment: "Sheet title")
    static func noLocationMessage(_ path: String) -> String {
        String(
            localized: "Stellar couldn’t find a folder in your home folder that it can use without an administrator password. Create \(path), then try again.",
            comment: "Sheet message"
        )
    }
    static func installFailedTitle(_ name: String) -> String {
        String(localized: "The \(name) Command Couldn’t Be Installed", comment: "Sheet title")
    }
    static let removeFailedTitle = String(localized: "Some Links Couldn’t Be Removed", comment: "Sheet title")
    static func conflictMessage(_ path: String) -> String {
        String(
            localized: "Something changed at \(path) after Stellar checked it, so nothing was changed. Check again to see what you can do now.",
            comment: "Sheet message"
        )
    }
}
