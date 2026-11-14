import AppKit

/// AppKit 캔버스 호스트(II.5.4, XII.3.2).
///
/// 창의 active View scene을 그리고, backing scale과 drawable 픽셀 크기 변경을
/// `stellar_runtime_set_drawable_size`로, mouse·trackpad·keyboard 이벤트를
/// `stellar_runtime_submit_mac_event`로 코어에 전달합니다.
final class CanvasView: NSView {
    /// drawable·입력을 전달할 대상 runtime handle. 미설정 시 전달하지 않습니다.
    var runtimeHandle: UInt64?

    /// 셸이 코어보다 먼저 해석할 key down(캔버스 단일 키 도구 단축키, tools_menus 6.2). true를 돌려주면 그 키의
    /// key down과 짝 key up을 코어 입력으로 보내지 않습니다. nil이면 모든 키를 코어로 보냅니다.
    var keyDownHandler: ((NSEvent) -> Bool)?
    /// `keyDownHandler`가 처리해 짝 key up도 삼킬 key code.
    private var handledKeyCodes: Set<UInt16> = []

    /// 창별 viewport가 처리한 스크롤은 공용 runtime 입력 대기열에 중복 제출하지 않습니다.
    var scrollWheelHandler: ((NSEvent) -> Bool)?

    private let frameScene = CanvasFrameScene()
    var typingLayer: CanvasFrameScene.TypingLayer? {
        get { frameScene.typingLayer }
        set { frameScene.typingLayer = newValue; needsDisplay = true }
    }
    func textHit(id: UUID? = nil) -> CanvasFrameHit? { frameScene.textHit(id: id) }
    var frameSession: UInt64? { didSet { refreshFrames() } }

    func invalidateComponentFrames() { frameScene.clear(); refreshFrames() }

    func refreshFrames() {
        if let runtimeHandle, let frameSession, bounds.width > 0, bounds.height > 0 {
            frameScene.refresh(handle: runtimeHandle, session: frameSession, size: bounds.size)
        } else { frameScene.clear() }
        needsDisplay = true
    }

    func setFrameDragPreview(id: UUID?, offset: CGVector) {
        frameScene.setDragPreview(id: id, offset: offset)
        needsDisplay = true
    }

    var insertionOutline: CGPath? { didSet { needsDisplay = true } }

    var selectedInspectorFrame: CanvasInspectorFrame? { frameScene.selectedInspectorFrame }

    func frameHit(at point: NSPoint, onlyFrames: Bool = false) -> CanvasFrameHit? { frameScene.hit(at: point, onlyFrames: onlyFrames) }

    var framePointerHandler: ((NSEvent) -> Bool)?
    var cancelFrameGesture: (() -> Bool)?

    override func viewWillMove(toWindow newWindow: NSWindow?) {
        if let window {
            NotificationCenter.default.removeObserver(self, name: NSWindow.didResignKeyNotification, object: window)
        }
        _ = cancelFrameGesture?()
        if let newWindow {
            NotificationCenter.default.addObserver(self, selector: #selector(cancelOnWindowBlur),
                name: NSWindow.didResignKeyNotification, object: newWindow)
        }
        super.viewWillMove(toWindow: newWindow)
    }

    @objc private func cancelOnWindowBlur(_ notification: Notification) { _ = cancelFrameGesture?() }

    override func resignFirstResponder() -> Bool {
        let resigned = super.resignFirstResponder()
        if resigned { _ = cancelFrameGesture?() }
        return resigned
    }

    private var mouseTrackingArea: NSTrackingArea?

    override var isFlipped: Bool { true }

    /// 클릭하면 first responder가 되어 keyboard 이벤트를 받습니다.
    override var acceptsFirstResponder: Bool { true }

    /// 현재 bounds(point)와 backing scale로부터 drawable 픽셀 크기를 계산합니다.
    func currentDrawable() -> (width: UInt32, height: UInt32, scale: Double) {
        let scale = Double(window?.backingScaleFactor ?? 1.0)
        let size = CanvasMetrics.pixelSize(pointSize: bounds.size, scale: scale)
        return (size.width, size.height, scale)
    }

