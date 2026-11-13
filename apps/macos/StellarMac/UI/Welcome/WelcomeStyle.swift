import AppKit

/// 시작 창 영역의 배치 수치. 한곳에서만 정의합니다(디자인 가이드 4.4).
///
/// 값은 이 구현이 HIG Layout을 근거로 정한 제안값이며, 통합 시 디자인 가이드 12장에 기록합니다.
enum WelcomeMetrics {
    /// 시작 창 콘텐츠 크기(크기 조절 불가 창).
    static let windowContentSize = NSSize(width: 780, height: 480)
    /// 왼쪽 동작 열 폭.
    static let actionsColumnWidth: CGFloat = 280
    /// 앱 아이콘 표시 크기.
    static let appIconSize: CGFloat = 64
    /// 창·sheet 바깥 여백.
    static let outerMargin: CGFloat = 20
    /// 묶음 사이 간격.
    static let groupSpacing: CGFloat = 16
    /// 묶음 안 행 간격.
    static let rowSpacing: CGFloat = 6
    /// sheet 콘텐츠 폭.
    static let sheetWidth: CGFloat = 460
}

/// 시작 창 영역이 공유하는 표준 컨트롤 생성기. 색은 시스템 의미 색만 씁니다(디자인 가이드 4.2).
enum WelcomeViews {
    /// 한 줄 레이블.
    static func label(_ text: String, style: NSFont.TextStyle, color: NSColor = .labelColor) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = NSFont.preferredFont(forTextStyle: style)
        label.textColor = color
        label.lineBreakMode = .byTruncatingTail
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return label
    }

    /// 여러 줄 레이블. 폭은 바깥 제약이 정합니다.
    static func wrappingLabel(_ text: String, style: NSFont.TextStyle, color: NSColor = .labelColor) -> NSTextField {
        let label = NSTextField(wrappingLabelWithString: text)
        label.font = NSFont.preferredFont(forTextStyle: style)
        label.textColor = color
        label.isSelectable = false
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        label.setContentCompressionResistancePriority(.required, for: .vertical)
        label.setContentHuggingPriority(.defaultHigh, for: .vertical)
        return label
    }

    /// 명령·경로처럼 그대로 복사할 값. 시스템 고정폭 글꼴, 선택 가능(디자인 가이드 4.1).
    static func codeLabel(_ text: String) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        let size = NSFont.preferredFont(forTextStyle: .callout).pointSize
        label.font = NSFont.monospacedSystemFont(ofSize: size, weight: .regular)
        label.textColor = .labelColor
        label.isSelectable = true
        label.lineBreakMode = .byTruncatingMiddle
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return label
    }

    /// SF Symbol 이미지 뷰. 장식이면 접근성 트리에서 숨깁니다.
    static func symbolView(
        _ name: String,
        style: NSFont.TextStyle,
        scale: NSImage.SymbolScale = .medium,
        multicolor: Bool = false,
        decorative: Bool = true
    ) -> NSImageView {
        var configuration = NSImage.SymbolConfiguration(textStyle: style, scale: scale)
        if multicolor {
            configuration = configuration.applying(.preferringMulticolor())
        }
        let image = NSImage(systemSymbolName: name, accessibilityDescription: nil)?
            .withSymbolConfiguration(configuration)
        let view = NSImageView()
        view.image = image
        view.contentTintColor = multicolor ? nil : .secondaryLabelColor
        view.setContentHuggingPriority(.required, for: .horizontal)
        view.setContentCompressionResistancePriority(.required, for: .horizontal)
        if decorative {
            view.setAccessibilityElement(false)
        }
        return view
    }

    /// 표준 push 버튼.
    static func button(_ title: String, target: AnyObject?, action: Selector?) -> NSButton {
        let button = NSButton(title: title, target: target, action: action)
        button.bezelStyle = .push
        return button
    }

    /// SF Symbol과 제목을 함께 쓰는 동작 버튼(시작 창 왼쪽 열).
    static func actionButton(_ title: String, symbol: String, target: AnyObject?, action: Selector?) -> NSButton {
        let button = NSButton(title: title, target: target, action: action)
        button.bezelStyle = .push
        button.controlSize = .large
        button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
        button.imagePosition = .imageLeading
        button.imageHugsTitle = true
        return button
    }

    /// 내용 폭에 맞는 가로 묶음. 바깥 세로 묶음 안에서 폭이 모호해지지 않도록 hugging을 높입니다.
    static func row(_ views: [NSView]) -> NSStackView {
        let stack = NSStackView(views: views)
        configureRow(stack)
        return stack
    }

    static func configureRow(_ stack: NSStackView) {
        stack.orientation = .horizontal
        stack.distribution = .fill
        stack.alignment = .centerY
        stack.spacing = WelcomeMetrics.rowSpacing
        stack.setHuggingPriority(.defaultHigh, for: .horizontal)
        stack.setHuggingPriority(.defaultHigh, for: .vertical)
    }

    /// 가로 구분선.
    static func separator() -> NSBox {
        let box = NSBox()
        box.boxType = .separator
        return box
    }
}

/// 글자를 클립보드에 복사하는 아이콘 버튼. 아이콘만 있으므로 레이블과 도움말을 반드시 붙입니다(가이드 8.1, 8.3).
final class WelcomeCopyButton: NSButton {
    var textToCopy = ""

    convenience init(accessibilityLabel label: String) {
        self.init(frame: .zero)
        image = NSImage(systemSymbolName: "doc.on.doc", accessibilityDescription: label)
        imagePosition = .imageOnly
        bezelStyle = .push
        isBordered = false
        toolTip = label
        setAccessibilityLabel(label)
        target = self
        action = #selector(copyText(_:))
        setContentHuggingPriority(.required, for: .horizontal)
        setContentCompressionResistancePriority(.required, for: .horizontal)
    }

    @objc private func copyText(_ sender: Any?) {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(textToCopy, forType: .string)
    }
}

/// 첫 실행 제안을 한 번만 보이기 위한 기록(앱 사용자 기본값). 프로젝트·Git에는 넣지 않습니다.
struct WelcomeFirstLaunchPolicy {
    static let commandInstallOfferedKey = "StellarCommandInstallOffered"

    let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    /// 아직 제안하지 않았고, 설치하거나 고칠 수 있는 상태일 때만 제안합니다.
    /// 보조 실행 파일이 없는 빌드(초기 릴리스, ADR-0005 결정 7)에서는 제안하지 않습니다.
    func shouldOfferCommandInstall(for status: StellarCommandInstallStatus) -> Bool {
        guard !defaults.bool(forKey: WelcomeFirstLaunchPolicy.commandInstallOfferedKey) else { return false }
        switch status {
        case .ready, .needsRepair:
            return true
        case .blocked(_, let fallback):
            // 대체 이름으로도 설치할 수 없으면 첫 실행에 막다른 안내를 띄우지 않습니다.
            return fallback != nil
        case .helperUnavailable, .unsupportedAppLocation, .installed, .noWritableLocation:
            return false
        }
    }

    /// 사용자가 제안에 답했습니다(설치 또는 나중에).
    func recordCommandInstallOffered() {
        defaults.set(true, forKey: WelcomeFirstLaunchPolicy.commandInstallOfferedKey)
    }
}
