import Darwin
import Foundation

// `stellar` 명령 PATH 설치·제거(제품 책임자 결정 2026-11-14: 첫 실행 시 설치 제안).
//
// 원칙
// - 명시적 동의 후에만 설치합니다. 이 타입은 상태 판정과 실행만 하고 묻는 것은 UI가 합니다.
// - 권한 상승을 쓰지 않습니다. 사용자가 이미 쓸 수 있는 폴더(또는 홈 아래 새 폴더)에만 링크를 만듭니다.
// - Stellar 소유 링크가 아닌 파일은 덮어쓰거나 지우지 않습니다.
// - 제거할 수 있습니다. 제거는 Stellar 소유 링크만 지웁니다.
// - 앱 번들 안 보조 실행 파일(ADR-0005 결정 7: Contents/Helpers/stellar-cli)이 없으면 설치하지 않습니다.
// 파일 I/O는 메인 스레드 밖에서 호출합니다(아키텍처 §6.4).

/// 경로 하나의 lstat 결과(마지막 심볼릭 링크를 따라가지 않음).
enum StellarCommandFileEntry: Equatable {
    case missing
    case symbolicLink(destination: String)
    case regularFile
    case directory
    /// 읽을 수 없거나 그 밖의 종류. 안전을 위해 남의 파일로 취급합니다.
    case other
}

/// 설치기가 쓰는 파일 시스템 연산. 검사에서는 메모리 구현으로 바꿉니다.
protocol StellarCommandFileSystem: AnyObject {
    /// 사용자 홈 디렉터리 절대 경로.
    var homeDirectory: String { get }
    func entry(atPath path: String) -> StellarCommandFileEntry
    /// 링크를 따라가 실행 가능한 일반 파일인지.
    func isExecutableFile(atPath path: String) -> Bool
    /// 링크를 따라가 현재 사용자가 쓸 수 있는 디렉터리인지.
    func isWritableDirectory(atPath path: String) -> Bool
    /// 경로가 읽기 전용 볼륨(예: 디스크 이미지)에 있는지.
    func isOnReadOnlyVolume(path: String) -> Bool
    /// 중간 디렉터리까지 현재 사용자 권한으로 만듭니다.
    func createDirectory(atPath path: String) throws
    /// 새 심볼릭 링크를 만듭니다. 이미 무엇이 있으면 실패해야 합니다(덮어쓰기 금지).
    func createSymbolicLink(atPath path: String, destination: String) throws
    /// 기존 심볼릭 링크를 원자적으로 새 링크로 바꿉니다. 호출자가 소유 여부를 먼저 확인합니다.
    func replaceSymbolicLink(atPath path: String, destination: String) throws
    /// 심볼릭 링크만 지웁니다. 링크가 아니면 실패해야 합니다.
    func removeSymbolicLink(atPath path: String) throws
    func contentsOfTextFile(atPath path: String) -> String?
    func contentsOfDirectory(atPath path: String) -> [String]
}

/// 파일 시스템 연산 실패.
struct StellarCommandFileSystemError: Error, Equatable {
    var path: String
    var message: String
}

/// 실제 파일 시스템 구현.
final class LocalStellarCommandFileSystem: StellarCommandFileSystem {
    private let fileManager = FileManager()

    var homeDirectory: String { NSHomeDirectory() }

    func entry(atPath path: String) -> StellarCommandFileEntry {
        var info = stat()
        guard lstat(path, &info) == 0 else {
            return errno == ENOENT ? .missing : .other
        }
        switch info.st_mode & S_IFMT {
        case S_IFLNK:
            guard let destination = try? fileManager.destinationOfSymbolicLink(atPath: path) else {
                return .other
            }
            return .symbolicLink(destination: destination)
        case S_IFREG: return .regularFile
        case S_IFDIR: return .directory
        default: return .other
        }
    }

    func isExecutableFile(atPath path: String) -> Bool {
        var isDirectory: ObjCBool = false
        return fileManager.fileExists(atPath: path, isDirectory: &isDirectory)
            && !isDirectory.boolValue
            && fileManager.isExecutableFile(atPath: path)
    }

