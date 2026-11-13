import AppKit

/// 앱 업데이트 채널(XVI.9.5). Stable과 Beta는 서로 다른 feed URL을 쓰고 서로의 항목을 받지 않습니다.
enum UpdateChannel: String, CaseIterable {
    case stable
    case beta

    /// 사용자 설정(`UserDefaults` key `StellarUpdateChannel`). 값이 없거나 알 수 없으면 Stable입니다.
    static func preferred(_ defaults: UserDefaults = .standard) -> UpdateChannel {
        defaults.string(forKey: "StellarUpdateChannel").flatMap(UpdateChannel.init(rawValue:)) ?? .stable
    }
}

/// feed의 업데이트 항목 하나.
struct UpdateItem: Equatable {
    /// 표시용 버전(예: "0.1.0").
    let version: String
    /// 비교용 build 번호(`CFBundleVersion` 정수).
    let build: Int
    /// 설치 가능한 최소 macOS.
    let minimumSystemVersion: OperatingSystemVersion
    /// 사용자가 승인한 뒤에만 여는 다운로드 위치(https).
    let downloadURL: URL

    static func == (lhs: UpdateItem, rhs: UpdateItem) -> Bool {
        lhs.version == rhs.version && lhs.build == rhs.build && lhs.downloadURL == rhs.downloadURL
            && lhs.minimumSystemVersion.majorVersion == rhs.minimumSystemVersion.majorVersion
            && lhs.minimumSystemVersion.minorVersion == rhs.minimumSystemVersion.minorVersion
            && lhs.minimumSystemVersion.patchVersion == rhs.minimumSystemVersion.patchVersion
    }
}

/// feed 읽기·검증 오류.
enum UpdateFeedError: Error, Equatable {
    /// feed URL이 https가 아님.
    case insecureFeedURL
    /// 내려받기 실패.
    case loadFailed(String)
    /// JSON 구조가 맞지 않음.
    case malformed
    /// 지원하지 않는 feed schema 버전.
    case unsupportedSchema(Int)
    /// feed가 선언한 채널이 요청한 채널과 다름(채널 격리 위반).
    case channelMismatch(expected: String, found: String)
}

/// 채널 feed 문서(JSON, schema 1).
///
/// ```json
/// { "schema": 1, "channel": "stable",
///   "items": [ { "version": "0.1.0", "build": 2, "minimumSystemVersion": "26.0",
///                "downloadURL": "https://…/Stellar-0.1.0.zip" } ] }
/// ```
/// https가 아닌 다운로드 URL·잘못된 버전 문자열·0 이하 build 항목은 버립니다.
struct UpdateFeed {
    static let schemaVersion = 1

    let channel: UpdateChannel
    let items: [UpdateItem]

    private struct Document: Decodable {
        let schema: Int
        let channel: String
        let items: [Entry]
    }

    private struct Entry: Decodable {
        let version: String
        let build: Int
        let minimumSystemVersion: String
        let downloadURL: String
    }

    /// `expected` 채널의 feed로 해석합니다. 다른 채널을 선언한 문서는 거부합니다.
    static func parse(_ data: Data, expected: UpdateChannel) -> Result<UpdateFeed, UpdateFeedError> {
        guard let document = try? JSONDecoder().decode(Document.self, from: data) else {
            return .failure(.malformed)
        }
        guard document.schema == schemaVersion else {
            return .failure(.unsupportedSchema(document.schema))
        }
        guard document.channel == expected.rawValue else {
            return .failure(.channelMismatch(expected: expected.rawValue, found: document.channel))
        }
        let items = document.items.compactMap { entry -> UpdateItem? in
            guard entry.build > 0,
                let minimum = parseSystemVersion(entry.minimumSystemVersion),
                let url = URL(string: entry.downloadURL), url.scheme == "https", url.host != nil
            else { return nil }
            return UpdateItem(
                version: entry.version, build: entry.build, minimumSystemVersion: minimum, downloadURL: url)
        }
        return .success(UpdateFeed(channel: expected, items: items))
    }

