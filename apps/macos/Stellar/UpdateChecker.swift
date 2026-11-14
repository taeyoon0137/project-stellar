import AppKit
import Sparkle

/// 앱 업데이트 채널(XVI.9.5, ADR-0017).
///
/// Stable과 Beta는 서로 다른 appcast URL을 씁니다. 그와 별도로 Sparkle channel 필터를 겹쳐, Stable은 기본 채널
/// 항목(`<sparkle:channel>` 없음)만, Beta는 기본 채널과 `beta` 채널 항목을 받습니다. 그래서 Beta 항목이 Stable
/// appcast에 잘못 올라가도 Stable 사용자에게는 제안되지 않습니다.
enum UpdateChannel: String, CaseIterable {
    case stable
    case beta

    /// 사용자 설정 key(`UserDefaults`).
    static let defaultsKey = "StellarUpdateChannel"

    /// 사용자 설정 채널. 값이 없거나 알 수 없으면 Stable입니다.
    static func preferred(_ defaults: UserDefaults = .standard) -> UpdateChannel {
        defaults.string(forKey: defaultsKey).flatMap(UpdateChannel.init(rawValue:)) ?? .stable
    }

    /// Sparkle `allowedChannels`. 기본 채널은 Sparkle이 항상 허용하므로 Stable은 빈 집합입니다.
    var sparkleChannels: Set<String> {
        self == .beta ? ["beta"] : []
    }
}

/// 업데이트 구성 Info.plist key. 값은 `apps/macos/Stellar/Info.plist`가 Xcode build setting
/// `STELLAR_UPDATE_FEED_STABLE`·`STELLAR_UPDATE_FEED_BETA`·`STELLAR_UPDATE_PUBLIC_ED_KEY`에서 채웁니다.
enum UpdateInfoKey {
    static let stableFeed = "SUFeedURL"
    static let betaFeed = "StellarUpdateFeedBeta"
    static let publicKey = "SUPublicEDKey"
    static let requireSignedFeed = "SURequireSignedFeed"
    static let verifyBeforeExtraction = "SUVerifyUpdateBeforeExtraction"
    static let allowsAutomaticUpdates = "SUAllowsAutomaticUpdates"
    static let automaticallyUpdate = "SUAutomaticallyUpdate"
}

/// 업데이트를 켤 수 없는 구성. 앱은 이 경우 updater를 시작하지 않고 네트워크 요청도 하지 않습니다.
enum UpdateConfigurationError: Error, Equatable {
    /// 채널 appcast URL이 없거나 비어 있음.
    case missingFeedURL(UpdateChannel)
    /// 채널 appcast URL이 허용 scheme(배포 앱은 https)이 아님.
    case insecureFeedURL(UpdateChannel)
    /// Stable과 Beta가 같은 appcast를 가리킴(채널 분리 위반).
    case sharedFeedURL
    /// appcast 서명 요구(`SURequireSignedFeed`)나 추출 전 검증(`SUVerifyUpdateBeforeExtraction`)이 꺼져 있음.
    case unsignedFeedAllowed
    /// 자동 다운로드·설치(`SUAllowsAutomaticUpdates`, `SUAutomaticallyUpdate`)가 명시적으로 꺼져 있지 않음.
    case automaticInstallAllowed
    /// EdDSA 공개 키가 없음(배포 키 생성 전, 사용자 단계).
    case missingPublicKey
    /// EdDSA 공개 키가 base64 32바이트가 아님.
    case invalidPublicKey
}

/// 검증된 업데이트 구성.
struct UpdateConfiguration: Equatable {
    let stableFeedURL: URL
    let betaFeedURL: URL
    /// Ed25519 공개 키(32바이트). appcast와 업데이트 archive 서명을 이 키로 검증합니다.
    let publicKey: Data

    /// Info.plist 사전에서 구성을 읽습니다. `allowedSchemes`는 배포 앱에서 https만이며, headless fixture만
    /// `file`을 씁니다.
    static func load(
        info: [String: Any], allowedSchemes: Set<String> = ["https"]
    ) -> Result<UpdateConfiguration, UpdateConfigurationError> {
        func feed(_ key: String, _ channel: UpdateChannel) -> Result<URL, UpdateConfigurationError> {
            guard let text = (info[key] as? String)?.trimmingCharacters(in: .whitespaces), !text.isEmpty,
                let url = URL(string: text)
            else { return .failure(.missingFeedURL(channel)) }
            guard let scheme = url.scheme?.lowercased(), allowedSchemes.contains(scheme),
                url.isFileURL || url.host?.isEmpty == false
            else { return .failure(.insecureFeedURL(channel)) }
            return .success(url)
        }

        let stable: URL
        let beta: URL
        switch feed(UpdateInfoKey.stableFeed, .stable) {
        case .failure(let error): return .failure(error)
        case .success(let url): stable = url
        }
        switch feed(UpdateInfoKey.betaFeed, .beta) {
        case .failure(let error): return .failure(error)
        case .success(let url): beta = url
        }
        guard stable.absoluteString != beta.absoluteString else { return .failure(.sharedFeedURL) }

        // 서명된 appcast와 추출 전 archive 서명 검증을 모두 요구합니다(feed 위조로 다운로드 위치를 바꾸지 못함).
        guard info[UpdateInfoKey.requireSignedFeed] as? Bool == true,
            info[UpdateInfoKey.verifyBeforeExtraction] as? Bool == true
        else { return .failure(.unsignedFeedAllowed) }
        // Sparkle은 key가 없으면 자동 업데이트를 허용하므로 명시적 false만 받습니다.
        guard info[UpdateInfoKey.allowsAutomaticUpdates] as? Bool == false,
            info[UpdateInfoKey.automaticallyUpdate] as? Bool == false
        else { return .failure(.automaticInstallAllowed) }

        guard let keyText = (info[UpdateInfoKey.publicKey] as? String)?.trimmingCharacters(in: .whitespaces),
            !keyText.isEmpty
        else { return .failure(.missingPublicKey) }
        guard let key = Data(base64Encoded: keyText), key.count == 32 else { return .failure(.invalidPublicKey) }
        return .success(UpdateConfiguration(stableFeedURL: stable, betaFeedURL: beta, publicKey: key))
    }

