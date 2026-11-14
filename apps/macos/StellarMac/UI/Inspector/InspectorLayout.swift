import AppKit

// 인스펙터 페이지 내용의 수동 배치(Auto Layout 제약 없음).
//
// macOS 26에서 NSStackView·제약 기반 페이지는 페이지 하나에 제약 1,200~2,000개를 만들고, 새 페이지를 창에
// 넣을 때 창의 제약 엔진이 이를 한 번에 풀어 30~90ms가 걸렸습니다. 행을 하나씩 넣어도 stack 전체를 다시 풀어
// 행마다 15~40ms가 걸려 나눠 만들 수도 없었습니다(docs/design/ui/inspector.md 8.3절 측정).
// 페이지 내용은 frame으로 배치하고 컨트롤 크기는 만들 때 한 번만 잽니다. 배치는 저장한 크기로 산술만 하므로
// 대기 페이지가 창의 배치 비용을 늘리지 않고, 페이지를 컨트롤 하나 단위로 나눠 만들 수 있습니다.

/// 폭이 정해지면 높이를 돌려주는 배치 단위(섹션, 행 묶음, 안내 문구 등).
@MainActor
protocol InspectorFlowContent: NSView {
    /// 주어진 폭에서의 높이. 컨트롤을 새로 재지 않고 저장한 크기만 씁니다.
    func flowHeight(forWidth width: CGFloat) -> CGFloat
}

/// 수동 배치 컨테이너의 공통 동작. 위에서 아래로 쌓고, 크기가 바뀌면 다시 배치합니다.
class InspectorFlowView: NSView, InspectorFlowContent {
    override var isFlipped: Bool { true }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("인스펙터 뷰는 코드로만 생성합니다")
    }

    override func setFrameSize(_ newSize: NSSize) {
        let changed = newSize != frame.size
        super.setFrameSize(newSize)
        if changed { needsLayout = true }
    }

    func flowHeight(forWidth width: CGFloat) -> CGFloat { 0 }

    /// 내용 높이나 배치가 바뀌었음을 이 뷰부터 페이지까지 알립니다.
    func invalidateFlow() {
        InspectorLayout.invalidate(from: self)
    }
}

@MainActor
enum InspectorLayout {
    /// 정렬 사각형 기준 고유 크기(Auto Layout이 쓰는 기준과 같음). 고유 폭이 없으면 폭 0(유연한 폭)입니다.
    static func measure(_ view: NSView) -> NSSize {
        let intrinsic = view.intrinsicContentSize
        var size = NSSize(width: max(intrinsic.width, 0), height: max(intrinsic.height, 0))
        if intrinsic.height == NSView.noIntrinsicMetric {
            size.height = view.alignmentRect(forFrame: NSRect(origin: .zero, size: view.fittingSize)).height
        }
        return size
    }

    /// 정렬 사각형을 frame으로 바꿔 화면 픽셀에 맞춰 놓습니다. 같은 frame이면 건드리지 않습니다.
    static func place(_ view: NSView, alignmentRect rect: NSRect) {
        var frame = view.frame(forAlignmentRect: rect)
        if let superview = view.superview, superview.window != nil {
            frame = superview.backingAlignedRect(frame, options: .alignAllEdgesNearest)
        }
        if view.frame != frame { view.frame = frame }
    }

    /// 높이 `height`인 줄 안에서 세로 가운데에 놓습니다.
    static func place(_ view: NSView, x: CGFloat, width: CGFloat, size: NSSize, lineY: CGFloat, lineHeight: CGFloat) {
        let y = lineY + (lineHeight - size.height) / 2
        place(view, alignmentRect: NSRect(x: x, y: y, width: max(width, 0), height: size.height))
    }

    /// 뷰부터 위로 올라가며 페이지(`InspectorPageView`)까지 다시 배치하게 표시합니다.
    static func invalidate(from view: NSView) {
        var current: NSView? = view
        while let node = current {
            node.needsLayout = true
            if node is InspectorPageView { break }
            current = node.superview
        }
    }
}

/// 폭에 맞춰 줄바꿈하는 보조 설명 레이블. 높이는 폭·문구별로 한 번 계산해 둡니다.
final class InspectorWrappingLabel: NSTextField, InspectorFlowContent {
    private var cachedHeight: (width: CGFloat, text: String, height: CGFloat)?

    init(text: String, font: NSFont = InspectorFonts.caption, color: NSColor = .secondaryLabelColor) {
        super.init(frame: .zero)
        stringValue = text
        isEditable = false
        isSelectable = false
        isBordered = false
        drawsBackground = false
        self.font = font
        textColor = color
        lineBreakMode = .byWordWrapping
        maximumNumberOfLines = 0
        cell?.wraps = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("InspectorWrappingLabel은 코드로만 생성합니다")
    }

    func flowHeight(forWidth width: CGFloat) -> CGFloat {
        if let cachedHeight, cachedHeight.width == width, cachedHeight.text == stringValue {
            return cachedHeight.height
        }
        let bounds = NSRect(x: 0, y: 0, width: max(width, 1), height: .greatestFiniteMagnitude)
        let height = ceil(cell?.cellSize(forBounds: bounds).height ?? 0)
        cachedHeight = (width, stringValue, height)
        return height
    }
}

/// 위에서 아래로 쌓는 스크롤 문서 뷰.
final class InspectorFlippedView: NSView {
    override var isFlipped: Bool { true }
}

/// 인스펙터 스크롤 안에 넣는 고정 높이 목록용 scroll view. 스크롤 휠은 바깥 인스펙터로 넘깁니다.
final class InspectorEmbeddedScrollView: NSScrollView {
    override func scrollWheel(with event: NSEvent) {
        nextResponder?.scrollWheel(with: event)
    }
}
