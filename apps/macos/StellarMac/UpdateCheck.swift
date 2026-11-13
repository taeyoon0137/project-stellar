import AppKit

/// Stable/Beta 업데이트 채널 격리와 승인 전 다운로드 금지를 headless로 검증합니다(XVI.9.5).
///
/// 네트워크 없이 기록용 feed loader(채널별 고정 JSON), 정해진 답을 주는 승인자, 기록용 downloader를 주입해
/// - channel isolation: 각 채널 checker는 자기 feed URL만 요청하고, 다른 채널을 선언한 문서는 거부
/// - Stable은 Beta build를 받지 않고 Beta는 Beta feed의 가장 높은 build를 받음
/// - 최소 OS 미달·현재 build 이하·https 아닌 다운로드 URL·잘못된 항목은 제외
/// - 확인은 background thread에서만 feed를 읽고, 승인 전에는 다운로드가 0회, 승인 시 정확히 1회
/// - http feed URL 거부, 미설정 앱(feed URL 없음)은 scheduler를 만들지 않음
/// 을 확인합니다. 실제 feed 서버·alert 클릭·브라우저 다운로드는 하지 않습니다.
enum UpdateCheck {
    private struct CheckFailure: Error {
        let message: String
    }

    private final class RecordingLoader: UpdateFeedLoading {
        let documents: [URL: Data]
        private let lock = NSLock()
        private var requestsStorage: [URL] = []
        private var mainThreadLoads = 0

        init(documents: [URL: Data]) {
            self.documents = documents
        }

        var requests: [URL] { lock.withLock { requestsStorage } }
        var loadsOnMainThread: Int { lock.withLock { mainThreadLoads } }

        func load(_ url: URL, completion: @escaping (Result<Data, UpdateFeedError>) -> Void) {
            lock.withLock {
                requestsStorage.append(url)
                if Thread.isMainThread { mainThreadLoads += 1 }
            }
            if let data = documents[url] {
                completion(.success(data))
            } else {
                completion(.failure(.loadFailed("fixture에 없는 URL \(url)")))
            }
        }
    }

    private final class ScriptedApprover: UpdateApproving {
        let answer: Bool
        private let lock = NSLock()
        private var askedStorage: [(UpdateItem, UpdateChannel)] = []

        init(answer: Bool) {
            self.answer = answer
        }

        var asked: [(UpdateItem, UpdateChannel)] { lock.withLock { askedStorage } }

        func requestApproval(for item: UpdateItem, channel: UpdateChannel, completion: @escaping (Bool) -> Void) {
            lock.withLock { askedStorage.append((item, channel)) }
            completion(answer)
        }
    }

    private final class RecordingDownloader: UpdateDownloading {
        private let lock = NSLock()
        private var downloadsStorage: [URL] = []

        var downloads: [URL] { lock.withLock { downloadsStorage } }

        func download(_ item: UpdateItem) {
            lock.withLock { downloadsStorage.append(item.downloadURL) }
        }
    }

    private static let stableURL = URL(string: "https://updates.invalid/stellar/stable.json")!
    private static let betaURL = URL(string: "https://updates.invalid/stellar/beta.json")!
    private static let macOS26 = OperatingSystemVersion(majorVersion: 26, minorVersion: 0, patchVersion: 0)

    private static func feed(_ channel: String, _ items: [(String, Int, String, String)]) -> Data {
        let entries = items.map { version, build, minimum, url in
            "{\"version\":\"\(version)\",\"build\":\(build),\"minimumSystemVersion\":\"\(minimum)\",\"downloadURL\":\"\(url)\"}"
        }
        return Data("{\"schema\":1,\"channel\":\"\(channel)\",\"items\":[\(entries.joined(separator: ","))]}".utf8)
    }

