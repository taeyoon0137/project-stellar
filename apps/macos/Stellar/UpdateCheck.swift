import AppKit
import CryptoKit
import Network
import Sparkle

/// 업데이트 구성·채널 격리·승인 gate·서명 검증을 headless로 검증합니다(XVI.9.5, ADR-0017).
///
/// 1. 배포 구성: 이 앱의 Info.plist가 Stable/Beta appcast URL(https, 서로 다름), 서명 appcast 요구, 추출 전
///    archive 검증, 자동 다운로드·설치 금지를 실제로 싣는지 확인합니다(build setting → Info.plist 전달).
///    배포 키가 없으면 `AppUpdateController`가 updater를 시작하지 않고(네트워크 요청 없음), 구성이 완전하면
///    시작하는지 fixture bundle로 확인합니다.
/// 2. 구성 검증 규칙: URL 누락·http·같은 URL·서명 요구 꺼짐·자동 설치 허용·키 누락/형식 오류를 거부합니다.
/// 3. channel isolation fixture(실제 Sparkle, 로컬 file appcast): Stable updater는 Stable appcast만 요청하고
///    같은 appcast에 잘못 올라간 beta 채널 항목과 최소 OS 미달 항목을 받지 않습니다. Beta updater는 Beta
///    appcast만 요청하고 beta 항목을 받습니다. Stable URL에 Beta appcast가 와도 beta 항목은 제외됩니다.
/// 4. 무결성: 서명 뒤 다운로드 URL이 바뀐 appcast와 서명 없는 appcast는 거부합니다.
/// 5. 승인 gate: background 확인에서 사용자가 거절하면 다운로드 0회, 승인하면 다운로드를 정확히 1회 시작하고
///    설치 단계로는 가지 않습니다(fixture archive가 없어 다운로드가 실패하거나 즉시 취소).
///
/// 네트워크·실제 배포 키·설치는 쓰지 않습니다. 고정 appcast 서명은 RFC 8032 §7.1 TEST 1 키(공개된 test vector)로
/// 저장소 밖에서 만들었고 개인 키는 앱에 넣지 않습니다. 이 키는 배포에 쓰면 안 됩니다(scripts/macos가 거부).
enum UpdateCheck {
    private struct CheckFailure: Error {
        let message: String
    }

    /// RFC 8032 §7.1 TEST 1 공개 키(base64). 개인 키가 공개된 test vector이므로 fixture 전용입니다.
    static let testVectorPublicKey = "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo="

    /// 아래 appcast 본문에 대한 고정 서명(RFC 8032 TEST 1 키). 본문을 바꾸면 다시 만들어야 합니다.
    private static let stableFeedSignature =
        "5EU/ra7QlgbLwmDyR4bAXuR9a1FAVqDOHnGMdAqMbMbaroJXVLxsgH46IS1fe0k9bDCJeSyjAb5R1xag7G4OBw=="
    private static let betaFeedSignature =
        "mv8uv7mSPzMg5gOOaSffH+l8JqLrjNqnhI/V2hm3GS5x3M23/n8r0R8dmJ8O8QOVpXMnVUehYyt213VmhHLQAw=="

    private struct FixtureItem {
        let build: Int
        let version: String
        let minimumSystem: String
        let channel: String?
    }

    /// 항목 enclosure의 자리표시 archive 서명(64바이트 0). fixture archive는 내려받지 않습니다.
    private static let placeholderArchiveSignature = Data(count: 64).base64EncodedString()

