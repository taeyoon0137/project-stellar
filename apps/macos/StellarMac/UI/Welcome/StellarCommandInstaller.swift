import Darwin
import Foundation

// `stellar` 명령 PATH 설치·제거와 상태 확인(ADR-0005 §2·§7, 계획 XXIV.5.3~XXIV.5.5).
//
// 원칙
// - 명시적 동의 후에만 파일을 바꿉니다. 이 타입은 상태 판정과 실행만 하고 묻는 것은 UI가 합니다.
// - 최소 권한(아키텍처 §37): 권한 상승, 보조 도구 설치, 셸 실행을 쓰지 않습니다. 현재 사용자 홈 아래 폴더에만
//   링크를 만들고, 여러 사용자가 함께 쓰는 폴더(`/usr/local/bin` 등)에는 쓰지 않습니다.
// - Stellar 소유 링크가 아닌 파일은 덮어쓰거나 지우지 않습니다. 확인과 실행 사이에 바뀐 경우도 막습니다.
// - 대체 이름 규칙(ADR-0005 §2): PATH에서 `stellar`가 다른 프로그램으로 먼저 해석되면(설치 위치에 있거나 PATH
//   앞쪽에 있음) `stellar-cli`로 설치하고, `stellar-cli`도 다른 프로그램이 쓰면 아무것도 설치하지 않고 알립니다.
// - 앱 번들 안 helper(`Contents/Helpers/stellar`, ADR-0005 §2·§7, 배치 확정은 XXIV.5.1)가 없으면 설치하지 않습니다.
// 파일 I/O는 메인 스레드 밖에서 호출합니다(아키텍처 §6.4). 위치·안전 조건의 근거는 docs/design/ui/welcome.md 6장입니다.

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
    /// 사용자 홈 디렉터리 절대 경로(파일 I/O 없음).
    var homeDirectory: String { get }
    func entry(atPath path: String) -> StellarCommandFileEntry
    /// 링크를 따라가 실행 가능한 일반 파일인지.
    func isExecutableFile(atPath path: String) -> Bool
    /// 링크를 따라가 현재 사용자가 쓸 수 있는 디렉터리인지.
    func isWritableDirectory(atPath path: String) -> Bool
    /// 경로가 읽기 전용 볼륨(디스크 이미지, App Translocation 위치 등)에 있는지.
    func isOnReadOnlyVolume(path: String) -> Bool
    /// `.app` 번들의 `CFBundleIdentifier`. 번들이 아니거나 읽을 수 없으면 nil.
    func bundleIdentifier(ofBundleAt path: String) -> String?
    /// 중간 디렉터리까지 현재 사용자 권한으로 만듭니다.
    func createDirectory(atPath path: String) throws
    /// 새 심볼릭 링크를 만듭니다. 이미 무엇이 있으면 실패해야 합니다(덮어쓰기 금지).
    func createSymbolicLink(atPath path: String, destination: String) throws
    /// `path`의 심볼릭 링크가 지금도 `expectedDestination`을 가리킬 때만 `newDestination`으로 원자적으로 바꿉니다.
    /// 그 사이 다른 항목으로 바뀌었으면 아무것도 바꾸지 않고 `isConflict`가 true인 오류를 던집니다.
    func replaceSymbolicLink(atPath path: String, expectedDestination: String, newDestination: String) throws
    /// `path`의 심볼릭 링크가 지금도 `expectedDestination`을 가리킬 때만 지웁니다. 조건은 replace와 같습니다.
    func removeSymbolicLink(atPath path: String, expectedDestination: String) throws
    func contentsOfTextFile(atPath path: String) -> String?
    func contentsOfDirectory(atPath path: String) -> [String]
}

/// 파일 시스템 연산 실패.
struct StellarCommandFileSystemError: Error, Equatable {
    var path: String
    var message: String
    /// 확인한 뒤 Stellar가 만들지 않은 항목으로 바뀌어 아무것도 바꾸지 않았으면 true.
    var isConflict = false
}

/// 실제 파일 시스템 구현.
final class LocalStellarCommandFileSystem: StellarCommandFileSystem {
    private let fileManager = FileManager()
    let homeDirectory: String

    init(homeDirectory: String = NSHomeDirectory()) {
        self.homeDirectory = homeDirectory
    }

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