    func feedURL(for channel: UpdateChannel) -> URL {
        channel == .stable ? stableFeedURL : betaFeedURL
    }
}

/// 채널 선택 정책. Sparkle은 확인할 때마다 delegate로 읽으므로 채널 변경은 다음 확인부터 적용됩니다.
struct UpdateChannelPolicy {
    let configuration: UpdateConfiguration
    let defaults: UserDefaults

    var channel: UpdateChannel { UpdateChannel.preferred(defaults) }
    /// 이 채널의 appcast 하나만 요청합니다.
    var feedURLString: String { configuration.feedURL(for: channel).absoluteString }
    var allowedChannels: Set<String> { channel.sparkleChannels }
}

/// 채널 정책만 Sparkle에 전달하는 updater delegate.
final class UpdateChannelDelegate: NSObject, SPUUpdaterDelegate {
    let policy: UpdateChannelPolicy

    init(policy: UpdateChannelPolicy) {
        self.policy = policy
    }

    func feedURLString(for updater: SPUUpdater) -> String? {
        policy.feedURLString
    }

    func allowedChannels(for updater: SPUUpdater) -> Set<String> {
        policy.allowedChannels
    }
}

/// 앱 업데이트(XVI.9.5, ADR-0017): Sparkle updater를 구성이 완전할 때만 시작합니다.
///
/// - 확인: `SUEnableAutomaticChecks`·`SUScheduledCheckInterval`(하루 1회)에 따른 background 확인.
/// - 승인: Sparkle 표준 UI가 새 버전을 알리고, 사용자가 설치를 고른 뒤에만 다운로드합니다. 설치·재실행도
///   사용자 선택 후입니다. 자동 다운로드·자동 설치는 Info.plist에서 끕니다.
/// - 무결성: appcast와 archive 모두 EdDSA 서명이 맞아야 합니다.
/// 구성이 불완전하면(예: 배포 키 생성 전) updater를 만들지 않아 네트워크 요청이 없습니다.
final class AppUpdateController {
    enum State: Equatable {
        /// 구성이 불완전해 시작하지 않음.
        case disabled(UpdateConfigurationError)
        /// Sparkle이 구성 검사에서 시작을 거부함.
        case failedToStart(String)
        /// 선호 채널로 시작함.
        case started(UpdateChannel)
    }

    let state: State
    private let updater: SPUUpdater?
    private let delegate: UpdateChannelDelegate?
    private let userDriver: SPUStandardUserDriver?

    /// `allowedSchemes`는 배포 앱에서 기본값(https)만 씁니다.
    init(bundle: Bundle = .main, defaults: UserDefaults = .standard, allowedSchemes: Set<String> = ["https"]) {
        switch UpdateConfiguration.load(info: bundle.infoDictionary ?? [:], allowedSchemes: allowedSchemes) {
        case .failure(let reason):
            state = .disabled(reason)
            updater = nil
            delegate = nil
            userDriver = nil
        case .success(let configuration):
            let channelDelegate = UpdateChannelDelegate(
                policy: UpdateChannelPolicy(configuration: configuration, defaults: defaults))
            let driver = SPUStandardUserDriver(hostBundle: bundle, delegate: nil)
            let sparkle = SPUUpdater(
                hostBundle: bundle, applicationBundle: bundle, userDriver: driver, delegate: channelDelegate)
            do {
                try sparkle.start()
                state = .started(channelDelegate.policy.channel)
            } catch {
                state = .failedToStart(error.localizedDescription)
            }
            updater = sparkle
            delegate = channelDelegate
            userDriver = driver
        }
    }

    /// 로그용 한 줄 설명.
    var summary: String {
        switch state {
        case .disabled(let reason): return "disabled(\(reason))"
        case .failedToStart(let message): return "failed(\(message))"
        case .started(let channel): return "started(\(channel.rawValue))"
        }
    }
}