    func isWritableDirectory(atPath path: String) -> Bool {
        var isDirectory: ObjCBool = false
        return fileManager.fileExists(atPath: path, isDirectory: &isDirectory)
            && isDirectory.boolValue
            && fileManager.isWritableFile(atPath: path)
    }

    func isOnReadOnlyVolume(path: String) -> Bool {
        let values = try? URL(fileURLWithPath: path).resourceValues(forKeys: [.volumeIsReadOnlyKey])
        return values?.volumeIsReadOnly ?? false
    }

    func createDirectory(atPath path: String) throws {
        do {
            try fileManager.createDirectory(atPath: path, withIntermediateDirectories: true)
        } catch {
            throw StellarCommandFileSystemError(path: path, message: error.localizedDescription)
        }
    }

    func createSymbolicLink(atPath path: String, destination: String) throws {
        // symlink(2)는 경로에 무엇이든 있으면 EEXIST로 실패하므로 덮어쓰지 않습니다.
        guard symlink(destination, path) == 0 else {
            throw StellarCommandFileSystemError(path: path, message: String(cString: strerror(errno)))
        }
    }

    func replaceSymbolicLink(atPath path: String, destination: String) throws {
        let directory = (path as NSString).deletingLastPathComponent
        let temporary = (directory as NSString).appendingPathComponent(".stellar-link-\(UUID().uuidString)")
        guard symlink(destination, temporary) == 0 else {
            throw StellarCommandFileSystemError(path: path, message: String(cString: strerror(errno)))
        }
        // 같은 폴더 안 rename(2)은 원자적입니다. 중간 상태에서 명령이 사라지지 않습니다.
        guard rename(temporary, path) == 0 else {
            let message = String(cString: strerror(errno))
            unlink(temporary)
            throw StellarCommandFileSystemError(path: path, message: message)
        }
    }

    func removeSymbolicLink(atPath path: String) throws {
        guard case .symbolicLink = entry(atPath: path) else {
            throw StellarCommandFileSystemError(path: path, message: "Not a symbolic link")
        }
        guard unlink(path) == 0 else {
            throw StellarCommandFileSystemError(path: path, message: String(cString: strerror(errno)))
        }
    }

    func contentsOfTextFile(atPath path: String) -> String? {
        try? String(contentsOfFile: path, encoding: .utf8)
    }

    func contentsOfDirectory(atPath path: String) -> [String] {
        (try? fileManager.contentsOfDirectory(atPath: path)) ?? []
    }
}

/// 검사용 메모리 파일 시스템(의도적 placeholder, 계획 1.2 규칙 6). 실제 디스크를 건드리지 않습니다.
final class StellarCommandFileSystemStub: StellarCommandFileSystem {
    enum Node: Equatable {
        case directory(writable: Bool)
        case file(executable: Bool)
        case symbolicLink(destination: String)
    }

    let homeDirectory: String
    private(set) var nodes: [String: Node] = [:]
    var readOnlyVolumePrefixes: [String] = []
    var textFiles: [String: String] = [:]
    /// 다음 링크 생성·교체를 이 메시지로 실패시킵니다(오류 상태 검사용).
    var failNextLinkOperation: String?

    init(homeDirectory: String = "/Users/stub") {
        self.homeDirectory = homeDirectory
        addDirectory("/", writable: false)
        addDirectory(homeDirectory, writable: true)
    }

    /// 중간 디렉터리를 쓰기 불가로 채우며 디렉터리를 추가합니다.
    func addDirectory(_ path: String, writable: Bool) {
        var current = ""
        for component in (path as NSString).pathComponents where component != "/" {
            current += "/" + component
            if nodes[current] == nil { nodes[current] = .directory(writable: false) }
        }
        nodes[path == "/" ? "/" : current] = .directory(writable: writable)
    }

    func addFile(_ path: String, executable: Bool) {
        addDirectory((path as NSString).deletingLastPathComponent, writable: isWritable(parentOf: path))
        nodes[path] = .file(executable: executable)
    }

    func addSymbolicLink(_ path: String, destination: String) {
        nodes[path] = .symbolicLink(destination: destination)
    }