    /// 정상 fixture: Stable feed(build 2·3, OS 99 요구 build 4, 잘못된 항목들), Beta feed(build 5).
    private static let documents: [URL: Data] = [
        stableURL: feed("stable", [
            ("0.1.0", 2, "26.0", "https://downloads.invalid/Stellar-0.1.0.zip"),
            ("0.2.0", 3, "26.0", "https://downloads.invalid/Stellar-0.2.0.zip"),
            ("0.3.0", 4, "99.0", "https://downloads.invalid/Stellar-0.3.0.zip"),
            ("0.9.0", 9, "26.0", "http://downloads.invalid/insecure.zip"),
            ("0.8.0", 8, "26.x", "https://downloads.invalid/bad-version.zip"),
            ("0.0.0", 0, "26.0", "https://downloads.invalid/zero-build.zip"),
        ]),
        betaURL: feed("beta", [
            ("0.3.0-beta.1", 5, "26.0", "https://downloads.invalid/Stellar-0.3.0-beta.1.zip"),
        ]),
    ]

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

    /// checker 하나를 실행해 결과와 기록을 돌려줍니다(최대 5초 대기).
    private static func runChecker(
        channel: UpdateChannel, feedURL: URL, documents: [URL: Data], currentBuild: Int, approve: Bool
    ) throws -> (UpdateCheckResult, RecordingLoader, ScriptedApprover, RecordingDownloader) {
        let loader = RecordingLoader(documents: documents)
        let approver = ScriptedApprover(answer: approve)
        let downloader = RecordingDownloader()
        let checker = UpdateChecker(
            channel: channel, feedURL: feedURL, currentBuild: currentBuild, systemVersion: macOS26,
            loader: loader, approver: approver, downloader: downloader)
        let done = DispatchSemaphore(value: 0)
        let box = NSLock()
        var outcome: UpdateCheckResult?
        checker.check { result in
            box.withLock { outcome = result }
            done.signal()
        }
        guard done.wait(timeout: .now() + 5) == .success, let result = box.withLock({ outcome }) else {
            throw CheckFailure(message: "\(channel) 확인이 끝나지 않음")
        }
        return (result, loader, approver, downloader)
    }