    /// Sparkle appcast 본문(서명 대상). 줄 구성은 서명 생성 시와 같아야 합니다.
    private static func appcast(_ title: String, _ items: [FixtureItem]) -> String {
        var lines = [
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>",
            "<rss version=\"2.0\" xmlns:sparkle=\"http://www.andymatuschak.org/xml-namespaces/sparkle\">",
            "<channel>",
            "<title>\(title)</title>",
        ]
        for item in items {
            lines += [
                "<item>",
                "<title>\(item.version)</title>",
                "<sparkle:version>\(item.build)</sparkle:version>",
                "<sparkle:shortVersionString>\(item.version)</sparkle:shortVersionString>",
                "<sparkle:minimumSystemVersion>\(item.minimumSystem)</sparkle:minimumSystemVersion>",
            ]
            if let channel = item.channel {
                lines.append("<sparkle:channel>\(channel)</sparkle:channel>")
            }
            lines += [
                "<enclosure url=\"Stellar-\(item.build).zip\" length=\"1024\" type=\"application/octet-stream\" "
                    + "sparkle:edSignature=\"\(placeholderArchiveSignature)\"/>",
                "</item>",
            ]
        }
        lines += ["</channel>", "</rss>"]
        return lines.joined(separator: "\n") + "\n"
    }

    /// Stable appcast: build 2·3, 최소 OS 99 요구 build 4, beta 채널로 잘못 올라간 build 6.
    private static let stableFeed = appcast(
        "Stellar Stable",
        [
            FixtureItem(build: 2, version: "0.1.0", minimumSystem: "26.0", channel: nil),
            FixtureItem(build: 3, version: "0.2.0", minimumSystem: "26.0", channel: nil),
            FixtureItem(build: 4, version: "0.3.0", minimumSystem: "99.0", channel: nil),
            FixtureItem(build: 6, version: "0.4.0-beta.1", minimumSystem: "26.0", channel: "beta"),
        ])

    /// Beta appcast: 기본 채널 build 3, beta 채널 build 5.
    private static let betaFeed = appcast(
        "Stellar Beta",
        [
            FixtureItem(build: 3, version: "0.2.0", minimumSystem: "26.0", channel: nil),
            FixtureItem(build: 5, version: "0.3.0-beta.1", minimumSystem: "26.0", channel: "beta"),
        ])

    /// Sparkle 서명 appcast 형식: 본문 뒤에 서명 block(`edSignature`, 본문 `length`)을 붙입니다.
    private static func signed(_ content: String, _ signature: String) -> Data {
        var data = Data(content.utf8)
        data.append(Data("<!-- sparkle-signatures:\nedSignature: \(signature)\nlength: \(data.count)\n-->\n".utf8))
        return data
    }

    // MARK: - 기록용 delegate·user driver

    /// 제품 `UpdateChannelDelegate`에 채널 정책을 맡기고 Sparkle 이벤트를 기록합니다.
    private final class RecordingDelegate: NSObject, SPUUpdaterDelegate {
        let channelDelegate: UpdateChannelDelegate
        var requestedFeeds: [String] = []
        var found: [String] = []
        var willDownload: [String] = []
        var finishedCycles = 0
        var cycleErrors: [NSError] = []

        init(channelDelegate: UpdateChannelDelegate) {
            self.channelDelegate = channelDelegate
        }

        func feedURLString(for updater: SPUUpdater) -> String? {
            let feed = channelDelegate.feedURLString(for: updater)
            requestedFeeds.append(feed ?? "<nil>")
            return feed
        }

        func allowedChannels(for updater: SPUUpdater) -> Set<String> {
            channelDelegate.allowedChannels(for: updater)
        }

        func updater(_ updater: SPUUpdater, didFindValidUpdate item: SUAppcastItem) {
            found.append(item.versionString)
        }

        func updater(_ updater: SPUUpdater, willDownloadUpdate item: SUAppcastItem, with request: NSMutableURLRequest) {
            willDownload.append(item.versionString)
        }

        func updater(_ updater: SPUUpdater, didFinishUpdateCycleFor updateCheck: SPUUpdateCheck, error: Error?) {
            finishedCycles += 1
            if let error { cycleErrors.append(error as NSError) }
        }
    }

    /// 새 버전 제안에 정해진 답을 주는 user driver. 다운로드가 시작되면 즉시 취소하고 설치 단계는 기록만 합니다.
    private final class ScriptedUserDriver: NSObject, SPUUserDriver {
        let choice: SPUUserUpdateChoice
        var offered: [String] = []
        var downloadsInitiated = 0
        var installPrompts = 0

        init(choice: SPUUserUpdateChoice) {
            self.choice = choice
        }