    private func isWritable(parentOf path: String) -> Bool {
        if case .directory(let writable)? = nodes[(path as NSString).deletingLastPathComponent] {
            return writable
        }
        return false
    }

    private func resolved(_ path: String, depth: Int = 0) -> Node? {
        guard depth < 8, let node = nodes[path] else { return nil }
        if case .symbolicLink(let destination) = node {
            let base = (path as NSString).deletingLastPathComponent
            let target = destination.hasPrefix("/")
                ? destination : (base as NSString).appendingPathComponent(destination)
            return resolved(StellarCommandInstaller.lexicallyNormalized(target), depth: depth + 1)
        }
        return node
    }

    func entry(atPath path: String) -> StellarCommandFileEntry {
        switch nodes[path] {
        case nil: return .missing
        case .directory?: return .directory
        case .file?: return .regularFile
        case .symbolicLink(let destination)?: return .symbolicLink(destination: destination)
        }
    }

    func isExecutableFile(atPath path: String) -> Bool {
        resolved(path) == .file(executable: true)
    }

    func isWritableDirectory(atPath path: String) -> Bool {
        resolved(path) == .directory(writable: true)
    }

    func isOnReadOnlyVolume(path: String) -> Bool {
        readOnlyVolumePrefixes.contains { path.hasPrefix($0) }
    }

    func createDirectory(atPath path: String) throws {
        var missing: [String] = []
        var current = path
        while nodes[current] == nil {
            missing.insert(current, at: 0)
            current = (current as NSString).deletingLastPathComponent
        }
        guard resolved(current) == .directory(writable: true) else {
            throw StellarCommandFileSystemError(path: path, message: "Permission denied")
        }
        for directory in missing { nodes[directory] = .directory(writable: true) }
    }

    func createSymbolicLink(atPath path: String, destination: String) throws {
        if let message = failNextLinkOperation {
            failNextLinkOperation = nil
            throw StellarCommandFileSystemError(path: path, message: message)
        }
        guard nodes[path] == nil else {
            throw StellarCommandFileSystemError(path: path, message: "File exists")
        }
        guard isWritable(parentOf: path) else {
            throw StellarCommandFileSystemError(path: path, message: "Permission denied")
        }
        nodes[path] = .symbolicLink(destination: destination)
    }

    func replaceSymbolicLink(atPath path: String, destination: String) throws {
        if let message = failNextLinkOperation {
            failNextLinkOperation = nil
            throw StellarCommandFileSystemError(path: path, message: message)
        }
        guard isWritable(parentOf: path) else {
            throw StellarCommandFileSystemError(path: path, message: "Permission denied")
        }
        if case .directory? = nodes[path] {
            throw StellarCommandFileSystemError(path: path, message: "Is a directory")
        }
        nodes[path] = .symbolicLink(destination: destination)
    }

    func removeSymbolicLink(atPath path: String) throws {
        guard case .symbolicLink? = nodes[path] else {
            throw StellarCommandFileSystemError(path: path, message: "Not a symbolic link")
        }
        guard isWritable(parentOf: path) else {
            throw StellarCommandFileSystemError(path: path, message: "Permission denied")
        }
        nodes[path] = nil
    }

    func contentsOfTextFile(atPath path: String) -> String? {
        textFiles[path]
    }

    func contentsOfDirectory(atPath path: String) -> [String] {
        let prefix = path.hasSuffix("/") ? path : path + "/"
        let names = textFiles.keys.compactMap { key -> String? in
            guard key.hasPrefix(prefix) else { return nil }
            let rest = String(key.dropFirst(prefix.count))
            return rest.contains("/") ? nil : rest
        }
        return names.sorted()
    }
}

/// 설치할 링크 하나의 계획.
struct StellarCommandInstallPlan: Equatable {
    enum Action: Equatable {
        /// 빈 자리에 새 링크를 만듭니다.
        case create
        /// 이동했거나 다른 사본을 가리키는 Stellar 소유 링크를 이 앱으로 다시 연결합니다.
        case replaceOwnLink(previousDestination: String)
    }