    private static func exercise() throws -> String {
        let build3 = URL(string: "https://downloads.invalid/Stellar-0.2.0.zip")!
        let build5 = URL(string: "https://downloads.invalid/Stellar-0.3.0-beta.1.zip")!

        // 1) Stable 거절: Stable feed만 요청, build 3(최소 OS 충족 최고 build)만 제안, 다운로드 0회.
        let (declined, stableLoader, stableApprover, stableDownloads) = try runChecker(
            channel: .stable, feedURL: stableURL, documents: documents, currentBuild: 1, approve: false)
        try check(stableLoader.requests == [stableURL], "stable 요청 URL: \(stableLoader.requests)")
        try check(stableLoader.loadsOnMainThread == 0, "feed를 main thread에서 읽음")
        guard case .declined(let offered) = declined, offered.build == 3, offered.downloadURL == build3 else {
            throw CheckFailure(message: "stable 거절 결과: \(declined)")
        }
        try check(stableApprover.asked.count == 1 && stableApprover.asked[0].1 == .stable, "stable 승인 요청 횟수")
        try check(stableDownloads.downloads.isEmpty, "승인 전(거절) 다운로드가 일어남")

        // 2) Stable 승인: 같은 항목을 정확히 1회 다운로드.
        let (approved, _, _, approvedDownloads) = try runChecker(
            channel: .stable, feedURL: stableURL, documents: documents, currentBuild: 1, approve: true)
        try check(approved == .approved(offered), "stable 승인 결과: \(approved)")
        try check(approvedDownloads.downloads == [build3], "승인 후 다운로드: \(approvedDownloads.downloads)")

        // 3) Beta: Beta feed만 요청, build 5 제안. Stable 항목(build 2·3)은 보지 않습니다.
        let (beta, betaLoader, betaApprover, _) = try runChecker(
            channel: .beta, feedURL: betaURL, documents: documents, currentBuild: 1, approve: false)
        try check(betaLoader.requests == [betaURL], "beta 요청 URL: \(betaLoader.requests)")
        guard case .declined(let betaOffer) = beta, betaOffer.build == 5, betaOffer.downloadURL == build5 else {
            throw CheckFailure(message: "beta 결과: \(beta)")
        }
        try check(betaApprover.asked.count == 1 && betaApprover.asked[0].1 == .beta, "beta 승인 요청")

        // 4) 채널 격리 위반: Stable URL에서 Beta를 선언한 문서는 거부(승인 요청·다운로드 없음).
        let swapped = [stableURL: documents[betaURL]!]
        let (mismatch, _, mismatchApprover, mismatchDownloads) = try runChecker(
            channel: .stable, feedURL: stableURL, documents: swapped, currentBuild: 1, approve: true)
        try check(
            mismatch == .failed(.channelMismatch(expected: "stable", found: "beta")), "채널 불일치 결과: \(mismatch)")
        try check(mismatchApprover.asked.isEmpty && mismatchDownloads.downloads.isEmpty, "채널 불일치에서 승인·다운로드 발생")

        // 5) 최신: Beta build 5에서 Beta feed → upToDate(승인 요청 없음).
        let (latest, _, latestApprover, _) = try runChecker(
            channel: .beta, feedURL: betaURL, documents: documents, currentBuild: 5, approve: true)
        try check(latest == .upToDate && latestApprover.asked.isEmpty, "최신 결과: \(latest)")

        // 6) http feed URL은 요청하지 않고 거부.
        let insecure = URL(string: "http://updates.invalid/stellar/stable.json")!
        let (http, httpLoader, _, _) = try runChecker(
            channel: .stable, feedURL: insecure, documents: documents, currentBuild: 1, approve: true)
        try check(http == .failed(.insecureFeedURL) && httpLoader.requests.isEmpty, "http feed 결과: \(http)")

        // 7) feed 해석: Stable 문서에서 잘못된 항목 3개(http 다운로드, 버전 형식, build 0)는 제외.
        guard case .success(let parsed) = UpdateFeed.parse(documents[stableURL]!, expected: .stable) else {
            throw CheckFailure(message: "stable feed 해석 실패")
        }
        try check(parsed.items.map(\.build) == [2, 3, 4], "유효 항목: \(parsed.items.map(\.build))")
        try check(
            UpdateFeed.parse(Data("{\"schema\":2,\"channel\":\"stable\",\"items\":[]}".utf8), expected: .stable)
                .failureValue == .unsupportedSchema(2), "schema 2 거부")
        try check(UpdateFeed.parse(Data("not json".utf8), expected: .beta).failureValue == .malformed, "JSON 거부")

        // 8) 채널 선호값: beta만 Beta, 그 밖은 Stable.
        let suiteName = "xyz.taeyoon.stellar.update-check.\(ProcessInfo.processInfo.processIdentifier)"
        guard let defaults = UserDefaults(suiteName: suiteName) else { throw CheckFailure(message: "defaults 생성 실패") }
        defer { defaults.removePersistentDomain(forName: suiteName) }
        try check(UpdateChannel.preferred(defaults) == .stable, "기본 채널")
        defaults.set("beta", forKey: "StellarUpdateChannel")
        try check(UpdateChannel.preferred(defaults) == .beta, "beta 채널 설정")
        defaults.set("nightly", forKey: "StellarUpdateChannel")
        try check(UpdateChannel.preferred(defaults) == .stable, "알 수 없는 채널은 stable")

        // 9) scheduler: 채널별 identifier, 하루 1회 반복, utility QoS(시작하지 않고 구성만 확인).
        let scheduler = UpdateScheduler(
            checker: UpdateChecker(
                channel: .beta, feedURL: betaURL, currentBuild: 1, systemVersion: macOS26,
                loader: RecordingLoader(documents: [:]), approver: ScriptedApprover(answer: false),
                downloader: RecordingDownloader()))
        try check(scheduler.activity.identifier == "xyz.taeyoon.stellar.update-check.beta", "scheduler identifier")
        try check(
            scheduler.activity.repeats && scheduler.activity.interval == 86_400
                && scheduler.activity.qualityOfService == .utility, "scheduler 구성")

        // 10) 이 빌드에는 feed URL이 설정되어 있지 않아 업데이트 확인을 시작하지 않습니다.
        try check(UpdateConfiguration.makeScheduler(defaults: defaults) == nil, "미설정 앱이 scheduler를 만듦")

        return "channel isolation(stable↔beta), approval gate(거절 0회·승인 1회 다운로드), "
            + "feed 검증, background loader, scheduler 구성"
    }
}

private extension Result {
    var failureValue: Failure? {
        if case .failure(let error) = self { return error }
        return nil
    }
}
