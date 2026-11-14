import Foundation

/// 프로젝트 열기 실패 원인과 사용자 안내(XII.6.2 "invalid repo 안내 error mapping").
///
/// 코어 `REQUEST_FAILED`의 고정 실패 코드(`STELLAR_FAILURE_*`)와 FFI 호출 상태(`StellarStatus`)를 셸 문구로 바꿉니다.
/// 코어 실패 메시지(한국어 진단 문자열, 경로 포함 가능)는 화면에 보이지 않고 코드만 씁니다. 문구는 문제와 해결 방법을
/// 함께 말합니다(가이드 7.1, HIG Writing). `stellar.yaml`이 없는 Git 저장소는 전환하지 않고 안내만 합니다(가이드 7.2).
enum ShellProjectOpenFailure: Error, Equatable {
    /// 폴더에 `.git`이 없습니다(`STELLAR_FAILURE_NOT_A_REPOSITORY`, 폴더는 있음).
    case notARepository
    /// 폴더 자체가 없습니다(`NOT_A_REPOSITORY`이면서 셸이 확인한 경로에 항목 없음 — 옮겨졌거나 지워진 최근·복원
    /// 프로젝트). 경로에 일반 파일이 있으면 `notARepository`입니다.
    case folderMissing
    /// Git 저장소지만 `stellar.yaml`이 없습니다(`STELLAR_FAILURE_NOT_A_STELLAR_PROJECT`).
    case notAStellarProject
    /// `stellar.yaml`을 읽지 못했습니다(`STELLAR_FAILURE_MANIFEST_UNREADABLE`).
    case manifestUnreadable
    /// `stellar.yaml` YAML 문법 오류·크기 초과(`STELLAR_FAILURE_MANIFEST_CORRUPT`).
    case manifestCorrupt
    /// `stellar.yaml` 스키마 위반(`STELLAR_FAILURE_MANIFEST_INVALID`).
    case manifestInvalid
    /// `project.id`가 UUID가 아닙니다(`STELLAR_FAILURE_INVALID_PROJECT_ID`).
    case invalidProjectID
    /// 신뢰되지 않은 프로젝트의 `stellar.yaml`이 심볼릭 링크입니다(`STELLAR_FAILURE_UNTRUSTED_SYMLINK`).
    case untrustedSymlink
    /// 프로젝트 경로를 해석하지 못했습니다(`STELLAR_FAILURE_PATH_UNRESOLVED`).
    case pathUnresolved
    /// 저장된 디자인(current generation·작업 문서)을 로드하지 못했습니다(`STELLAR_FAILURE_WINDOW_SESSION_LOAD_FAILED`).
    case designUnavailable
    /// 경로가 비었거나 절대 경로가 아니거나 NUL을 포함합니다(`STELLAR_INVALID_ARGUMENT`, `STELLAR_INVALID_UTF8`).
    case invalidPath
    /// 대기 요청이 가득 찼습니다(`STELLAR_QUEUE_FULL`).
    case busy
    /// 코어 runtime을 쓸 수 없습니다(생성 실패, `STELLAR_INVALID_HANDLE`, 내부 오류).
    case runtimeUnavailable
    /// 이 셸이 모르는 코어 실패 코드(새 코어와 오래된 셸).
    case unknown(code: UInt32)
    /// 열기 요청에 다른 종류의 완료 event가 왔습니다.
    case unexpectedResponse

    /// 코어 실패 코드에서 원인을 만듭니다. `itemExists`는 `NOT_A_REPOSITORY`일 때만 씁니다.
    init(coreFailure code: UInt32, itemExists: @autoclosure () -> Bool = true) {
        switch Int32(bitPattern: code) {
        case STELLAR_FAILURE_NOT_A_REPOSITORY: self = itemExists() ? .notARepository : .folderMissing
        case STELLAR_FAILURE_NOT_A_STELLAR_PROJECT: self = .notAStellarProject
        case STELLAR_FAILURE_MANIFEST_UNREADABLE: self = .manifestUnreadable
        case STELLAR_FAILURE_MANIFEST_CORRUPT: self = .manifestCorrupt
        case STELLAR_FAILURE_MANIFEST_INVALID: self = .manifestInvalid
        case STELLAR_FAILURE_INVALID_PROJECT_ID: self = .invalidProjectID
        case STELLAR_FAILURE_UNTRUSTED_SYMLINK: self = .untrustedSymlink
        case STELLAR_FAILURE_PATH_UNRESOLVED: self = .pathUnresolved
        case STELLAR_FAILURE_WINDOW_SESSION_LOAD_FAILED: self = .designUnavailable
        default: self = .unknown(code: code)
        }
    }

    /// FFI 호출이 요청을 받지 않은 상태에서 원인을 만듭니다.
    init(callStatus status: StellarStatus) {
        switch status {
        case STELLAR_INVALID_ARGUMENT, STELLAR_INVALID_UTF8, STELLAR_NULL_POINTER: self = .invalidPath
        case STELLAR_QUEUE_FULL: self = .busy
        default: self = .runtimeUnavailable
        }
    }

    /// 경로에 항목(폴더·파일)이 있는지. `NOT_A_REPOSITORY` 실패 경로에서만 부릅니다(로컬 stat 한 번).
    ///
    /// 폴더가 아닌 일반 파일(Finder·`open` 명령으로 받은 파일)도 있음으로 셉니다. 있는 파일을 "폴더가 더는 없음"으로
    /// 안내하지 않고, 코어 코드대로 "Git 저장소 아님"(최상위 폴더를 고르라는 해결 방법)으로 안내합니다.
    static func itemExists(atPath path: String) -> Bool {
        FileManager.default.fileExists(atPath: path)
    }