    /// 만들 링크 경로(예: ~/.local/bin/stellar).
    var linkPath: String
    /// 링크 대상(앱 번들 안 보조 실행 파일).
    var destination: String
    var action: Action
    /// 링크 폴더를 새로 만들어야 하면 true.
    var createsDirectory: Bool
    /// 링크 폴더가 PATH에 있다고 확인했으면 true. 확인하지 못했으면 PATH 추가 안내를 보여 줍니다.
    var directoryOnSearchPath: Bool

    var directory: String { (linkPath as NSString).deletingLastPathComponent }
    var commandName: String { (linkPath as NSString).lastPathComponent }
}

/// 설치된 링크.
struct StellarCommandInstallation: Equatable {
    var linkPath: String
    var directoryOnSearchPath: Bool

    var directory: String { (linkPath as NSString).deletingLastPathComponent }
    var commandName: String { (linkPath as NSString).lastPathComponent }
}

/// 현재 설치 상태.
enum StellarCommandInstallStatus: Equatable {
    enum UnsupportedLocation: Equatable {
        /// Gatekeeper App Translocation 임시 경로에서 실행 중입니다.
        case translocated
        /// 디스크 이미지 같은 읽기 전용 볼륨에서 실행 중입니다.
        case readOnlyVolume
    }

    /// 앱 번들에 보조 실행 파일이 없습니다(ADR-0005 결정 7: 초기 릴리스에는 넣지 않음).
    case helperUnavailable(helperPath: String)
    /// 링크 대상이 곧 사라질 위치에서 실행 중입니다.
    case unsupportedAppLocation(UnsupportedLocation)
    /// 설치할 수 있습니다.
    case ready(StellarCommandInstallPlan)
    /// 이미 이 앱을 가리키는 링크가 있습니다.
    case installed(StellarCommandInstallation)
    /// Stellar 소유 링크가 다른 위치를 가리킵니다. 다시 연결할 수 있습니다.
    case needsRepair(StellarCommandInstallPlan)
    /// 기본 이름 자리에 Stellar가 만들지 않은 파일이 있습니다. 대체 이름 계획이 있으면 함께 줍니다.
    case blocked(conflictPath: String, fallback: StellarCommandInstallPlan?)
    /// 권한 상승 없이 쓸 수 있는 폴더가 없습니다.
    case noWritableLocation(suggestedDirectory: String)
}

/// 설치 실패.
enum StellarCommandInstallError: Error, Equatable {
    case helperUnavailable(helperPath: String)
    /// 실행 직전 다시 확인했을 때 Stellar 소유가 아닌 파일이 생겼습니다. 아무것도 바꾸지 않았습니다.
    case conflict(path: String)
    case fileSystem(StellarCommandFileSystemError)
}

/// 제거 결과. 일부만 지웠을 수 있으므로 둘 다 보고합니다.
struct StellarCommandUninstallResult: Equatable {
    var removedPaths: [String]
    var failures: [StellarCommandFileSystemError]
}

/// PATH 디렉터리 목록 구성.
enum StellarCommandSearchPath {
    /// 셸을 실행하지 않고 확인할 수 있는 PATH: `/etc/paths`, `/etc/paths.d/*`, 앱 프로세스의 `PATH`.
    ///
    /// 사용자의 셸 설정 파일(~/.zshrc 등)이 더하는 경로는 셸을 실행해야 알 수 있으므로 넣지 않습니다.
    /// 따라서 여기에 없는 폴더는 "PATH에 없음"이 아니라 "확인하지 못함"으로 다룹니다.
    static func system(
        fileSystem: StellarCommandFileSystem,
        environmentPath: String? = ProcessInfo.processInfo.environment["PATH"]
    ) -> [String] {
        var lines: [String] = []
        if let paths = fileSystem.contentsOfTextFile(atPath: "/etc/paths") {
            lines += paths.components(separatedBy: .newlines)
        }
        for name in fileSystem.contentsOfDirectory(atPath: "/etc/paths.d").sorted() {
            if let text = fileSystem.contentsOfTextFile(atPath: "/etc/paths.d/" + name) {
                lines += text.components(separatedBy: .newlines)
            }
        }
        if let environmentPath {
            lines += environmentPath.components(separatedBy: ":")
        }
        var seen = Set<String>()
        var result: [String] = []
        for line in lines {
            var entry = line.trimmingCharacters(in: .whitespaces)
            guard !entry.isEmpty, !entry.hasPrefix("#") else { continue }
            if entry == "~" || entry.hasPrefix("~/") {
                entry = fileSystem.homeDirectory + entry.dropFirst()
            }
            guard entry.hasPrefix("/") else { continue }
            let normalized = StellarCommandInstaller.lexicallyNormalized(entry)
            if seen.insert(normalized).inserted { result.append(normalized) }
        }
        return result
    }
}