    func bundleIdentifier(ofBundleAt path: String) -> String? {
        // `Bundle(path:)`는 프로세스 안에 캐시되므로 Info.plist를 직접 읽습니다.
        let plist = ((path as NSString).appendingPathComponent("Contents") as NSString)
            .appendingPathComponent("Info.plist")
        return NSDictionary(contentsOfFile: plist)?["CFBundleIdentifier"] as? String
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
            throw Self.failure(path)
        }
    }

    func replaceSymbolicLink(atPath path: String, expectedDestination: String, newDestination: String) throws {
        let directory = (path as NSString).deletingLastPathComponent
        let temporary = Self.temporarySibling(in: directory, tag: "new")
        guard symlink(newDestination, temporary) == 0 else {
            throw Self.failure(path)
        }
        // RENAME_SWAP(rename(2)): 새 링크와 기존 항목을 원자적으로 맞바꿉니다. 명령이 사라지는 순간이 없습니다.
        if renamex_np(temporary, path, UInt32(RENAME_SWAP)) == 0 {
            // 이제 `temporary` 자리에 바뀌기 전 항목이 있습니다. 확인한 소유 링크였는지 다시 봅니다.
            if entry(atPath: temporary) == .symbolicLink(destination: expectedDestination) {
                unlink(temporary)
                return
            }
            // 확인 뒤 다른 항목이 생겼습니다. 다시 맞바꿔 원래대로 돌리고, 새 링크만 지웁니다.
            if renamex_np(temporary, path, UInt32(RENAME_SWAP)) == 0 {
                unlink(temporary)
                throw Self.conflict(path)
            }
            throw StellarCommandFileSystemError(
                path: path,
                message: "Couldn’t restore the original item. It was moved to \(temporary).",
                isConflict: true
            )
        }
        let swapError = errno
        guard swapError == ENOTSUP else {
            unlink(temporary)
            throw Self.failure(path, code: swapError)
        }
        // 맞바꾸기를 지원하지 않는 볼륨: 옆으로 옮겨 확인한 뒤 새 링크를 만듭니다(남의 항목은 되돌림).
        unlink(temporary)
        let aside = try moveAsideVerifying(path, expectedDestination: expectedDestination)
        guard symlink(newDestination, path) == 0 else {
            let code = errno
            // 그 사이 다른 것이 생겼으면 그대로 두고, 옮겨 둔 소유 링크만 정리합니다.
            unlink(aside)
            throw code == EEXIST ? Self.conflict(path) : Self.failure(path, code: code)
        }
        unlink(aside)
    }

    func removeSymbolicLink(atPath path: String, expectedDestination: String) throws {
        let aside = try moveAsideVerifying(path, expectedDestination: expectedDestination)
        guard unlink(aside) == 0 else {
            throw Self.failure(aside)
        }
    }

    func contentsOfTextFile(atPath path: String) -> String? {
        try? String(contentsOfFile: path, encoding: .utf8)
    }

    func contentsOfDirectory(atPath path: String) -> [String] {
        (try? fileManager.contentsOfDirectory(atPath: path)) ?? []
    }

    // MARK: 내부

    /// 항목을 같은 폴더의 임시 이름으로 옮긴 뒤(rename(2)은 내용을 지우지 않음) 확인한 소유 링크인지 봅니다.
    /// 아니면 원래 자리로 되돌리고(그 자리에 새 항목이 생겼으면 덮지 않음) 충돌로 보고합니다.
    private func moveAsideVerifying(_ path: String, expectedDestination: String) throws -> String {
        let aside = Self.temporarySibling(in: (path as NSString).deletingLastPathComponent, tag: "old")
        guard rename(path, aside) == 0 else {
            throw Self.failure(path)
        }
        if entry(atPath: aside) == .symbolicLink(destination: expectedDestination) {
            return aside
        }
        if renamex_np(aside, path, UInt32(RENAME_EXCL)) == 0 {
            throw Self.conflict(path)
        }
        if errno == ENOTSUP, entry(atPath: path) == .missing, rename(aside, path) == 0 {
            throw Self.conflict(path)
        }
        throw StellarCommandFileSystemError(
            path: path,
            message: "Couldn’t restore the original item. It was moved to \(aside).",
            isConflict: true
        )
    }

    private static func temporarySibling(in directory: String, tag: String) -> String {
        (directory as NSString).appendingPathComponent(".stellar-link-\(tag)-\(UUID().uuidString)")
    }

    private static func failure(_ path: String, code: Int32 = errno) -> StellarCommandFileSystemError {
        StellarCommandFileSystemError(path: path, message: String(cString: strerror(code)))
    }

    private static func conflict(_ path: String) -> StellarCommandFileSystemError {
        StellarCommandFileSystemError(path: path, message: "Changed after Stellar checked it", isConflict: true)
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
    /// `.app` 경로 → 번들 식별자.
    var bundleIdentifiers: [String: String] = [:]
    /// 다음 링크 생성·교체를 이 메시지로 실패시킵니다(오류 상태 검사용).
    var failNextLinkOperation: String?
    /// 다음 변경 연산 직전에 한 번 실행합니다(확인과 실행 사이 다른 프로세스의 변경 흉내).
    var beforeNextMutation: (() -> Void)?
    /// 성공한 변경 연산 수(폴더 생성, 링크 생성·교체·삭제). "동의 없는 파일 변경 0" 검사에 씁니다.
    private(set) var mutationCount = 0

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

    func removeNode(_ path: String) {
        nodes[path] = nil
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

    private func runMutationHook() {
        if let hook = beforeNextMutation {
            beforeNextMutation = nil
            hook()
        }
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

    func bundleIdentifier(ofBundleAt path: String) -> String? {
        bundleIdentifiers[path]
    }

    func createDirectory(atPath path: String) throws {
        runMutationHook()
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
        mutationCount += 1
    }

    func createSymbolicLink(atPath path: String, destination: String) throws {
        runMutationHook()
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
        mutationCount += 1
    }

    func replaceSymbolicLink(atPath path: String, expectedDestination: String, newDestination: String) throws {
        runMutationHook()
        if let message = failNextLinkOperation {
            failNextLinkOperation = nil
            throw StellarCommandFileSystemError(path: path, message: message)
        }
        guard isWritable(parentOf: path) else {
            throw StellarCommandFileSystemError(path: path, message: "Permission denied")
        }
        guard nodes[path] == .symbolicLink(destination: expectedDestination) else {
            throw StellarCommandFileSystemError(path: path, message: "Changed after Stellar checked it", isConflict: true)
        }
        nodes[path] = .symbolicLink(destination: newDestination)
        mutationCount += 1
    }

    func removeSymbolicLink(atPath path: String, expectedDestination: String) throws {
        runMutationHook()
        guard isWritable(parentOf: path) else {
            throw StellarCommandFileSystemError(path: path, message: "Permission denied")
        }
        guard nodes[path] == .symbolicLink(destination: expectedDestination) else {
            throw StellarCommandFileSystemError(path: path, message: "Changed after Stellar checked it", isConflict: true)
        }
        nodes[path] = nil
        mutationCount += 1
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

/// 다른 파일 시스템 구현을 감싸 메인 스레드에서 들어온 호출 수를 셉니다(XXIV.5.3·XXIV.5.5 "UI 스레드 호출 0" 검사).
final class StellarCommandFileSystemThreadAudit: StellarCommandFileSystem {
    let base: StellarCommandFileSystem
    private let lock = NSLock()
    private var mainThreadCalls: [String] = []
    private var backgroundCallCount = 0

    init(base: StellarCommandFileSystem) {
        self.base = base
    }

    /// 메인 스레드에서 호출된 연산 이름.
    var mainThreadOperations: [String] {
        lock.lock()
        defer { lock.unlock() }
        return mainThreadCalls
    }

    /// 메인 스레드 밖에서 호출된 연산 수.
    var backgroundOperationCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return backgroundCallCount
    }

    private func record(_ operation: String) {
        lock.lock()
        if Thread.isMainThread { mainThreadCalls.append(operation) } else { backgroundCallCount += 1 }
        lock.unlock()
    }

    var homeDirectory: String { base.homeDirectory }

    func entry(atPath path: String) -> StellarCommandFileEntry {
        record("entry")
        return base.entry(atPath: path)
    }

    func isExecutableFile(atPath path: String) -> Bool {
        record("isExecutableFile")
        return base.isExecutableFile(atPath: path)
    }

    func isWritableDirectory(atPath path: String) -> Bool {
        record("isWritableDirectory")
        return base.isWritableDirectory(atPath: path)
    }

    func isOnReadOnlyVolume(path: String) -> Bool {
        record("isOnReadOnlyVolume")
        return base.isOnReadOnlyVolume(path: path)
    }

    func bundleIdentifier(ofBundleAt path: String) -> String? {
        record("bundleIdentifier")
        return base.bundleIdentifier(ofBundleAt: path)
    }

    func createDirectory(atPath path: String) throws {
        record("createDirectory")
        try base.createDirectory(atPath: path)
    }

    func createSymbolicLink(atPath path: String, destination: String) throws {
        record("createSymbolicLink")
        try base.createSymbolicLink(atPath: path, destination: destination)
    }

    func replaceSymbolicLink(atPath path: String, expectedDestination: String, newDestination: String) throws {
        record("replaceSymbolicLink")
        try base.replaceSymbolicLink(atPath: path, expectedDestination: expectedDestination, newDestination: newDestination)
    }

    func removeSymbolicLink(atPath path: String, expectedDestination: String) throws {
        record("removeSymbolicLink")
        try base.removeSymbolicLink(atPath: path, expectedDestination: expectedDestination)
    }

    func contentsOfTextFile(atPath path: String) -> String? {
        record("contentsOfTextFile")
        return base.contentsOfTextFile(atPath: path)
    }

    func contentsOfDirectory(atPath path: String) -> [String] {
        record("contentsOfDirectory")
        return base.contentsOfDirectory(atPath: path)
    }
}

/// 다른 프로그램이 명령 이름을 쓰고 있다는 판정(ADR-0005 §2 대체 이름 규칙).
struct StellarCommandNameConflict: Equatable {
    enum Position: Equatable {
        /// 설치 위치에 Stellar가 만들지 않은 항목이 있습니다.
        case atInstallLocation
        /// 순서를 확인한 PATH에서 설치 폴더보다 앞에 있거나, 설치 폴더가 그 PATH에 없어 이 프로그램이 먼저 실행됩니다.
        case earlierOnPath
        /// PATH에 있을 수 있으나 순서를 확인하지 못한 폴더에 있어, 먼저 실행될 수 있습니다(보수적으로 충돌로 봄).
        case possiblyEarlierOnPath
    }

    var commandName: String
    var path: String
    var position: Position
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
    /// 링크 대상(앱 번들 안 helper).
    var destination: String
    var action: Action
    /// 링크 폴더를 새로 만들어야 하면 true.
    var createsDirectory: Bool
    /// 링크 폴더가 확인한 PATH에 있으면 true. 확인하지 못했으면 PATH 추가 안내를 보여 줍니다.
    var directoryOnSearchPath: Bool
    /// 기본 이름(`stellar`)을 쓸 수 없어 대체 이름을 고른 이유. 기본 이름이면 nil.
    var primaryNameConflict: StellarCommandNameConflict?
    /// 설치하면 이 링크가 먼저 실행되어 가려지게 되는 같은 이름의 다른 프로그램(확인한 PATH의 뒤쪽).
    var hiddenProgramPaths: [String]

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

/// 현재 설치 상태(XXIV.5.5). 판정 규칙은 docs/design/ui/welcome.md 6.5절입니다.
enum StellarCommandInstallStatus: Equatable {
    enum UnsupportedLocation: Equatable {
        /// Gatekeeper App Translocation 임시 경로에서 실행 중입니다.
        case translocated
        /// 디스크 이미지 같은 읽기 전용 볼륨에서 실행 중입니다.
        case readOnlyVolume
    }

    /// 앱 번들에 helper가 없습니다(XXIV.5.1 전 빌드).
    case helperUnavailable(helperPath: String)
    /// 링크 대상이 곧 사라질 위치에서 실행 중입니다.
    case unsupportedAppLocation(UnsupportedLocation)
    /// 설치할 수 있습니다. 계획의 이름이 `stellar-cli`이면 `primaryNameConflict`가 이유입니다.
    case ready(StellarCommandInstallPlan)
    /// 이 앱을 가리키는 링크가 있고, 그 이름이 다른 프로그램에 가려지지 않습니다.
    case installed(StellarCommandInstallation)
    /// Stellar 소유 링크가 다른 위치(옮겨진 앱, 다른 사본)를 가리킵니다. 다시 연결할 수 있습니다.
    case needsRepair(StellarCommandInstallPlan)
    /// 이 앱을 가리키는 링크가 있지만, 같은 이름의 다른 프로그램이 PATH에서 먼저 실행됩니다.
    /// 대체 이름으로 설치할 수 있으면 그 계획을 함께 줍니다.
    case shadowed(StellarCommandInstallation, conflict: StellarCommandNameConflict, fallback: StellarCommandInstallPlan?)
    /// `stellar`와 `stellar-cli`를 모두 다른 프로그램이 씁니다. 아무것도 설치하지 않고 알립니다.
    case namesUnavailable([StellarCommandNameConflict])
    /// 권한 상승 없이 쓸 수 있는 홈 아래 폴더가 없습니다.
    case noWritableLocation(suggestedDirectory: String)
}

/// 설치 실패.
enum StellarCommandInstallError: Error, Equatable {
    case helperUnavailable(helperPath: String)
    /// 실행 직전 다시 확인했을 때 상황이 바뀌었습니다(남의 파일이 생김, 같은 이름이 PATH 앞쪽에 생김).
    /// 아무것도 바꾸지 않았습니다.
    case conflict(path: String)
    case fileSystem(StellarCommandFileSystemError)
}

/// 제거 결과. 일부만 지웠을 수 있으므로 둘 다 보고합니다.
struct StellarCommandUninstallResult: Equatable {
    var removedPaths: [String]
    var failures: [StellarCommandFileSystemError]
}

/// 셸을 실행하지 않고 확인할 수 있는 명령 탐색 경로(XXIV.5.3).
struct StellarCommandSearchPath: Equatable {
    /// 순서를 아는 PATH 폴더. 셸은 이 순서대로 실행 파일을 찾습니다(zsh·bash `COMMAND EXECUTION`).
    var ordered: [String]
    /// PATH에 있을 수 있으나 순서를 모르는 폴더(셸 설정 파일이 더하는 흔한 위치). 이름 충돌 판정에만 보수적으로 씁니다.
    var unordered: [String]

    init(ordered: [String], unordered: [String] = []) {
        self.ordered = ordered.map(StellarCommandInstaller.lexicallyNormalized)
        let orderedSet = Set(self.ordered)
        var seen = Set<String>()
        self.unordered = unordered.map(StellarCommandInstaller.lexicallyNormalized).filter {
            !orderedSet.contains($0) && seen.insert($0).inserted
        }
    }

    /// 셸 설정 파일이 흔히 PATH에 더하는 패키지 관리자 폴더(Homebrew Apple silicon 기본 접두사, MacPorts 기본 접두사).
    static let packageManagerDirectories = ["/opt/homebrew/bin", "/opt/local/bin"]

    /// login 셸 PATH 구성의 시스템 부분을 읽습니다.
    ///
    /// - 순서를 아는 부분: `/etc/paths`, `/etc/paths.d/*`(path_helper(8)가 이 순서로 PATH를 만들고, macOS
    ///   `/etc/zprofile`이 login zsh에서 path_helper를 실행), 앱 프로세스의 `PATH`.
    /// - 사용자 셸 설정 파일(~/.zprofile, ~/.zshrc 등)은 셸이 실행하는 명령 파일이라 실행하지 않고는 결과를 알 수
    ///   없으므로 읽지 않습니다. 대신 그 파일이 흔히 더하는 폴더를 `unordered`로 둡니다.
    static func system(
        fileSystem: StellarCommandFileSystem,
        environmentPath: String? = ProcessInfo.processInfo.environment["PATH"]
    ) -> StellarCommandSearchPath {
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
        var ordered: [String] = []
        for line in lines {
            var entry = line.trimmingCharacters(in: .whitespaces)
            guard !entry.isEmpty, !entry.hasPrefix("#") else { continue }
            if entry == "~" || entry.hasPrefix("~/") {
                entry = fileSystem.homeDirectory + entry.dropFirst()
            }
            guard entry.hasPrefix("/") else { continue }
            let normalized = StellarCommandInstaller.lexicallyNormalized(entry)
            if seen.insert(normalized).inserted { ordered.append(normalized) }
        }
        let unordered = StellarCommandInstaller.candidateDirectories(homeDirectory: fileSystem.homeDirectory)
            + packageManagerDirectories
        return StellarCommandSearchPath(ordered: ordered, unordered: unordered)
    }
}

/// `stellar` 명령 링크 설치기.
struct StellarCommandInstaller {
    /// 기본 명령 이름(ADR-0005 §2).
    static let commandName = "stellar"
    /// 기본 이름을 쓸 수 없을 때 쓰는 대체 이름(ADR-0005 §2).
    static let fallbackCommandName = "stellar-cli"
    /// 앱 번들 안 helper 상대 경로(ADR-0005 §2·§7: helper 실행 이름은 `stellar`).
    static let helperRelativePath = "Contents/Helpers/stellar"
    /// 이 이름 순서로 시도합니다.
    static let commandNames = [commandName, fallbackCommandName]

    let appBundlePath: String
    let helperPath: String
    let fileSystem: StellarCommandFileSystem
    /// 홈 디렉터리(생성 시 한 번 읽음, UI가 경로를 `~`로 줄일 때 씁니다).
    let homeDirectory: String
    let searchPath: StellarCommandSearchPath
    /// Stellar 소유 링크로 인정할 앱 번들 식별자.
    let ownerBundleIdentifier: String

    init(
        appBundlePath: String,
        fileSystem: StellarCommandFileSystem,
        searchPath: StellarCommandSearchPath,
        ownerBundleIdentifier: String
    ) {
        let bundle = StellarCommandInstaller.lexicallyNormalized(appBundlePath)
        self.appBundlePath = bundle
        self.helperPath = (bundle as NSString).appendingPathComponent(StellarCommandInstaller.helperRelativePath)
        self.fileSystem = fileSystem
        self.homeDirectory = fileSystem.homeDirectory
        self.searchPath = searchPath
        self.ownerBundleIdentifier = ownerBundleIdentifier
    }

    /// 실행 중인 앱 번들과 실제 파일 시스템으로 만듭니다. PATH 파일을 읽으므로 메인 스레드 밖에서 부릅니다.
    static func forRunningApp() -> StellarCommandInstaller {
        let fileSystem = LocalStellarCommandFileSystem()
        return StellarCommandInstaller(
            appBundlePath: Bundle.main.bundlePath,
            fileSystem: fileSystem,
            searchPath: StellarCommandSearchPath.system(fileSystem: fileSystem),
            ownerBundleIdentifier: Bundle.main.bundleIdentifier ?? "xyz.taeyoon.stellar"
        )
    }

    /// 링크 후보 폴더(선호 순). 모두 현재 사용자 홈 아래입니다(최소 권한, 다른 사용자 계정에 영향 없음).
    static func candidateDirectories(homeDirectory home: String) -> [String] {
        [
            (home as NSString).appendingPathComponent(".local/bin"),
            (home as NSString).appendingPathComponent("bin"),
        ]
    }

    var candidateDirectories: [String] {
        StellarCommandInstaller.candidateDirectories(homeDirectory: homeDirectory)
    }

    /// PATH에 추가하라고 안내할 셸 설정 한 줄(zsh·bash 공통 문법).
    static func pathExportLine(for directory: String, homeDirectory: String) -> String {
        var shown = directory
        if directory == homeDirectory {
            shown = "$HOME"
        } else if directory.hasPrefix(homeDirectory + "/") {
            shown = "$HOME" + directory.dropFirst(homeDirectory.count)
        }
        return "export PATH=\"\(shown):$PATH\""
    }

    // MARK: 상태 판정(XXIV.5.5)

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

        // 1. 이 앱을 가리키는 Stellar 소유 링크: 가려지지 않은 것이 있으면 설치됨입니다.
        let ownLinks = installedLinkPaths()
        let current = ownLinks.filter(pointsToHelper)
        if let visible = current.first(where: { shadowingProgram(named: name(of: $0), installingIn: directory(of: $0)) == nil }) {
            return .installed(installation(at: visible))
        }
        if let hidden = current.first,
            let conflict = shadowingProgram(named: name(of: hidden), installingIn: directory(of: hidden))
        {
            var fallback: StellarCommandInstallPlan?
            if case .plan(let plan) = naming(in: directory(of: hidden), createsDirectory: false) {
                fallback = plan
            }
            return .shadowed(installation(at: hidden), conflict: conflict, fallback: fallback)
        }

        // 2. 다른 위치를 가리키는 Stellar 소유 링크: 같은 이름을 쓸 수 있으면 그 자리에서 다시 연결합니다.
        if let stale = ownLinks.first, case .symbolicLink(let previous) = fileSystem.entry(atPath: stale) {
            if shadowingProgram(named: name(of: stale), installingIn: directory(of: stale)) == nil {
                return .needsRepair(
                    StellarCommandInstallPlan(
                        linkPath: stale,
                        destination: helperPath,
                        action: .replaceOwnLink(previousDestination: previous),
                        createsDirectory: false,
                        directoryOnSearchPath: isOnSearchPath(directory(of: stale)),
                        primaryNameConflict: nil,
                        hiddenProgramPaths: hiddenPrograms(named: name(of: stale), installingIn: directory(of: stale))
                    )
                )
            }
            return status(fromNaming: naming(in: directory(of: stale), createsDirectory: false))
        }

        // 3. 새 설치.
        guard let target = preferredDirectory() else {
            return .noWritableLocation(suggestedDirectory: candidateDirectories[0])
        }
        return status(fromNaming: naming(in: target.path, createsDirectory: target.needsCreation))
    }

    /// 후보 폴더에 있는 Stellar 소유 링크 경로.
    func installedLinkPaths() -> [String] {
        var result: [String] = []
        for directory in candidateDirectories {
            for name in StellarCommandInstaller.commandNames {
                let path = (directory as NSString).appendingPathComponent(name)
                if isStellarOwnedLink(atPath: path) { result.append(path) }
            }
        }
        return result
    }

    /// Stellar 소유 링크 판정.
    ///
    /// 심볼릭 링크이고 대상(상대 경로는 링크 폴더 기준)이 `<이름>.app/Contents/Helpers/stellar` 형식이며,
    /// 그 앱 번들이 있으면 번들 식별자가 Stellar의 것이어야 합니다. 앱이 옮겨지거나 지워져 번들이 없는(dangling)
    /// 링크는 Stellar 소유로 봅니다. 그 밖의 파일은 절대 바꾸지 않습니다.
    func isStellarOwnedLink(atPath path: String) -> Bool {
        guard case .symbolicLink(let destination) = fileSystem.entry(atPath: path),
            let bundle = StellarCommandInstaller.appBundlePath(
                ofHelper: resolvedDestination(destination, linkPath: path))
        else { return false }
        if fileSystem.entry(atPath: bundle) == .missing { return true }
        return fileSystem.bundleIdentifier(ofBundleAt: bundle) == ownerBundleIdentifier
    }

    /// helper 경로 모양이면 그 앱 번들 경로를 돌려줍니다.
    static func appBundlePath(ofHelper path: String) -> String? {
        let components = (path as NSString).pathComponents
        guard components.count >= 5 else { return nil }
        let tail = Array(components.suffix(4))
        guard tail[0].hasSuffix(".app"), tail[0].count > 4, tail[1] == "Contents", tail[2] == "Helpers",
            tail[3] == (helperRelativePath as NSString).lastPathComponent
        else { return nil }
        return NSString.path(withComponents: Array(components.dropLast(3)))
    }

    /// `name`을 `directory`에 설치했을 때, 셸 명령 탐색에서 Stellar 링크보다 먼저 찾힐 수 있는 다른 프로그램.
    ///
    /// zsh·bash·POSIX sh는 `/`가 없는 명령 이름을 PATH 폴더 순서대로 찾아 처음 만난 실행 파일을 씁니다.
    /// - 설치 폴더가 확인한 PATH에 있으면 그보다 앞 폴더만, 없으면 확인한 PATH 전체가 먼저입니다.
    /// - 순서를 모르는 폴더(`unordered`)는 먼저일 수 있으므로 보수적으로 충돌로 봅니다.
    /// - Stellar 소유 링크는 다른 프로그램이 아닙니다.
    func shadowingProgram(named name: String, installingIn directory: String) -> StellarCommandNameConflict? {
        let target = StellarCommandInstaller.lexicallyNormalized(directory)
        let ordered = searchPath.ordered
        let earlier = ordered.firstIndex(of: target).map { ordered[..<$0] } ?? ordered[...]
        for folder in earlier {
            let path = (folder as NSString).appendingPathComponent(name)
            if isOtherProgram(atPath: path) {
                return StellarCommandNameConflict(commandName: name, path: path, position: .earlierOnPath)
            }
        }
        for folder in searchPath.unordered where folder != target {
            let path = (folder as NSString).appendingPathComponent(name)
            if isOtherProgram(atPath: path) {
                return StellarCommandNameConflict(commandName: name, path: path, position: .possiblyEarlierOnPath)
            }
        }
        return nil
    }

    // MARK: 실행(XXIV.5.4)

    /// 계획대로 링크를 만듭니다. 실행 직전에 다시 확인해 남의 파일은 건드리지 않고, 같은 이름이 PATH 앞쪽에
    /// 생겼으면 설치하지 않습니다.
    func install(_ plan: StellarCommandInstallPlan) -> Result<StellarCommandInstallation, StellarCommandInstallError> {
        guard fileSystem.isExecutableFile(atPath: helperPath) else {
            return .failure(.helperUnavailable(helperPath: helperPath))
        }
        if let shadow = shadowingProgram(named: plan.commandName, installingIn: plan.directory) {
            return .failure(.conflict(path: shadow.path))
        }
        do {
            if fileSystem.entry(atPath: plan.directory) == .missing {
                guard canCreate(plan.directory) else {
                    return .failure(
                        .fileSystem(StellarCommandFileSystemError(path: plan.directory, message: "Permission denied")))
                }
                try fileSystem.createDirectory(atPath: plan.directory)
            }
            switch fileSystem.entry(atPath: plan.linkPath) {
            case .missing:
                do {
                    try fileSystem.createSymbolicLink(atPath: plan.linkPath, destination: helperPath)
                } catch {
                    // 확인과 생성 사이에 다른 파일이 생겼으면 충돌로 보고합니다.
                    if fileSystem.entry(atPath: plan.linkPath) != .missing, !pointsToHelper(plan.linkPath) {
                        return .failure(.conflict(path: plan.linkPath))
                    }
                    if fileSystem.entry(atPath: plan.linkPath) == .missing { throw error }
                }
            case .symbolicLink(let current) where isStellarOwnedLink(atPath: plan.linkPath):
                if !pointsToHelper(plan.linkPath) {
                    try fileSystem.replaceSymbolicLink(
                        atPath: plan.linkPath, expectedDestination: current, newDestination: helperPath)
                }
            default:
                return .failure(.conflict(path: plan.linkPath))
            }
        } catch let error as StellarCommandFileSystemError {
            return .failure(error.isConflict ? .conflict(path: error.path) : .fileSystem(error))
        } catch {
            return .failure(.fileSystem(StellarCommandFileSystemError(path: plan.linkPath, message: "\(error)")))
        }
        return .success(installation(at: plan.linkPath))
    }

    /// 후보 폴더의 Stellar 소유 링크를 모두 지웁니다. 만든 폴더(~/.local/bin 등)는 남깁니다.
    func uninstall() -> StellarCommandUninstallResult {
        var result = StellarCommandUninstallResult(removedPaths: [], failures: [])
        for path in installedLinkPaths() {
            guard case .symbolicLink(let destination) = fileSystem.entry(atPath: path) else { continue }
            do {
                try fileSystem.removeSymbolicLink(atPath: path, expectedDestination: destination)
                result.removedPaths.append(path)
            } catch let error as StellarCommandFileSystemError where error.isConflict {
                // 확인 뒤 다른 항목으로 바뀌었습니다. 그 항목은 Stellar 것이 아니므로 그대로 둡니다.
                continue
            } catch let error as StellarCommandFileSystemError {
                result.failures.append(error)
            } catch {
                result.failures.append(StellarCommandFileSystemError(path: path, message: "\(error)"))
            }
        }
        return result
    }

    // MARK: 내부

    private enum Naming {
        case plan(StellarCommandInstallPlan)
        case installed(String)
        case unavailable([StellarCommandNameConflict])
    }

    /// 대체 이름 규칙(ADR-0005 §2): `stellar`, `stellar-cli` 순서로 설치 위치 충돌과 PATH 앞쪽 충돌을 봅니다.
    private func naming(in directory: String, createsDirectory: Bool) -> Naming {
        var conflicts: [StellarCommandNameConflict] = []
        for name in StellarCommandInstaller.commandNames {
            let linkPath = (directory as NSString).appendingPathComponent(name)
            var action = StellarCommandInstallPlan.Action.create
            var alreadyInstalled = false
            switch fileSystem.entry(atPath: linkPath) {
            case .missing:
                break
            case .symbolicLink(let destination) where isStellarOwnedLink(atPath: linkPath):
                if pointsToHelper(linkPath) {
                    alreadyInstalled = true
                } else {
                    action = .replaceOwnLink(previousDestination: destination)
                }
            default:
                conflicts.append(
                    StellarCommandNameConflict(commandName: name, path: linkPath, position: .atInstallLocation))
                continue
            }
            if let shadow = shadowingProgram(named: name, installingIn: directory) {
                conflicts.append(shadow)
                continue
            }
            if alreadyInstalled { return .installed(linkPath) }
            return .plan(
                StellarCommandInstallPlan(
                    linkPath: linkPath,
                    destination: helperPath,
                    action: action,
                    createsDirectory: createsDirectory,
                    directoryOnSearchPath: isOnSearchPath(directory),
                    primaryNameConflict: conflicts.first,
                    hiddenProgramPaths: hiddenPrograms(named: name, installingIn: directory)
                )
            )
        }
        return .unavailable(conflicts)
    }

    private func status(fromNaming naming: Naming) -> StellarCommandInstallStatus {
        switch naming {
        case .plan(let plan):
            if case .replaceOwnLink = plan.action { return .needsRepair(plan) }
            return .ready(plan)
        case .installed(let linkPath):
            return .installed(installation(at: linkPath))
        case .unavailable(let conflicts):
            return .namesUnavailable(conflicts)
        }
    }

    /// 확인한 PATH에서 설치 폴더보다 뒤에 있어, 설치하면 가려지는 같은 이름의 다른 프로그램.
    private func hiddenPrograms(named name: String, installingIn directory: String) -> [String] {
        let target = StellarCommandInstaller.lexicallyNormalized(directory)
        guard let index = searchPath.ordered.firstIndex(of: target) else { return [] }
        return searchPath.ordered[(index + 1)...].compactMap { folder in
            let path = (folder as NSString).appendingPathComponent(name)
            return isOtherProgram(atPath: path) ? path : nil
        }
    }

    private func isOtherProgram(atPath path: String) -> Bool {
        fileSystem.isExecutableFile(atPath: path) && !isStellarOwnedLink(atPath: path)
    }

    private func installation(at linkPath: String) -> StellarCommandInstallation {
        StellarCommandInstallation(linkPath: linkPath, directoryOnSearchPath: isOnSearchPath(directory(of: linkPath)))
    }

    private func directory(of path: String) -> String {
        (path as NSString).deletingLastPathComponent
    }

    private func name(of path: String) -> String {
        (path as NSString).lastPathComponent
    }

    private func isOnSearchPath(_ directory: String) -> Bool {
        searchPath.ordered.contains(StellarCommandInstaller.lexicallyNormalized(directory))
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

    /// 위치 결정(XXIV.5.3): 홈 아래 후보 중 확인한 PATH에 있고 쓸 수 있는 첫 폴더, 없으면 쓸 수 있는(또는 만들 수
    /// 있는) 첫 후보. 권한 상승이 필요하거나 여러 사용자가 함께 쓰는 폴더는 고르지 않습니다.
    private func preferredDirectory() -> (path: String, needsCreation: Bool)? {
        let usable = candidateDirectories.compactMap { directory -> (path: String, needsCreation: Bool)? in
            guard !fileSystem.isOnReadOnlyVolume(path: directory) else { return nil }
            switch fileSystem.entry(atPath: directory) {
            case .missing:
                return canCreate(directory) ? (directory, true) : nil
            default:
                return fileSystem.isWritableDirectory(atPath: directory) ? (directory, false) : nil
            }
        }
        return usable.first(where: { isOnSearchPath($0.path) }) ?? usable.first
    }

    /// 홈 아래이고, 가장 가까운 기존 상위 폴더에 쓸 수 있으면 만들 수 있습니다.
    private func canCreate(_ directory: String) -> Bool {
        guard directory.hasPrefix(homeDirectory + "/") else { return false }
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