    /// 계산한 drawable 픽셀 크기·scale을 코어로 전달합니다.
    func forwardDrawableSize() {
        guard let handle = runtimeHandle else { return }
        let drawable = currentDrawable()
        _ = stellar_runtime_set_drawable_size(handle, drawable.width, drawable.height, drawable.scale)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        forwardDrawableSize()
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        forwardDrawableSize()
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        forwardDrawableSize()
        refreshFrames()
    }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.windowBackgroundColor.setFill()
        dirtyRect.fill()
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        frameScene.draw(in: context, dirtyRect: dirtyRect)
        if let path = insertionOutline {
            context.saveGState()
            context.addPath(path)
            context.setFillColor(NSColor.controlAccentColor.withAlphaComponent(0.12).cgColor)
            context.setStrokeColor(NSColor.controlAccentColor.cgColor)
            context.setLineWidth(1)
            context.drawPath(using: .fillStroke)
            context.restoreGState()
        }
    }

    // MARK: - 입력 (XII.3.2)

    /// 버튼 없이 움직이는 pointer(hover)도 받도록 보이는 영역 전체에 tracking area를 둡니다.
    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let mouseTrackingArea {
            removeTrackingArea(mouseTrackingArea)
        }
        let area = NSTrackingArea(
            rect: .zero,
            options: [.mouseMoved, .activeInKeyWindow, .inVisibleRect],
            owner: self,
            userInfo: nil
        )
        addTrackingArea(area)
        mouseTrackingArea = area
    }

    override func mouseDown(with event: NSEvent) {
        if framePointerHandler?(event) != true { submitInput(event) }
    }
    override func mouseUp(with event: NSEvent) {
        if framePointerHandler?(event) != true { submitInput(event) }
    }
    override func mouseDragged(with event: NSEvent) {
        if framePointerHandler?(event) != true { submitInput(event) }
    }
    override func mouseMoved(with event: NSEvent) { submitInput(event) }
    override func rightMouseDown(with event: NSEvent) { submitInput(event) }
    override func rightMouseUp(with event: NSEvent) { submitInput(event) }
    override func rightMouseDragged(with event: NSEvent) { submitInput(event) }
    override func otherMouseDown(with event: NSEvent) { submitInput(event) }
    override func otherMouseUp(with event: NSEvent) { submitInput(event) }
    override func otherMouseDragged(with event: NSEvent) { submitInput(event) }
    override func scrollWheel(with event: NSEvent) {
        if scrollWheelHandler?(event) == true { return }
        submitInput(event)
    }
    override func keyDown(with event: NSEvent) {
        if (event.keyCode == 53 && cancelFrameGesture?() == true) || keyDownHandler?(event) == true {
            handledKeyCodes.insert(event.keyCode)
            return
        }
        submitInput(event)
    }

    override func keyUp(with event: NSEvent) {
        if handledKeyCodes.remove(event.keyCode) != nil { return }
        submitInput(event)
    }

    override func flagsChanged(with event: NSEvent) {
        submitInput(event)
        super.flagsChanged(with: event)
    }

    /// NSEvent 원시 필드를 코어 입력 대기열에 제출합니다. 결과 상태와 대기열에 넣은 수를 반환합니다.
    @discardableResult
    func submitInput(_ event: NSEvent) -> (status: StellarStatus, queued: UInt32) {
        guard let handle = runtimeHandle else { return (STELLAR_INVALID_HANDLE, 0) }
        var raw = CanvasInput.macEvent(
            from: event,
            in: self,
            receivedAt: ProcessInfo.processInfo.systemUptime
        )
        var queued: UInt32 = 0
        let status = stellar_runtime_submit_mac_event(handle, &raw, &queued)
        return (status, queued)
    }
}

/// `NSEvent` → `StellarMacEvent` 원시 필드 복사(XII.3.2).
///
/// 이벤트 해석(종류·button·modifier·phase·key code 변환)은 코어(`input_macos.rs`)가 합니다.
/// 셸은 AppKit 값을 그대로 복사하고, 위치만 캔버스 view 좌표로 옮깁니다. AppKit은 이벤트 종류에
/// 맞지 않는 accessor(예: mouse 이벤트의 `keyCode`)를 호출하면 예외를 던지므로 종류별로만 읽습니다.
enum CanvasInput {
    static func macEvent(from event: NSEvent, in view: NSView, receivedAt uptime: TimeInterval) -> StellarMacEvent {
        var raw = StellarMacEvent()
        raw.version = UInt32(STELLAR_MAC_EVENT_VERSION)
        raw.event_type = UInt32(clamping: event.type.rawValue)
        raw.modifier_flags = UInt64(event.modifierFlags.rawValue)
        raw.timestamp = event.timestamp
        raw.receive_uptime = uptime

        switch event.type {
        case .leftMouseDown, .leftMouseUp, .rightMouseDown, .rightMouseUp, .otherMouseDown, .otherMouseUp,
             .mouseMoved, .leftMouseDragged, .rightMouseDragged, .otherMouseDragged:
            let point = viewPoint(of: event, in: view)
            raw.x = Double(point.x)
            raw.y = Double(point.y)
            raw.click_count = Int64(event.clickCount)
            raw.button_number = Int64(event.buttonNumber)
            raw.subtype = event.subtype.rawValue
        case .scrollWheel:
            let point = viewPoint(of: event, in: view)
            raw.x = Double(point.x)
            raw.y = Double(point.y)
            raw.scrolling_delta_x = Double(event.scrollingDeltaX)
            raw.scrolling_delta_y = Double(event.scrollingDeltaY)
            raw.has_precise_deltas = event.hasPreciseScrollingDeltas ? 1 : 0
            raw.phase = UInt64(event.phase.rawValue)
            raw.momentum_phase = UInt64(event.momentumPhase.rawValue)
        case .keyDown, .keyUp:
            raw.key_code = event.keyCode
            raw.is_repeat = event.isARepeat ? 1 : 0
        case .flagsChanged:
            raw.key_code = event.keyCode
        default:
            break
        }
        return raw
    }

    /// 이벤트 위치를 view 좌표로 바꿉니다(캔버스 view는 flipped라 좌상단 원점, y 아래로 증가).
    ///
    /// 창 없이 만든 이벤트(`event.window == nil`)의 `locationInWindow`는 화면 좌표이므로
    /// 화면 → 창 → view 순서로 옮깁니다.
    static func viewPoint(of event: NSEvent, in view: NSView) -> NSPoint {
        var windowPoint = event.locationInWindow
        if let window = view.window, event.window !== window {
            let screenPoint = event.window?.convertPoint(toScreen: windowPoint) ?? windowPoint
            windowPoint = window.convertPoint(fromScreen: screenPoint)
        }
        return view.convert(windowPoint, from: nil)
    }
}