/// `stellar` 명령 링크 설치기.
struct StellarCommandInstaller {
    /// 기본 명령 이름(제품 책임자 결정 2026-11-14).
    static let commandName = "stellar"
    /// 기본 이름 자리가 남의 파일일 때 쓰는 대체 이름.
    static let fallbackCommandName = "stellar-cli"
    /// 앱 번들 안 보조 실행 파일 상대 경로(ADR-0005 결정 7).
    static let helperRelativePath = "Contents/Helpers/stellar-cli"

    let appBundlePath: String
    let helperPath: String
    let fileSystem: StellarCommandFileSystem
    /// 확인된 PATH 디렉터리(정규화됨).
    let searchPath: [String]

    init(appBundlePath: String, fileSystem: StellarCommandFileSystem, searchPath: [String]) {
        let bundle = StellarCommandInstaller.lexicallyNormalized(appBundlePath)
        self.appBundlePath = bundle
        self.helperPath = (bundle as NSString).appendingPathComponent(StellarCommandInstaller.helperRelativePath)
        self.fileSystem = fileSystem
        self.searchPath = searchPath.map(StellarCommandInstaller.lexicallyNormalized)
    }

    /// 실행 중인 앱 번들과 실제 파일 시스템으로 만듭니다. PATH 파일을 읽으므로 메인 스레드 밖에서 부릅니다.
    static func forRunningApp() -> StellarCommandInstaller {
        let fileSystem = LocalStellarCommandFileSystem()
        return StellarCommandInstaller(
            appBundlePath: Bundle.main.bundlePath,
            fileSystem: fileSystem,
            searchPath: StellarCommandSearchPath.system(fileSystem: fileSystem)
        )
    }

    /// 링크 후보 폴더(선호 순). 홈 아래 폴더를 먼저 두어 다른 사용자 계정에 영향을 주지 않습니다.
    var candidateDirectories: [String] {
        let home = fileSystem.homeDirectory
        return [
            (home as NSString).appendingPathComponent(".local/bin"),
            (home as NSString).appendingPathComponent("bin"),
            "/usr/local/bin",
        ]
    }

    /// PATH에 추가하라고 안내할 셸 설정 한 줄.
    static func pathExportLine(for directory: String, homeDirectory: String) -> String {
        var shown = directory
        if directory == homeDirectory {
            shown = "$HOME"
        } else if directory.hasPrefix(homeDirectory + "/") {
            shown = "$HOME" + directory.dropFirst(homeDirectory.count)
        }
        return "export PATH=\"\(shown):$PATH\""
    }

    // MARK: 상태 판정