        func show(_ request: SPUUpdatePermissionRequest, reply: @escaping (SUUpdatePermissionResponse) -> Void) {
            reply(SUUpdatePermissionResponse(automaticUpdateChecks: false, sendSystemProfile: false))
        }

        func showUserInitiatedUpdateCheck(cancellation: @escaping () -> Void) {}

        func showUpdateFound(
            with appcastItem: SUAppcastItem, state: SPUUserUpdateState,
            reply: @escaping (SPUUserUpdateChoice) -> Void
        ) {
            offered.append(appcastItem.versionString)
            reply(choice)
        }

        func showUpdateReleaseNotes(with downloadData: SPUDownloadData) {}

        func showUpdateReleaseNotesFailedToDownloadWithError(_ error: Error) {}

        func showUpdateNotFoundWithError(_ error: Error, acknowledgement: @escaping () -> Void) {
            acknowledgement()
        }

        func showUpdaterError(_ error: Error, acknowledgement: @escaping () -> Void) {
            acknowledgement()
        }

        func showDownloadInitiated(cancellation: @escaping () -> Void) {
            downloadsInitiated += 1
            cancellation()
        }

        func showDownloadDidReceiveExpectedContentLength(_ expectedContentLength: UInt64) {}

        func showDownloadDidReceiveData(ofLength length: UInt64) {}

        func showDownloadDidStartExtractingUpdate() {
            installPrompts += 1
        }

        func showExtractionReceivedProgress(_ progress: Double) {}

        func showReady(toInstallAndRelaunch reply: @escaping (SPUUserUpdateChoice) -> Void) {
            installPrompts += 1
            reply(.dismiss)
        }

        func showInstallingUpdate(
            withApplicationTerminated applicationTerminated: Bool, retryTerminatingApplication: @escaping () -> Void
        ) {
            installPrompts += 1
        }

        func showUpdateInstalledAndRelaunched(_ relaunched: Bool, acknowledgement: @escaping () -> Void) {
            installPrompts += 1
            acknowledgement()
        }

        func dismissUpdateInstallation() {}
    }

    // MARK: - fixture

    /// fixture appcast를 내주는 loopback(127.0.0.1) HTTP 서버. Sparkle은 http(s) URL만 내려받습니다.
    ///
    /// 요청 경로를 기록해 각 채널이 실제로 어떤 appcast를 요청했는지 확인합니다. 외부 interface에는 열지 않습니다.
    private final class LoopbackServer {
        private let listener: NWListener
        private let files: [String: Data]
        private let queue = DispatchQueue(label: "xyz.taeyoon.stellar.update-check.server")
        private let lock = NSLock()
        private var requestsStorage: [String] = []
        private(set) var port: UInt16 = 0

        init(files: [String: Data]) throws {
            self.files = files
            let parameters = NWParameters.tcp
            parameters.requiredLocalEndpoint = NWEndpoint.hostPort(host: .ipv4(.loopback), port: .any)
            parameters.allowLocalEndpointReuse = true
            listener = try NWListener(using: parameters)
        }

        var requests: [String] { lock.withLock { requestsStorage } }

        func url(_ path: String) -> URL {
            URL(string: "http://127.0.0.1:\(port)\(path)")!
        }

        func start() throws {
            let ready = DispatchSemaphore(value: 0)
            let failure = NSLock()
            var startError: NWError?
            listener.stateUpdateHandler = { state in
                switch state {
                case .ready:
                    ready.signal()
                case .failed(let error):
                    failure.withLock { startError = error }
                    ready.signal()
                default:
                    break
                }
            }
            listener.newConnectionHandler = { [weak self] connection in
                self?.serve(connection)
            }
            listener.start(queue: queue)
            guard ready.wait(timeout: .now() + 5) == .success else {
                throw CheckFailure(message: "fixture HTTP 서버가 준비되지 않음")
            }
            if let error = failure.withLock({ startError }) {
                throw CheckFailure(message: "fixture HTTP 서버 시작 실패: \(error)")
            }
            guard let port = listener.port?.rawValue, port != 0 else {
                throw CheckFailure(message: "fixture HTTP 서버 port 없음")
            }
            self.port = port
        }

