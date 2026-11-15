import AppKit

// 코어 연결 전 셸 프레임을 띄우고 검사하기 위한 의도적 placeholder(이름에 "Stub" 포함).
// 코어 연결 시 FFI 기반 data source와 각 영역의 실제 슬롯으로 바꿉니다.

/// 고정 sample 값을 돌려주는 셸 프레임 data source.
///
/// 명령을 기록하고, 도구 선택처럼 snapshot이 바뀌는 명령은 세대 번호를 올려 다시 보냅니다. 동작 명령은
/// 기록만 합니다.
@MainActor
final class ShellFrameDataSourceStub: ShellFrameDataSource {
    private(set) var currentSnapshot: ShellFrameSnapshot
    var snapshotHandler: ((ShellFrameSnapshot) -> Void)?
    /// 받은 명령(검사용).
    private(set) var sentCommands: [ShellFrameCommand] = []

    static let sampleTools = [
        ShellToolDescriptor(id: ShellCommandID("tool.select"), label: "Select", symbolName: "cursorarrow"),
        ShellToolDescriptor(id: ShellCommandID("tool.pan"), label: "Pan", symbolName: "hand.raised"),
        ShellToolDescriptor(id: ShellCommandID("tool.zoom"), label: "Zoom", symbolName: "plus.magnifyingglass"),
    ]

    static let sampleMoreActions = [
        ShellActionDescriptor(id: ShellCommandID("project.saveNow"), title: "Save Now"),
        ShellActionDescriptor(id: ShellCommandID("project.revealInFinder"), title: "Show in Finder"),
        ShellActionDescriptor(id: ShellCommandID("project.settings"), title: "Project Settings…", isEnabled: false),
    ]

    static let sampleEmptyStatus = ShellFrameStatus.empty(
        title: "No Project Open",
        message: "Create a new project, or open or clone an existing one.",
        actions: [
            ShellActionDescriptor(id: ShellCommandID("project.new"), title: "New Project…", isPrimary: true),
            ShellActionDescriptor(id: ShellCommandID("project.open"), title: "Open Project…"),
            ShellActionDescriptor(id: ShellCommandID("project.clone"), title: "Clone Project…"),
        ]
    )

    static let sampleLoadingStatus = ShellFrameStatus.loading(message: "Opening Project…")

    static let sampleFailedStatus = ShellFrameStatus.failed(
        title: "Couldn’t Open the Project",
        message: "The project folder is no longer available. Choose the folder again.",
        recoveryActions: [
            ShellActionDescriptor(id: ShellCommandID("project.locate"), title: "Locate Folder…", isPrimary: true),
            ShellActionDescriptor(id: ShellCommandID("project.close"), title: "Close Project"),
        ]
    )

    init(status: ShellFrameStatus = .ready) {
        currentSnapshot = ShellFrameSnapshot(
            generation: 1,
            title: "Sample Project",
            subtitle: "Page 2",
            status: status,
            tools: Self.sampleTools,
            activeToolID: Self.sampleTools[0].id,
            moreActions: Self.sampleMoreActions
        )
    }

    func send(_ command: ShellFrameCommand) {
        sentCommands.append(command)
        var next = currentSnapshot
        switch command {
        case .selectTool(let id):
            guard next.tools.contains(where: { $0.id == id }) else { return }
            next.activeToolID = id
        case .perform:
            return
        }
        publish(next)
    }

    /// 창 상태를 바꿔 새 세대로 보냅니다.
    func setStatus(_ status: ShellFrameStatus) {
        var next = currentSnapshot
        next.status = status
        publish(next)
    }

    /// 세대 번호를 하나 올려 보냅니다.
    func publish(_ snapshot: ShellFrameSnapshot) {
        var next = snapshot
        next.generation = currentSnapshot.generation + 1
        currentSnapshot = next
        snapshotHandler?(next)
    }

    /// 세대 번호를 바꾸지 않고 그대로 보냅니다(늦게 도착한 오래된 snapshot 재현용).
    func deliverUnchecked(_ snapshot: ShellFrameSnapshot) {
        snapshotHandler?(snapshot)
    }
}