    /// "26", "26.1", "26.1.2" 형식만 받습니다.
    static func parseSystemVersion(_ text: String) -> OperatingSystemVersion? {
        let parts = text.split(separator: ".", omittingEmptySubsequences: false)
        guard (1...3).contains(parts.count) else { return nil }
        var numbers: [Int] = []
        for part in parts {
            guard !part.isEmpty, part.allSatisfy({ $0.isASCII && $0.isNumber }), let value = Int(part) else {
                return nil
            }
            numbers.append(value)
        }
        while numbers.count < 3 { numbers.append(0) }
        return OperatingSystemVersion(majorVersion: numbers[0], minorVersion: numbers[1], patchVersion: numbers[2])
    }
}

/// feed 내려받기(주입 가능). 호출은 background queue에서 일어납니다.
protocol UpdateFeedLoading {
    func load(_ url: URL, completion: @escaping (Result<Data, UpdateFeedError>) -> Void)
}

/// 사용자 승인 요청(주입 가능). `completion(true)`일 때만 다운로드합니다.
protocol UpdateApproving {
    func requestApproval(for item: UpdateItem, channel: UpdateChannel, completion: @escaping (Bool) -> Void)
}

/// 승인된 업데이트의 다운로드 시작(주입 가능).
protocol UpdateDownloading {
    func download(_ item: UpdateItem)
}

/// 한 채널의 업데이트 확인 결과.
enum UpdateCheckResult: Equatable {
    /// 설치할 새 항목이 없음.
    case upToDate
    /// 새 항목이 있으나 사용자가 거절함(다운로드 없음).
    case declined(UpdateItem)
    /// 사용자가 승인해 다운로드를 시작함.
    case approved(UpdateItem)
    /// feed 오류(다운로드 없음).
    case failed(UpdateFeedError)
}

/// 한 채널의 background 업데이트 확인(XVI.9.5).
///
/// 확인 자체는 background queue에서 feed만 읽습니다. 새 항목이 있으면 사용자 승인을 요청하고, 승인한 경우에만
/// 다운로드를 시작합니다. 자동 다운로드·자동 설치·앱 강제 종료는 하지 않습니다.
final class UpdateChecker {
    let channel: UpdateChannel
    let feedURL: URL
    private let currentBuild: Int
    private let systemVersion: OperatingSystemVersion
    private let loader: UpdateFeedLoading
    private let approver: UpdateApproving
    private let downloader: UpdateDownloading
    private let queue = DispatchQueue(label: "xyz.taeyoon.stellar.update-check", qos: .utility)

    init(
        channel: UpdateChannel, feedURL: URL, currentBuild: Int, systemVersion: OperatingSystemVersion,
        loader: UpdateFeedLoading, approver: UpdateApproving, downloader: UpdateDownloading
    ) {
        self.channel = channel
        self.feedURL = feedURL
        self.currentBuild = currentBuild
        self.systemVersion = systemVersion
        self.loader = loader
        self.approver = approver
        self.downloader = downloader
    }

    /// feed에서 현재 build보다 새롭고 이 OS에 설치 가능한 가장 높은 build 항목.
    func newestApplicable(in feed: UpdateFeed) -> UpdateItem? {
        feed.items
            .filter { $0.build > currentBuild && UpdateChecker.version(systemVersion, isAtLeast: $0.minimumSystemVersion) }
            .max { $0.build < $1.build }
    }

    /// background queue에서 확인합니다. 결과는 같은 background queue에서 `completion`으로 전달합니다.
    func check(completion: @escaping (UpdateCheckResult) -> Void) {
        queue.async { [self] in
            guard feedURL.scheme == "https" else {
                completion(.failed(.insecureFeedURL))
                return
            }
            loader.load(feedURL) { [self] loaded in
                queue.async { [self] in
                    let parsed = loaded.flatMap { UpdateFeed.parse($0, expected: channel) }
                    switch parsed {
                    case .failure(let error):
                        completion(.failed(error))
                    case .success(let feed):
                        guard let item = newestApplicable(in: feed) else {
                            completion(.upToDate)
                            return
                        }
                        approver.requestApproval(for: item, channel: channel) { [self] approved in
                            if approved {
                                downloader.download(item)
                                completion(.approved(item))
                            } else {
                                completion(.declined(item))
                            }
                        }
                    }
                }
            }
        }
    }

    static func version(_ version: OperatingSystemVersion, isAtLeast minimum: OperatingSystemVersion) -> Bool {
        (version.majorVersion, version.minorVersion, version.patchVersion)
            >= (minimum.majorVersion, minimum.minorVersion, minimum.patchVersion)
    }
}