        func stop() {
            listener.cancel()
        }

        private func serve(_ connection: NWConnection) {
            connection.start(queue: queue)
            receiveHeader(connection, buffer: Data())
        }

        /// 요청 header(최대 16 KiB)를 받은 뒤 첫 줄의 경로로 응답합니다.
        private func receiveHeader(_ connection: NWConnection, buffer: Data) {
            connection.receive(minimumIncompleteLength: 1, maximumLength: 4096) { [weak self] data, _, isComplete, error in
                guard let self else { return }
                var buffer = buffer
                if let data { buffer.append(data) }
                if buffer.range(of: Data("\r\n\r\n".utf8)) != nil {
                    self.respond(connection, header: buffer)
                } else if error != nil || isComplete || buffer.count > 16 * 1024 {
                    connection.cancel()
                } else {
                    self.receiveHeader(connection, buffer: buffer)
                }
            }
        }

        private func respond(_ connection: NWConnection, header: Data) {
            let requestLine = String(decoding: header, as: UTF8.self).split(separator: "\r\n").first ?? ""
            let parts = requestLine.split(separator: " ")
            let path = parts.count >= 2 ? String(parts[1]) : ""
            lock.withLock { requestsStorage.append(path) }
            let body = files[path]
            let status = body == nil ? "404 Not Found" : "200 OK"
            let payload = body ?? Data()
            var response = Data(
                ("HTTP/1.1 \(status)\r\nContent-Type: application/xml\r\nContent-Length: \(payload.count)\r\n"
                    + "Connection: close\r\n\r\n").utf8)
            response.append(payload)
            connection.send(content: response, completion: .contentProcessed { _ in connection.cancel() })
        }
    }

    /// 임시 폴더의 host bundle(Info.plist만)과 loopback 서버의 서명 appcast들.
    private struct Fixture {
        let root: URL
        let bundle: Bundle
        let bundleIdentifier: String
        let channelSuite: String
        let server: LoopbackServer

        static let stablePath = "/stable.xml"
        static let betaPath = "/beta.xml"
        /// Stable URL에 Beta appcast가 배포된 경우.
        static let swappedPath = "/stable-swapped.xml"
        /// 서명 뒤 다운로드 URL이 바뀐 경우.
        static let tamperedPath = "/stable-tampered.xml"
        /// 서명 block이 없는 경우.
        static let unsignedPath = "/stable-unsigned.xml"

        /// fixture 정보(Stable feed 경로를 바꿔 끼울 수 있음).
        func info(stable: String = Fixture.stablePath) -> [String: Any] {
            var info = bundle.infoDictionary ?? [:]
            info[UpdateInfoKey.stableFeed] = server.url(stable).absoluteString
            info[UpdateInfoKey.betaFeed] = server.url(Fixture.betaPath).absoluteString
            return info
        }
    }