/// 사이드바·인스펙터·보조 패널 자리를 표시하는 placeholder pane.
///
/// 아이콘, 이름, 안내문만 표준 레이블로 보여 줍니다. 배경을 칠하지 않아 사이드바·인스펙터의 시스템
/// 재질이 그대로 보입니다(가이드 3.2).
final class ShellPanePlaceholderStub: NSViewController, ShellSidebarSlot, ShellInspectorSlot,
    ShellAuxiliaryPanelSlot
{
    let paneTitle: String
    let symbolName: String
    let message: String

    var paneViewController: NSViewController { self }

    init(title: String, symbolName: String, message: String) {
        paneTitle = title
        self.symbolName = symbolName
        self.message = message
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("ShellPanePlaceholderStub은 코드로만 생성합니다")
    }

    override func loadView() {
        let root = NSView()
        root.setAccessibilityElement(true)
        root.setAccessibilityRole(.group)
        root.setAccessibilityLabel(paneTitle)

        let imageView = NSImageView()
        imageView.image = NSImage(systemSymbolName: symbolName, accessibilityDescription: nil)
        imageView.symbolConfiguration = NSImage.SymbolConfiguration(textStyle: .title2, scale: .large)
        imageView.contentTintColor = .tertiaryLabelColor
        imageView.setAccessibilityElement(false)

        let titleField = NSTextField(labelWithString: paneTitle)
        titleField.font = .preferredFont(forTextStyle: .headline)
        titleField.textColor = .secondaryLabelColor

        let messageField = NSTextField(wrappingLabelWithString: message)
        messageField.font = .preferredFont(forTextStyle: .callout)
        messageField.textColor = .secondaryLabelColor
        messageField.alignment = .center
        messageField.isSelectable = false
        messageField.preferredMaxLayoutWidth = ShellFrameMetrics.sidebarThickness.lowerBound

        let stack = NSStackView(views: [imageView, titleField, messageField])
        stack.orientation = .vertical
        stack.alignment = .centerX
        stack.spacing = ShellFrameMetrics.statusStackSpacing
        stack.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(stack)

        // 가운데 정렬 내용이라 창 모서리에 닿지 않으므로 모서리 적응 없이 margins 영역을 씁니다.
        // 사이드바 접기 애니메이션 중 모서리 적응 영역이 제약 갱신을 반복시켜 예외가 났습니다(명세 참고).
        let margins = root.layoutGuide(for: .margins())
        NSLayoutConstraint.activate([
            stack.centerXAnchor.constraint(equalTo: margins.centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: margins.centerYAnchor),
            stack.leadingAnchor.constraint(greaterThanOrEqualTo: margins.leadingAnchor),
            stack.topAnchor.constraint(greaterThanOrEqualTo: margins.topAnchor),
        ])
        view = root
    }
}

extension ShellFrameSlots {
    /// placeholder 사이드바·인스펙터·보조 패널과 기존 캔버스 호스트로 슬롯을 채웁니다.
    @MainActor
    static func stub(runtimeHandle: UInt64?, includesAuxiliaryPanel: Bool = true) -> ShellFrameSlots {
        ShellFrameSlots(
            sidebar: ShellPanePlaceholderStub(
                title: "Sidebar", symbolName: "sidebar.left",
                message: "Documents, pages, views, and layers appear here."
            ),
            content: ShellCanvasHostViewController(runtimeHandle: runtimeHandle),
            inspector: ShellPanePlaceholderStub(
                title: "Inspector", symbolName: "sidebar.right",
                message: "Properties of the selection appear here."
            ),
            auxiliary: includesAuxiliaryPanel
                ? ShellPanePlaceholderStub(
                    title: "Auxiliary Area", symbolName: "rectangle.bottomthird.inset.filled",
                    message: "Git and agent panels can appear here."
                )
                : nil
        )
    }

    /// stub 슬롯이 쓰는 SF Symbol 이름(검사용).
    static let stubSymbolNames = ["sidebar.left", "sidebar.right", "rectangle.bottomthird.inset.filled"]
}