/// 앱 실행 중 주기적 background 확인(`NSBackgroundActivityScheduler`, 하루 1회·utility QoS).
final class UpdateScheduler {
    static let interval: TimeInterval = 24 * 60 * 60
    static let tolerance: TimeInterval = 60 * 60

    let activity: NSBackgroundActivityScheduler
    private let checker: UpdateChecker

    init(checker: UpdateChecker) {
        self.checker = checker
        activity = NSBackgroundActivityScheduler(
            identifier: "xyz.taeyoon.stellar.update-check.\(checker.channel.rawValue)")
        activity.repeats = true
        activity.interval = UpdateScheduler.interval
        activity.tolerance = UpdateScheduler.tolerance
        activity.qualityOfService = .utility
    }

    func start() {
        activity.schedule { [checker] done in
            checker.check { _ in done(.finished) }
        }
    }

    func stop() {
        activity.invalidate()
    }
}

/// 배포 앱 구성: Info.plist의 채널별 feed URL(`StellarUpdateFeedStable`, `StellarUpdateFeedBeta`).
///
/// 값이 없으면 업데이트 확인을 시작하지 않습니다(feed 위치는 배포 결정 후 설정).
enum UpdateConfiguration {
    static func feedURL(for channel: UpdateChannel, bundle: Bundle = .main) -> URL? {
        let key = channel == .stable ? "StellarUpdateFeedStable" : "StellarUpdateFeedBeta"
        guard let text = bundle.object(forInfoDictionaryKey: key) as? String,
            let url = URL(string: text), url.scheme == "https"
        else { return nil }
        return url
    }

    static func currentBuild(bundle: Bundle = .main) -> Int {
        (bundle.object(forInfoDictionaryKey: "CFBundleVersion") as? String).flatMap(Int.init) ?? 0
    }

    /// 설정된 경우에만 선호 채널의 scheduler를 만듭니다.
    static func makeScheduler(bundle: Bundle = .main, defaults: UserDefaults = .standard) -> UpdateScheduler? {
        let channel = UpdateChannel.preferred(defaults)
        guard let url = feedURL(for: channel, bundle: bundle) else { return nil }
        let checker = UpdateChecker(
            channel: channel, feedURL: url, currentBuild: currentBuild(bundle: bundle),
            systemVersion: ProcessInfo.processInfo.operatingSystemVersion,
            loader: URLSessionFeedLoader(), approver: AlertUpdateApprover(), downloader: BrowserUpdateDownloader())
        return UpdateScheduler(checker: checker)
    }
}

/// https feed를 ephemeral session으로 읽습니다(cookie·cache 없음).
struct URLSessionFeedLoader: UpdateFeedLoading {
    func load(_ url: URL, completion: @escaping (Result<Data, UpdateFeedError>) -> Void) {
        guard url.scheme == "https" else {
            completion(.failure(.insecureFeedURL))
            return
        }
        let session = URLSession(configuration: .ephemeral)
        session.dataTask(with: url) { data, response, error in
            if let error {
                completion(.failure(.loadFailed(error.localizedDescription)))
            } else if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
                completion(.failure(.loadFailed("HTTP \(http.statusCode)")))
            } else {
                completion(.success(data ?? Data()))
            }
            session.finishTasksAndInvalidate()
        }.resume()
    }
}

/// main thread에서 native alert로 승인을 묻습니다(두 선택지: 다운로드, 나중에).
struct AlertUpdateApprover: UpdateApproving {
    func requestApproval(for item: UpdateItem, channel: UpdateChannel, completion: @escaping (Bool) -> Void) {
        DispatchQueue.main.async {
            let alert = NSAlert()
            alert.messageText = "Stellar \(item.version) 업데이트가 있습니다"
            alert.informativeText = "\(channel.rawValue) 채널의 새 버전입니다. 다운로드하면 브라우저에서 내려받습니다."
            alert.addButton(withTitle: "다운로드")
            alert.addButton(withTitle: "나중에")
            completion(alert.runModal() == .alertFirstButtonReturn)
        }
    }
}

/// 승인된 업데이트를 기본 브라우저로 내려받습니다. 설치는 사용자가 공증된 앱으로 직접 합니다.
struct BrowserUpdateDownloader: UpdateDownloading {
    func download(_ item: UpdateItem) {
        DispatchQueue.main.async {
            NSWorkspace.shared.open(item.downloadURL)
        }
    }
}