    private static func makeFixture() throws -> Fixture {
        let signedStable = signed(stableFeed, stableFeedSignature)
        let signedBeta = signed(betaFeed, betaFeedSignature)
        // 서명 뒤 build 3의 다운로드 위치를 다른 host로 바꾼 appcast(feed host 탈취 모사).
        let redirected = Data(
            String(decoding: signedStable, as: UTF8.self).replacingOccurrences(
                of: "url=\"Stellar-3.zip\"", with: "url=\"https://attacker.invalid/Stellar-3.zip\"").utf8)
        guard redirected.count != signedStable.count else {
            throw CheckFailure(message: "변조 fixture를 만들지 못함")
        }
        let server = try LoopbackServer(files: [
            Fixture.stablePath: signedStable,
            Fixture.betaPath: signedBeta,
            Fixture.swappedPath: signedBeta,
            Fixture.tamperedPath: redirected,
            Fixture.unsignedPath: Data(stableFeed.utf8),
        ])
        try server.start()

        // 고정 identifier: 실행마다 defaults 파일이 늘지 않도록 하고, 이전 실행이 남긴 상태는 먼저 지웁니다.
        let identifier = fixtureBundleIdentifier
        removeFixtureState(identifier)
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(
            "stellar-update-check-\(UUID().uuidString)", isDirectory: true)
        let contents = root.appendingPathComponent("UpdateFixture.app/Contents", isDirectory: true)
        try FileManager.default.createDirectory(at: contents, withIntermediateDirectories: true)
        let info: [String: Any] = [
            "CFBundleIdentifier": identifier,
            "CFBundleName": "StellarUpdateFixture",
            "CFBundlePackageType": "APPL",
            "CFBundleVersion": "1",
            "CFBundleShortVersionString": "0.0.1",
            UpdateInfoKey.stableFeed: server.url(Fixture.stablePath).absoluteString,
            UpdateInfoKey.betaFeed: server.url(Fixture.betaPath).absoluteString,
            UpdateInfoKey.publicKey: testVectorPublicKey,
            UpdateInfoKey.requireSignedFeed: true,
            UpdateInfoKey.verifyBeforeExtraction: true,
            UpdateInfoKey.allowsAutomaticUpdates: false,
            UpdateInfoKey.automaticallyUpdate: false,
            // fixture는 예약 확인을 하지 않습니다(확인은 테스트가 명시적으로 시작).
            "SUEnableAutomaticChecks": false,
        ]
        let plist = try PropertyListSerialization.data(fromPropertyList: info, format: .xml, options: 0)
        try plist.write(to: contents.appendingPathComponent("Info.plist"))
        guard let bundle = Bundle(url: root.appendingPathComponent("UpdateFixture.app")),
            bundle.bundleIdentifier == identifier
        else { throw CheckFailure(message: "fixture bundle을 열지 못함") }

        return Fixture(
            root: root, bundle: bundle, bundleIdentifier: identifier, channelSuite: "\(identifier).channel",
            server: server)
    }

    /// fixture host bundle identifier(Sparkle은 이 이름으로 defaults·cache를 씁니다).
    private static let fixtureBundleIdentifier = "xyz.taeyoon.stellar.update-fixture"

    /// fixture identifier의 defaults·Sparkle cache와 앱 defaults의 서명 실패 기록을 지웁니다.
    private static func removeFixtureState(_ identifier: String) {
        UserDefaults.standard.removePersistentDomain(forName: identifier)
        UserDefaults.standard.removePersistentDomain(forName: "\(identifier).channel")
        UserDefaults.standard.removeObject(forKey: "SUInitialFailedFeedSigningValidationDate_\(identifier)")
        if let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first {
            try? FileManager.default.removeItem(at: caches.appendingPathComponent(identifier))
        }
    }

    /// fixture가 남긴 서버·파일·defaults·cache를 지웁니다.
    private static func cleanUp(_ fixture: Fixture) {
        fixture.server.stop()
        try? FileManager.default.removeItem(at: fixture.root)
        removeFixtureState(fixture.bundleIdentifier)
    }

    // MARK: - 실행

    static func run() -> Bool {
        do {
            let summary = try exercise()
            print("update ok: \(summary)")
            return true
        } catch let failure as CheckFailure {
            FileHandle.standardError.write(Data("update 실패: \(failure.message)\n".utf8))
            return false
        } catch {
            FileHandle.standardError.write(Data("update 실패: \(error)\n".utf8))
            return false
        }
    }

    private static func check(_ condition: Bool, _ message: @autoclosure () -> String) throws {
        if !condition { throw CheckFailure(message: message()) }
    }