    func status() -> StellarCommandInstallStatus {
        guard fileSystem.isExecutableFile(atPath: helperPath) else {
            return .helperUnavailable(helperPath: helperPath)
        }
        if appBundlePath.contains("/AppTranslocation/") {
            return .unsupportedAppLocation(.translocated)
        }
        if fileSystem.isOnReadOnlyVolume(path: appBundlePath) {
            return .unsupportedAppLocation(.readOnlyVolume)
        }

        // 이미 있는 Stellar 소유 링크가 먼저입니다. 같은 링크를 두 군데 만들지 않습니다.
        let ownLinks = installedLinkPaths()
        for linkPath in ownLinks where pointsToHelper(linkPath) {
            return .installed(installation(at: linkPath))
        }
        if let stale = ownLinks.first, case .symbolicLink(let previous) = fileSystem.entry(atPath: stale) {
            return .needsRepair(
                StellarCommandInstallPlan(
                    linkPath: stale,
                    destination: helperPath,
                    action: .replaceOwnLink(previousDestination: previous),
                    createsDirectory: false,
                    directoryOnSearchPath: isOnSearchPath(directory(of: stale))
                )
            )
        }

        guard let target = preferredDirectory() else {
            return .noWritableLocation(suggestedDirectory: candidateDirectories[0])
        }
        let primary = (target.path as NSString).appendingPathComponent(StellarCommandInstaller.commandName)
        if fileSystem.entry(atPath: primary) == .missing {
            return .ready(plan(linkPath: primary, createsDirectory: target.needsCreation))
        }
        let fallback = (target.path as NSString).appendingPathComponent(StellarCommandInstaller.fallbackCommandName)
        let fallbackPlan = fileSystem.entry(atPath: fallback) == .missing
            ? plan(linkPath: fallback, createsDirectory: target.needsCreation) : nil
        return .blocked(conflictPath: primary, fallback: fallbackPlan)
    }

    /// 후보 폴더에 있는 Stellar 소유 링크 경로.
    func installedLinkPaths() -> [String] {
        var result: [String] = []
        for directory in candidateDirectories {
            for name in [StellarCommandInstaller.commandName, StellarCommandInstaller.fallbackCommandName] {
                let path = (directory as NSString).appendingPathComponent(name)
                if isStellarOwnedLink(atPath: path) { result.append(path) }
            }
        }
        return result
    }

    /// Stellar 소유 링크 판정: 심볼릭 링크이고 대상이 `<이름>.app/Contents/Helpers/stellar-cli` 형식입니다.
    /// 앱이 옮겨져 대상이 없어진(dangling) 링크도 Stellar 소유로 봅니다. 그 밖의 파일은 절대 바꾸지 않습니다.
    func isStellarOwnedLink(atPath path: String) -> Bool {
        guard case .symbolicLink(let destination) = fileSystem.entry(atPath: path) else { return false }
        let components = (resolvedDestination(destination, linkPath: path) as NSString).pathComponents
        guard components.count >= 5 else { return false }
        let tail = Array(components.suffix(4))
        return tail[0].hasSuffix(".app") && tail[0].count > 4
            && tail[1] == "Contents" && tail[2] == "Helpers"
            && tail[3] == (StellarCommandInstaller.helperRelativePath as NSString).lastPathComponent
    }

    // MARK: 실행

    /// 계획대로 링크를 만듭니다. 실행 직전에 다시 확인해 남의 파일은 건드리지 않습니다.
    func install(_ plan: StellarCommandInstallPlan) -> Result<StellarCommandInstallation, StellarCommandInstallError> {
        guard fileSystem.isExecutableFile(atPath: helperPath) else {
            return .failure(.helperUnavailable(helperPath: helperPath))
        }
        do {
            if fileSystem.entry(atPath: plan.directory) == .missing {
                try fileSystem.createDirectory(atPath: plan.directory)
            }
            switch fileSystem.entry(atPath: plan.linkPath) {
            case .missing:
                do {
                    try fileSystem.createSymbolicLink(atPath: plan.linkPath, destination: helperPath)
                } catch {
                    // 확인과 생성 사이에 다른 파일이 생겼으면 충돌로 보고합니다.
                    if fileSystem.entry(atPath: plan.linkPath) != .missing,
                        !isStellarOwnedLink(atPath: plan.linkPath)
                    {
                        return .failure(.conflict(path: plan.linkPath))
                    }
                    throw error
                }
            case .symbolicLink where isStellarOwnedLink(atPath: plan.linkPath):
                if !pointsToHelper(plan.linkPath) {
                    try fileSystem.replaceSymbolicLink(atPath: plan.linkPath, destination: helperPath)
                }
            default:
                return .failure(.conflict(path: plan.linkPath))
            }
        } catch let error as StellarCommandFileSystemError {
            return .failure(.fileSystem(error))
        } catch {
            return .failure(.fileSystem(StellarCommandFileSystemError(path: plan.linkPath, message: "\(error)")))
        }
        return .success(installation(at: plan.linkPath))
    }