    /// alert·실패 상태 제목.
    var title: String {
        switch self {
        case .notARepository:
            return String(localized: "This Folder Isn’t a Git Repository", comment: "Project open failure title")
        case .folderMissing:
            return String(localized: "Project Folder Not Found", comment: "Project open failure title")
        case .notAStellarProject:
            return String(localized: "This Folder Isn’t a Stellar Project", comment: "Project open failure title")
        case .manifestUnreadable:
            return String(localized: "The Project File Couldn’t Be Read", comment: "Project open failure title")
        case .manifestCorrupt:
            return String(localized: "The Project File Is Damaged", comment: "Project open failure title")
        case .manifestInvalid:
            return String(localized: "The Project File Isn’t Valid", comment: "Project open failure title")
        case .invalidProjectID:
            return String(localized: "The Project ID Isn’t Valid", comment: "Project open failure title")
        case .untrustedSymlink:
            return String(localized: "The Project File Is a Symbolic Link", comment: "Project open failure title")
        case .pathUnresolved:
            return String(localized: "The Project Folder Couldn’t Be Located", comment: "Project open failure title")
        case .designUnavailable:
            return String(localized: "The Project’s Design Couldn’t Be Loaded", comment: "Project open failure title")
        case .invalidPath:
            return String(localized: "The Folder Path Isn’t Valid", comment: "Project open failure title")
        case .busy:
            return String(localized: "Stellar Is Busy", comment: "Project open failure title")
        case .runtimeUnavailable:
            return String(localized: "Stellar Can’t Open Projects Right Now", comment: "Project open failure title")
        case .unknown, .unexpectedResponse:
            return String(localized: "The Project Couldn’t Be Opened", comment: "Project open failure title")
        }
    }

    /// 문제와 해결 방법. `path`는 고른 폴더 경로입니다(폴더 이름만 보여 줌).
    func message(path: String) -> String {
        let name = ShellProjectFrameDataSource.folderName(path)
        switch self {
        case .notARepository:
            return String(
                localized: "“\(name)” isn’t a Git repository. A Stellar project is a Git repository with a stellar.yaml file in its top-level folder. Choose the project’s top-level folder.",
                comment: "Project open failure message; the argument is the folder name"
            )
        case .folderMissing:
            return String(
                localized: "“\(name)” no longer exists. It may have been moved, renamed, or deleted. Locate the folder, or open another project.",
                comment: "Project open failure message"
            )
        case .notAStellarProject:
            return String(
                localized: "“\(name)” doesn’t contain stellar.yaml. Stellar opens Stellar projects only and doesn’t convert existing repositories.",
                comment: "Project open failure message"
            )
        case .manifestUnreadable:
            return String(
                localized: "Stellar couldn’t read stellar.yaml in “\(name)”. Check that you have permission to read the folder, then try again.",
                comment: "Project open failure message"
            )
        case .manifestCorrupt:
            return String(
                localized: "stellar.yaml in “\(name)” isn’t valid YAML or is too large to read. Restore the file from your Git history, then try again.",
                comment: "Project open failure message"
            )
        case .manifestInvalid:
            return String(
                localized: "stellar.yaml in “\(name)” is missing required settings or has values this version of Stellar doesn’t support.",
                comment: "Project open failure message"
            )
        case .invalidProjectID:
            return String(
                localized: "The project.id value in stellar.yaml in “\(name)” must be a UUID. Restore the file from your Git history, then try again.",
                comment: "Project open failure message"
            )
        case .untrustedSymlink:
            return String(
                localized: "stellar.yaml in “\(name)” is a symbolic link. For your safety, Stellar doesn’t follow links in projects you haven’t trusted. Replace the link with the file itself.",
                comment: "Project open failure message"
            )
        case .pathUnresolved:
            return String(
                localized: "Stellar couldn’t resolve the location of “\(name)”. It may have been moved or deleted while opening.",
                comment: "Project open failure message"
            )
        case .designUnavailable:
            return String(
                localized: "Stellar couldn’t load the saved design in “\(name)”. Files in its design folder may be missing or damaged. Restore the folder from your Git history, then try again.",
                comment: "Project open failure message"
            )
        case .invalidPath:
            return String(
                localized: "Choose a folder on this Mac and try again.",
                comment: "Project open failure message"
            )
        case .busy:
            return String(
                localized: "Too many requests are waiting. Wait a moment, then try again.",
                comment: "Project open failure message"
            )
        case .runtimeUnavailable:
            return String(
                localized: "Quit and reopen Stellar, then try again.",
                comment: "Project open failure message"
            )
        case .unknown, .unexpectedResponse:
            return String(
                localized: "“\(name)” couldn’t be opened as a Stellar project. Try again, or open another project.",
                comment: "Project open failure message"
            )
        }
    }

    /// 시작 창 안내 종류. 시작 창이 이미 문구를 가진 경우(`stellar.yaml` 없음, 폴더 없음)는 그 종류를 씁니다.
    func welcomeNotice(path: String) -> WelcomeNotice.Kind {
        switch self {
        case .notAStellarProject: return .notStellarProject(path: path)
        case .folderMissing: return .projectMissing(path: path)
        default: return .operationFailed(.projectOpenFailed(self, path: path))
        }
    }
}