    /// main run loop를 돌리며 조건을 기다립니다(Sparkle 콜백은 main thread).
    private static func waitUntil(_ timeout: TimeInterval, _ condition: () -> Bool) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while !condition() {
            if Date() >= deadline { return false }
            RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.01))
        }
        return true
    }

    private enum CheckKind {
        /// UI 없는 정보 확인(`checkForUpdateInformation`).
        case probe
        /// 예약 확인과 같은 background 확인(`checkForUpdatesInBackground`), 새 버전이면 user driver에 묻습니다.
        case background
    }

    /// 실제 Sparkle updater로 한 번의 확인 주기를 실행하고 기록과 그동안의 HTTP 요청 경로를 돌려줍니다.
    private static func runCycle(
        _ fixture: Fixture, channel: UpdateChannel, info: [String: Any], kind: CheckKind,
        choice: SPUUserUpdateChoice = .dismiss
    ) throws -> (delegate: RecordingDelegate, driver: ScriptedUserDriver, requests: [String]) {
        guard case .success(let configuration) = UpdateConfiguration.load(info: info, allowedSchemes: ["http"]) else {
            throw CheckFailure(message: "fixture 구성이 유효하지 않음")
        }
        guard let defaults = UserDefaults(suiteName: fixture.channelSuite) else {
            throw CheckFailure(message: "채널 defaults 생성 실패")
        }
        defaults.set(channel.rawValue, forKey: UpdateChannel.defaultsKey)
        let channelDelegate = UpdateChannelDelegate(
            policy: UpdateChannelPolicy(configuration: configuration, defaults: defaults))
        let delegate = RecordingDelegate(channelDelegate: channelDelegate)
        let driver = ScriptedUserDriver(choice: choice)
        let updater = SPUUpdater(
            hostBundle: fixture.bundle, applicationBundle: fixture.bundle, userDriver: driver, delegate: delegate)
        do {
            try updater.start()
        } catch {
            throw CheckFailure(message: "Sparkle 시작 실패(\(channel)): \(error.localizedDescription)")
        }
        try check(!updater.allowsAutomaticUpdates, "자동 업데이트가 허용됨")
        try check(!updater.automaticallyDownloadsUpdates, "자동 다운로드가 켜져 있음")
        let before = fixture.server.requests.count
        switch kind {
        case .probe: updater.checkForUpdateInformation()
        case .background: updater.checkForUpdatesInBackground()
        }
        guard waitUntil(15, { delegate.finishedCycles > 0 }) else {
            throw CheckFailure(message: "\(channel) 확인 주기가 끝나지 않음")
        }
        // 주기 종료 뒤 늦게 오는 콜백(취소된 다운로드 등)을 잠시 더 받습니다.
        _ = waitUntil(0.2, { false })
        return (delegate, driver, Array(fixture.server.requests.dropFirst(before)))
    }

    private static func exercise() throws -> String {
        // 0) 고정 appcast 서명이 본문과 맞는지(fixture 자체 무결성).
        guard let keyData = Data(base64Encoded: testVectorPublicKey),
            let key = try? Curve25519.Signing.PublicKey(rawRepresentation: keyData),
            let stableSignature = Data(base64Encoded: stableFeedSignature),
            let betaSignature = Data(base64Encoded: betaFeedSignature)
        else { throw CheckFailure(message: "fixture 키·서명 해석 실패") }
        try check(key.isValidSignature(stableSignature, for: Data(stableFeed.utf8)), "stable fixture 서명 불일치")
        try check(key.isValidSignature(betaSignature, for: Data(betaFeed.utf8)), "beta fixture 서명 불일치")

        // 1) 배포 구성: 이 빌드의 Info.plist가 업데이트 구성을 실제로 싣습니다.
        let mainInfo = Bundle.main.infoDictionary ?? [:]
        for key in [UpdateInfoKey.stableFeed, UpdateInfoKey.betaFeed] {
            let text = mainInfo[key] as? String ?? ""
            try check(
                URL(string: text)?.scheme == "https" && URL(string: text)?.host?.isEmpty == false,
                "Info.plist \(key)가 https URL이 아님: '\(text)'")
        }
        try check(mainInfo["SUEnableAutomaticChecks"] as? Bool == true, "Info.plist 자동 확인(background) 꺼짐")
        try check(mainInfo["SUScheduledCheckInterval"] as? Int == 86_400, "Info.plist 확인 주기가 하루가 아님")
        let keyState: String
        switch UpdateConfiguration.load(info: mainInfo) {
        case .success:
            keyState = "배포 키 설정됨"
        case .failure(.missingPublicKey):
            // 키 생성 전 빌드: updater를 시작하지 않습니다(네트워크 요청 없음).
            keyState = "배포 키 미설정(업데이트 비활성)"
            let controller = AppUpdateController(bundle: .main)
            try check(controller.state == .disabled(.missingPublicKey), "키 없는 앱 상태: \(controller.state)")
        case .failure(let error):
            throw CheckFailure(message: "Info.plist 업데이트 구성 오류: \(error)")
        }

        let fixture = try makeFixture()
        defer { cleanUp(fixture) }

        // 2) 구성 검증 규칙.
        var https = fixture.info()
        https[UpdateInfoKey.stableFeed] = "https://updates.invalid/stable.xml"
        https[UpdateInfoKey.betaFeed] = "https://updates.invalid/beta.xml"
        func rejects(_ change: (inout [String: Any]) -> Void, _ expected: UpdateConfigurationError) throws {
            var info = https
            change(&info)
            let result = UpdateConfiguration.load(info: info)
            guard case .failure(let error) = result, error == expected else {
                throw CheckFailure(message: "구성 \(expected) 기대, 결과 \(result)")
            }
        }
        guard case .success(let valid) = UpdateConfiguration.load(info: https), valid.publicKey.count == 32 else {
            throw CheckFailure(message: "유효 구성 거부")
        }
        try rejects({ $0[UpdateInfoKey.stableFeed] = "" }, .missingFeedURL(.stable))
        try rejects({ $0.removeValue(forKey: UpdateInfoKey.betaFeed) }, .missingFeedURL(.beta))
        try rejects({ $0[UpdateInfoKey.stableFeed] = "http://updates.invalid/stable.xml" }, .insecureFeedURL(.stable))
        try rejects({ $0[UpdateInfoKey.betaFeed] = "file:///tmp/beta.xml" }, .insecureFeedURL(.beta))
        try rejects({ $0[UpdateInfoKey.betaFeed] = "https:///beta.xml" }, .insecureFeedURL(.beta))
        try rejects({ $0[UpdateInfoKey.betaFeed] = "https://updates.invalid/stable.xml" }, .sharedFeedURL)
        try rejects({ $0[UpdateInfoKey.requireSignedFeed] = false }, .unsignedFeedAllowed)
        try rejects({ $0.removeValue(forKey: UpdateInfoKey.verifyBeforeExtraction) }, .unsignedFeedAllowed)
        try rejects({ $0.removeValue(forKey: UpdateInfoKey.allowsAutomaticUpdates) }, .automaticInstallAllowed)
        try rejects({ $0[UpdateInfoKey.automaticallyUpdate] = true }, .automaticInstallAllowed)
        try rejects({ $0[UpdateInfoKey.publicKey] = "" }, .missingPublicKey)
        try rejects({ $0[UpdateInfoKey.publicKey] = "not base64!" }, .invalidPublicKey)
        try rejects({ $0[UpdateInfoKey.publicKey] = Data(count: 31).base64EncodedString() }, .invalidPublicKey)

        // 채널 선호값: beta만 Beta, 그 밖은 Stable.
        guard let preference = UserDefaults(suiteName: fixture.channelSuite) else {
            throw CheckFailure(message: "defaults 생성 실패")
        }
        preference.removeObject(forKey: UpdateChannel.defaultsKey)
        try check(UpdateChannel.preferred(preference) == .stable, "기본 채널")
        preference.set("beta", forKey: UpdateChannel.defaultsKey)
        try check(UpdateChannel.preferred(preference) == .beta, "beta 채널 설정")
        preference.set("nightly", forKey: UpdateChannel.defaultsKey)
        try check(UpdateChannel.preferred(preference) == .stable, "알 수 없는 채널은 stable")

        // 앱 controller: https만 허용하는 기본 구성은 http feed bundle을 시작하지 않고, 구성이 완전한 bundle은
        // 선호 채널로 updater를 시작합니다(fixture는 예약 확인 꺼짐, 요청 없음).
        preference.removeObject(forKey: UpdateChannel.defaultsKey)
        let refused = AppUpdateController(bundle: fixture.bundle, defaults: preference)
        try check(refused.state == .disabled(.insecureFeedURL(.stable)), "http feed 앱 상태: \(refused.state)")
        let started = AppUpdateController(bundle: fixture.bundle, defaults: preference, allowedSchemes: ["http"])
        try check(started.state == .started(.stable), "구성 완료 앱 상태: \(started.state)")
        try check(fixture.server.requests.isEmpty, "확인 전 요청 발생: \(fixture.server.requests)")

        // 3) channel isolation(실제 Sparkle): 각 채널은 자기 appcast만 요청하고 자기 채널 항목만 받습니다.
        let stable = try runCycle(fixture, channel: .stable, info: fixture.info(), kind: .probe)
        try check(stable.requests == [Fixture.stablePath], "stable 요청: \(stable.requests)")
        try check(
            stable.delegate.found == ["3"],
            "stable 제안(build 3 기대, beta·최소 OS 미달 제외): \(stable.delegate.found), "
                + "오류 \(stable.delegate.cycleErrors)")

        let beta = try runCycle(fixture, channel: .beta, info: fixture.info(), kind: .probe)
        try check(beta.requests == [Fixture.betaPath], "beta 요청: \(beta.requests)")
        try check(beta.delegate.found == ["5"], "beta 제안(build 5 기대): \(beta.delegate.found)")

        let swapped = try runCycle(
            fixture, channel: .stable, info: fixture.info(stable: Fixture.swappedPath), kind: .probe)
        try check(swapped.requests == [Fixture.swappedPath], "swapped 요청: \(swapped.requests)")
        try check(
            swapped.delegate.found == ["3"],
            "Stable URL의 Beta appcast에서 beta 항목이 새어 나옴: \(swapped.delegate.found)")

        // 4) 무결성: 서명 뒤 변조·서명 없음은 거부(제안 없음, 주기 오류, 변조된 다운로드 위치 요청 없음).
        for (name, path) in [("변조", Fixture.tamperedPath), ("서명 없음", Fixture.unsignedPath)] {
            let rejected = try runCycle(fixture, channel: .stable, info: fixture.info(stable: path), kind: .probe)
            try check(rejected.requests == [path], "\(name) 요청: \(rejected.requests)")
            try check(rejected.delegate.found.isEmpty, "\(name) appcast에서 업데이트 제안: \(rejected.delegate.found)")
            try check(!rejected.delegate.cycleErrors.isEmpty, "\(name) appcast가 오류 없이 처리됨")
        }

        // 5) 승인 gate(background 확인): 거절 → 다운로드 0회, 승인 → 정확히 1회 시작, 설치 단계 없음.
        let declined = try runCycle(
            fixture, channel: .stable, info: fixture.info(), kind: .background, choice: .dismiss)
        try check(declined.driver.offered == ["3"], "거절 경로 제안: \(declined.driver.offered)")
        try check(
            declined.delegate.willDownload.isEmpty && declined.driver.downloadsInitiated == 0
                && declined.requests == [Fixture.stablePath],
            "거절했는데 다운로드: \(declined.delegate.willDownload), \(declined.requests)")

        let approved = try runCycle(
            fixture, channel: .stable, info: fixture.info(), kind: .background, choice: .install)
        try check(approved.driver.offered == ["3"], "승인 경로 제안: \(approved.driver.offered)")
        try check(approved.delegate.willDownload == ["3"], "승인 후 다운로드 시작: \(approved.delegate.willDownload)")
        try check(
            approved.requests.first == Fixture.stablePath && approved.requests.count <= 2
                && approved.requests.dropFirst().allSatisfy { $0 == "/Stellar-3.zip" },
            "승인 경로 요청: \(approved.requests)")
        try check(approved.driver.installPrompts == 0, "설치 단계에 도달함: \(approved.driver.installPrompts)")

        return "구성(Info.plist feed·서명 요구·자동 설치 금지, \(keyState)), channel isolation(stable↔beta, "
            + "잘못 올라간 beta 항목 제외), appcast 서명(변조·무서명 거부), approval gate(거절 0회·승인 1회)"
    }
}