    /// 후보 폴더의 Stellar 소유 링크를 모두 지웁니다. 만든 폴더(~/.local/bin 등)는 남깁니다.
    func uninstall() -> StellarCommandUninstallResult {
        var result = StellarCommandUninstallResult(removedPaths: [], failures: [])
        for path in installedLinkPaths() {
            // 지우기 직전에 다시 확인합니다.
            guard isStellarOwnedLink(atPath: path) else { continue }
            do {
                try fileSystem.removeSymbolicLink(atPath: path)
                result.removedPaths.append(path)
            } catch let error as StellarCommandFileSystemError {
                result.failures.append(error)
            } catch {
                result.failures.append(StellarCommandFileSystemError(path: path, message: "\(error)"))
            }
        }
        return result
    }

    // MARK: 내부

    private func plan(linkPath: String, createsDirectory: Bool) -> StellarCommandInstallPlan {
        StellarCommandInstallPlan(
            linkPath: linkPath,
            destination: helperPath,
            action: .create,
            createsDirectory: createsDirectory,
            directoryOnSearchPath: isOnSearchPath(directory(of: linkPath))
        )
    }

    private func installation(at linkPath: String) -> StellarCommandInstallation {
        StellarCommandInstallation(linkPath: linkPath, directoryOnSearchPath: isOnSearchPath(directory(of: linkPath)))
    }

    private func directory(of path: String) -> String {
        (path as NSString).deletingLastPathComponent
    }

    private func isOnSearchPath(_ directory: String) -> Bool {
        searchPath.contains(StellarCommandInstaller.lexicallyNormalized(directory))
    }

    private func pointsToHelper(_ linkPath: String) -> Bool {
        guard case .symbolicLink(let destination) = fileSystem.entry(atPath: linkPath) else { return false }
        return resolvedDestination(destination, linkPath: linkPath) == helperPath
    }

    private func resolvedDestination(_ destination: String, linkPath: String) -> String {
        let absolute = destination.hasPrefix("/")
            ? destination : (directory(of: linkPath) as NSString).appendingPathComponent(destination)
        return StellarCommandInstaller.lexicallyNormalized(absolute)
    }

    /// PATH에 있는 폴더를 먼저, 없으면 홈 아래 첫 후보를 고릅니다. 권한 상승이 필요한 폴더는 고르지 않습니다.
    private func preferredDirectory() -> (path: String, needsCreation: Bool)? {
        let usable = candidateDirectories.compactMap { directory -> (path: String, needsCreation: Bool)? in
            switch fileSystem.entry(atPath: directory) {
            case .missing:
                return canCreate(directory) ? (directory, true) : nil
            default:
                return fileSystem.isWritableDirectory(atPath: directory) ? (directory, false) : nil
            }
        }
        if let onPath = usable.first(where: { isOnSearchPath($0.path) }) {
            return onPath
        }
        let home = fileSystem.homeDirectory
        return usable.first { $0.path.hasPrefix(home + "/") }
    }

    /// 홈 아래이고, 가장 가까운 기존 상위 폴더에 쓸 수 있으면 만들 수 있습니다.
    private func canCreate(_ directory: String) -> Bool {
        guard directory.hasPrefix(fileSystem.homeDirectory + "/") else { return false }
        var current = (directory as NSString).deletingLastPathComponent
        while fileSystem.entry(atPath: current) == .missing, current != "/" {
            current = (current as NSString).deletingLastPathComponent
        }
        return fileSystem.isWritableDirectory(atPath: current)
    }

    /// 파일 시스템을 조회하지 않는 경로 정규화("."·".."·중복 구분자 제거).
    static func lexicallyNormalized(_ path: String) -> String {
        var stack: [String] = []
        for component in path.split(separator: "/", omittingEmptySubsequences: true) {
            switch component {
            case ".": continue
            case "..": if !stack.isEmpty { stack.removeLast() }
            default: stack.append(String(component))
            }
        }
        return "/" + stack.joined(separator: "/")
    }
}
