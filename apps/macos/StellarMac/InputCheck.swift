import AppKit
import Carbon.HIToolbox

/// macOS mouse·trackpad·keyboard 이벤트 → 공통 InputEvent 변환과 대기열 coalescing을 headless로 검증합니다(XII.3.2, XII.3.4).
///
/// 앱 창(`CanvasWindowController`)을 화면에 표시하지 않고 만든 뒤 합성 `NSEvent`를 `CanvasView`에
/// 전달하고(mouse·scroll은 창 hit test로 찾은 view의 처리기, key는 창 `sendEvent` → first responder),
/// 코어 입력 대기열(`stellar_runtime_poll_input`)에서 읽은 InputEvent 바이트(레이아웃 버전 1)를 확인합니다.
/// - coordinate: 창 좌표(좌하단 원점) → 캔버스 view point(좌상단 원점), 창 없는 scroll 이벤트(화면 좌표)
/// - modifiers: Shift·Control·Option·Command·Caps Lock bit, numericPad·function·device bit 무시
/// - double click: `clickCount` 1·2·3이 down/up에 그대로 기록, drag·move는 0
/// - button: 오른쪽·가운데·뒤로·앞으로, drag·hover move
/// - key: Carbon `kVK_*` 상수 → HID usage, repeat, flagsChanged 좌우 Shift
/// - scroll: trackpad(precise·phase·momentum)와 일반 wheel(line)의 단위·부호
/// - 시각: 코어 단조 domain 간격이 native 간격과 같음
/// - coalescing(XII.3.4): down → 10,000 drag → up이 down·최신 drag 1개·up으로 남음
/// 실제 HID 장치 입력과 사람 조작은 하지 않습니다.
enum InputCheck {
    private struct CheckFailure: Error {
        let message: String
    }

    /// 직렬화된 공통 InputEvent(레이아웃 버전 1).
    private struct InputBytes {
        let bytes: [UInt8]

        var kind: UInt8 { bytes[1] }
        var timestamp: UInt64 { (0..<8).reduce(UInt64(0)) { $0 | UInt64(bytes[2 + $1]) << (8 * UInt64($1)) } }

        func u8(_ offset: Int) -> UInt8 { bytes[offset] }

        func u16(_ offset: Int) -> UInt16 { UInt16(bytes[offset]) | UInt16(bytes[offset + 1]) << 8 }

        func f32(_ offset: Int) -> Float {
            let bits = (0..<4).reduce(UInt32(0)) { $0 | UInt32(bytes[offset + $1]) << (8 * UInt32($1)) }
            return Float(bitPattern: bits)
        }
    }

    // InputEvent 레이아웃(input.rs `InputEvent::encode`) 상수.
    private static let kindPointer: UInt8 = 1
    private static let kindWheel: UInt8 = 2
    private static let kindKey: UInt8 = 3
    // pointer phase / button / device
    private static let phaseDown: UInt8 = 1
    private static let phaseUp: UInt8 = 2
    private static let phaseMove: UInt8 = 3
    private static let buttonNone: UInt8 = 0
    private static let buttonPrimary: UInt8 = 1
    private static let buttonSecondary: UInt8 = 2
    private static let buttonMiddle: UInt8 = 3
    private static let buttonBack: UInt8 = 4
    private static let buttonForward: UInt8 = 5
    private static let deviceMouse: UInt8 = 1
    // modifier bit
    private static let modShift: UInt8 = 1 << 0
    private static let modControl: UInt8 = 1 << 1
    private static let modAlt: UInt8 = 1 << 2
    private static let modMeta: UInt8 = 1 << 3
    private static let modCapsLock: UInt8 = 1 << 4
    // wheel phase / unit
    private static let wheelDiscrete: UInt8 = 1
    private static let wheelBegan: UInt8 = 2
    private static let wheelMomentum: UInt8 = 5
    private static let unitPoint: UInt8 = 1
    private static let unitLine: UInt8 = 2
    // key state
    private static let keyDown: UInt8 = 1
    private static let keyUp: UInt8 = 2

    static func run() -> Bool {
        let application = NSApplication.shared
        application.setActivationPolicy(.accessory)

        let host = RuntimeHost()
        guard host.created else { return report("runtime create 실패") }
        defer { host.shutdown() }

        let controller = CanvasWindowController(runtimeHandle: host.handle)
        guard let window = controller.window,
            let canvas = (window.contentView as? NSSplitView)?.arrangedSubviews.last as? CanvasView
        else {
            return report("창에 CanvasView가 없음")
        }
        window.isReleasedWhenClosed = false
        window.contentView?.layoutSubtreeIfNeeded()
        defer { window.close() }

        do {
            let summary = try exercise(window: window, canvas: canvas, handle: host.handle)
            print("input ok: \(summary)")
            return true
        } catch let failure as CheckFailure {
            return report(failure.message)
        } catch {
            return report("예상하지 못한 오류: \(error)")
        }
    }

    private static func report(_ message: String) -> Bool {
        FileHandle.standardError.write(Data("input 실패: \(message)\n".utf8))
        return false
    }

    private static func check(_ condition: Bool, _ message: @autoclosure () -> String) throws {
        if !condition {
            throw CheckFailure(message: message())
        }
    }

    private static func exercise(window: NSWindow, canvas: CanvasView, handle: UInt64) throws -> String {
        // 캔버스 view의 창 좌표 사각형(좌하단 원점). 기대 view 좌표는 이 값으로 직접 뒤집어 계산합니다.
        let canvasRect = canvas.convert(canvas.bounds, to: nil)
        try check(canvasRect.width > 300 && canvasRect.height > 300, "캔버스 크기 비정상: \(canvasRect)")
        try check(stellarPoll(handle) == nil, "시작 시 입력 대기열이 비어 있지 않음")

        let now = ProcessInfo.processInfo.systemUptime
        var count = 0

        /// 캔버스 view 기준 (x, y)(좌상단 원점)에 해당하는 창 좌표.
        func windowPoint(_ viewX: CGFloat, _ viewY: CGFloat) -> NSPoint {
            NSPoint(x: canvasRect.minX + viewX, y: canvasRect.maxY - viewY)
        }

        func mouse(
            _ type: NSEvent.EventType, at point: NSPoint, flags: NSEvent.ModifierFlags = [],
            clicks: Int = 1, time: TimeInterval
        ) throws -> NSEvent {
            guard
                let event = NSEvent.mouseEvent(
                    with: type, location: point, modifierFlags: flags, timestamp: time,
                    windowNumber: window.windowNumber, context: nil, eventNumber: 0,
                    clickCount: clicks, pressure: 1
                )
            else { throw CheckFailure(message: "mouse 이벤트 생성 실패 \(type)") }
            return event
        }

        /// 창 hit test로 찾은 view의 AppKit 처리기로 mouse 이벤트를 전달하고 대기열에서 하나를 읽습니다.
        func deliverMouse(_ event: NSEvent) throws -> InputBytes {
            guard let content = window.contentView else { throw CheckFailure(message: "contentView 없음") }
            // 창 없이 만든 이벤트(CGEvent 경유)의 위치는 화면 좌표입니다.
            let eventWindowPoint =
                event.window == nil
                ? window.convertPoint(fromScreen: event.locationInWindow) : event.locationInWindow
            let pointInSuperview = content.superview?.convert(eventWindowPoint, from: nil) ?? eventWindowPoint
            guard let hit = content.hitTest(pointInSuperview), hit === canvas else {
                throw CheckFailure(message: "hit test가 캔버스가 아님 (\(eventWindowPoint))")
            }
            switch event.type {
            case .leftMouseDown: hit.mouseDown(with: event)
            case .leftMouseUp: hit.mouseUp(with: event)
            case .leftMouseDragged: hit.mouseDragged(with: event)
            case .mouseMoved: hit.mouseMoved(with: event)
            case .rightMouseDown: hit.rightMouseDown(with: event)
            case .rightMouseUp: hit.rightMouseUp(with: event)
            case .otherMouseDown: hit.otherMouseDown(with: event)
            case .otherMouseUp: hit.otherMouseUp(with: event)
            case .scrollWheel: hit.scrollWheel(with: event)
            default: throw CheckFailure(message: "지원하지 않는 검사 이벤트 \(event.type)")
            }
            return try pollOne("\(event.type)")
        }

        func pollOne(_ label: String) throws -> InputBytes {
            guard let bytes = stellarPoll(handle) else {
                throw CheckFailure(message: "\(label): 대기열에 이벤트 없음")
            }
            try check(stellarPoll(handle) == nil, "\(label): 이벤트가 2개 이상 들어감")
            count += 1
            return InputBytes(bytes: bytes)
        }

        func expectPointer(
            _ event: InputBytes, _ label: String, phase: UInt8, button: UInt8, clicks: UInt8,
            modifiers: UInt8, x: Float, y: Float
        ) throws {
            try check(event.kind == kindPointer && event.bytes.count == 23, "\(label): pointer 레이아웃 아님")
            try check(event.u8(10) == phase, "\(label): phase \(event.u8(10)) != \(phase)")
            try check(event.u8(11) == button, "\(label): button \(event.u8(11)) != \(button)")
            try check(event.u8(12) == deviceMouse, "\(label): device \(event.u8(12))")
            try check(event.u8(13) == clicks, "\(label): click count \(event.u8(13)) != \(clicks)")
            try check(event.u8(14) == modifiers, "\(label): modifiers \(event.u8(14)) != \(modifiers)")
            try check(
                event.f32(15) == x && event.f32(19) == y,
                "\(label): 좌표 (\(event.f32(15)), \(event.f32(19))) != (\(x), \(y))"
            )
        }

        // 1) coordinate: 좌상단 원점 view point로 뒤집힘. 두 위치(정수·소수).
        let p1 = windowPoint(10, 20)
        let down1 = try deliverMouse(try mouse(.leftMouseDown, at: p1, time: now))
        try expectPointer(
            down1, "coordinate 1", phase: phaseDown, button: buttonPrimary, clicks: 1, modifiers: 0, x: 10, y: 20)
        let p2 = windowPoint(200.5, 30.25)
        let up1 = try deliverMouse(try mouse(.leftMouseUp, at: p2, time: now + 0.25))
        try expectPointer(
            up1, "coordinate 2", phase: phaseUp, button: buttonPrimary, clicks: 1, modifiers: 0, x: 200.5, y: 30.25)
        // 코어 단조 domain 간격 == native 간격(0.25초, ns 반올림 오차 1µs 이내).
        let gap = Int64(bitPattern: up1.timestamp &- down1.timestamp)
        try check(abs(gap - 250_000_000) <= 1_000, "timestamp 간격 \(gap)ns != 250ms")

        // 2) double·triple click: clickCount가 down/up에 그대로 기록됩니다.
        for clicks in 1...3 {
            let time = now + 1 + Double(clicks) * 0.1
            let down = try deliverMouse(try mouse(.leftMouseDown, at: p1, clicks: clicks, time: time))
            try expectPointer(
                down, "click \(clicks) down", phase: phaseDown, button: buttonPrimary, clicks: UInt8(clicks),
                modifiers: 0, x: 10, y: 20)
            let up = try deliverMouse(try mouse(.leftMouseUp, at: p1, clicks: clicks, time: time + 0.05))
            try expectPointer(
                up, "click \(clicks) up", phase: phaseUp, button: buttonPrimary, clicks: UInt8(clicks),
                modifiers: 0, x: 10, y: 20)
        }

        // 3) modifiers: device-independent bit만 반영(numericPad·function·device-dependent bit 무시).
        let modifierCases: [(NSEvent.ModifierFlags, UInt8)] = [
            ([.shift], modShift),
            ([.control], modControl),
            ([.option], modAlt),
            ([.command], modMeta),
            ([.capsLock], modCapsLock),
            ([.shift, .command], modShift | modMeta),
            ([.control, .option, .capsLock], modControl | modAlt | modCapsLock),
            ([.numericPad, .function, NSEvent.ModifierFlags(rawValue: 0x2 | 0x2000)], 0),
        ]
        for (index, (flags, bits)) in modifierCases.enumerated() {
            let event = try deliverMouse(
                try mouse(.leftMouseDown, at: p2, flags: flags, time: now + 2 + Double(index) * 0.01))
            try expectPointer(
                event, "modifier \(flags.rawValue)", phase: phaseDown, button: buttonPrimary, clicks: 1,
                modifiers: bits, x: 200.5, y: 30.25)
        }

        // 4) button·drag·hover move.
        let drag = try deliverMouse(try mouse(.leftMouseDragged, at: windowPoint(50, 60), clicks: 1, time: now + 3))
        try expectPointer(drag, "drag", phase: phaseMove, button: buttonPrimary, clicks: 0, modifiers: 0, x: 50, y: 60)
        let hover = try deliverMouse(try mouse(.mouseMoved, at: windowPoint(70, 80), clicks: 0, time: now + 3.1))
        try expectPointer(
            hover, "hover", phase: phaseMove, button: buttonNone, clicks: 0, modifiers: 0, x: 70, y: 80)
        let right = try deliverMouse(try mouse(.rightMouseDown, at: p1, time: now + 3.2))
        try expectPointer(
            right, "right", phase: phaseDown, button: buttonSecondary, clicks: 1, modifiers: 0, x: 10, y: 20)
        let rightUp = try deliverMouse(try mouse(.rightMouseUp, at: p1, time: now + 3.3))
        try expectPointer(
            rightUp, "right up", phase: phaseUp, button: buttonSecondary, clicks: 1, modifiers: 0, x: 10, y: 20)
        // other mouse button 번호는 CGEvent로 만듭니다(NSEvent.mouseEvent는 번호를 지정할 수 없음).
        for (cgButton, expected) in [(2, buttonMiddle), (3, buttonBack), (4, buttonForward)] {
            let event = try otherMouseEvent(button: cgButton, window: window, at: windowPoint(90, 100))
            let converted = try deliverMouse(event)
            try expectPointer(
                converted, "other \(cgButton)", phase: phaseDown, button: expected, clicks: 1, modifiers: 0,
                x: 90, y: 100)
        }

        // 5) scroll: trackpad(precise, began·momentum)과 일반 wheel(line). AppKit content 방향 delta를 반전합니다.
        let scrollPoint = windowPoint(40, 30)
        let began = try deliverMouse(
            try scrollEvent(window: window, at: scrollPoint, pixel: true, vertical: -5, horizontal: 3, phase: 1))
        try expectWheel(began, "trackpad began", phase: wheelBegan, unit: unitPoint, dx: -3, dy: 5)
        let momentum = try deliverMouse(
            try scrollEvent(window: window, at: scrollPoint, pixel: true, vertical: -2, horizontal: 0, momentum: 2))
        try expectWheel(momentum, "trackpad momentum", phase: wheelMomentum, unit: unitPoint, dx: 0, dy: 2)
        let line = try deliverMouse(
            try scrollEvent(window: window, at: scrollPoint, pixel: false, vertical: 2, horizontal: 0))
        try expectWheel(line, "wheel line", phase: wheelDiscrete, unit: unitLine, dx: 0, dy: -2)

        // 6) keyboard: first responder(캔버스) 경유. Carbon kVK 상수 → HID usage.
        try check(window.makeFirstResponder(canvas), "캔버스 first responder 실패")
        func key(
            _ type: NSEvent.EventType, _ code: Int, flags: UInt = 0, repeating: Bool = false, time: TimeInterval
        ) throws -> InputBytes {
            guard
                let event = NSEvent.keyEvent(
                    with: type, location: .zero, modifierFlags: NSEvent.ModifierFlags(rawValue: flags),
                    timestamp: time, windowNumber: window.windowNumber, context: nil, characters: "",
                    charactersIgnoringModifiers: "", isARepeat: repeating, keyCode: UInt16(code)
                )
            else { throw CheckFailure(message: "key 이벤트 생성 실패 \(code)") }
            window.sendEvent(event)
            return try pollOne("key \(type) \(code)")
        }
        func expectKey(
            _ event: InputBytes, _ label: String, state: UInt8, usage: UInt16, repeating: UInt8 = 0,
            modifiers: UInt8 = 0
        ) throws {
            try check(event.kind == kindKey && event.bytes.count == 15, "\(label): key 레이아웃 아님")
            try check(event.u8(10) == state, "\(label): state \(event.u8(10)) != \(state)")
            try check(event.u8(11) == repeating, "\(label): repeat \(event.u8(11))")
            try check(event.u8(12) == modifiers, "\(label): modifiers \(event.u8(12)) != \(modifiers)")
            try check(event.u16(13) == usage, "\(label): usage \(event.u16(13)) != \(usage)")
        }
        let command = NSEvent.ModifierFlags.command.rawValue
        try expectKey(
            try key(.keyDown, kVK_ANSI_A, flags: command, time: now + 4), "cmd+A down", state: keyDown, usage: 0x04,
            modifiers: modMeta)
        try expectKey(
            try key(.keyDown, kVK_ANSI_A, repeating: true, time: now + 4.1), "A repeat", state: keyDown, usage: 0x04,
            repeating: 1)
        try expectKey(try key(.keyUp, kVK_ANSI_A, time: now + 4.2), "A up", state: keyUp, usage: 0x04)
        let usages: [(Int, UInt16)] = [
            (kVK_ANSI_Z, 0x1D), (kVK_ANSI_1, 0x1E), (kVK_ANSI_0, 0x27), (kVK_Return, 0x28), (kVK_Escape, 0x29),
            (kVK_Delete, 0x2A), (kVK_Tab, 0x2B), (kVK_Space, 0x2C), (kVK_LeftArrow, 0x50), (kVK_RightArrow, 0x4F),
            (kVK_DownArrow, 0x51), (kVK_UpArrow, 0x52), (kVK_F1, 0x3A), (kVK_F12, 0x45), (kVK_ForwardDelete, 0x4C),
            (kVK_Home, 0x4A), (kVK_End, 0x4D), (kVK_PageUp, 0x4B), (kVK_PageDown, 0x4E), (kVK_ANSI_Keypad0, 0x62),
            (kVK_ANSI_KeypadEnter, 0x58), (kVK_ISO_Section, 0x64), (kVK_JIS_Kana, 0x90), (kVK_JIS_Eisu, 0x91),
        ]
        for (index, (code, usage)) in usages.enumerated() {
            try expectKey(
                try key(.keyDown, code, time: now + 5 + Double(index) * 0.01), "kVK \(code)", state: keyDown,
                usage: usage)
        }
        // flagsChanged: 왼쪽 Shift down → 오른쪽 down → 오른쪽 up(왼쪽 유지) → 왼쪽 up.
        let shift = NSEvent.ModifierFlags.shift.rawValue
        try expectKey(
            try key(.flagsChanged, kVK_Shift, flags: shift | 0x2, time: now + 6), "LShift down", state: keyDown,
            usage: 0xE1, modifiers: modShift)
        try expectKey(
            try key(.flagsChanged, kVK_RightShift, flags: shift | 0x2 | 0x4, time: now + 6.1), "RShift down",
            state: keyDown, usage: 0xE5, modifiers: modShift)
        try expectKey(
            try key(.flagsChanged, kVK_RightShift, flags: shift | 0x2, time: now + 6.2), "RShift up", state: keyUp,
            usage: 0xE5, modifiers: modShift)
        try expectKey(
            try key(.flagsChanged, kVK_Shift, flags: 0, time: now + 6.3), "LShift up", state: keyUp, usage: 0xE1)

        // 7) 공통 집합 밖 이벤트(mouseEntered)는 대기열에 넣지 않습니다.
        guard
            let entered = NSEvent.enterExitEvent(
                with: .mouseEntered, location: p1, modifierFlags: [], timestamp: now + 7,
                windowNumber: window.windowNumber, context: nil, eventNumber: 0, trackingNumber: 0, userData: nil)
        else { throw CheckFailure(message: "mouseEntered 생성 실패") }
        let ignored = canvas.submitInput(entered)
        try check(ignored.status == STELLAR_OK && ignored.queued == 0, "mouseEntered가 대기열에 들어감")
        try check(stellarPoll(handle) == nil, "mouseEntered 후 대기열이 비어 있지 않음")

        // 8) coalescing(XII.3.4): down → 10,000 drag → up을 꺼내지 않고 연속 제출하면
        //    대기열에는 down, 최신 drag 1개, up만 남습니다.
        let dragCount = 10_000
        guard let content = window.contentView else { throw CheckFailure(message: "contentView 없음") }
        func submitDirect(_ event: NSEvent) throws {
            let pointInSuperview = content.superview?.convert(event.locationInWindow, from: nil)
                ?? event.locationInWindow
            guard let hit = content.hitTest(pointInSuperview), hit === canvas else {
                throw CheckFailure(message: "coalescing: hit test가 캔버스가 아님")
            }
            switch event.type {
            case .leftMouseDown: hit.mouseDown(with: event)
            case .leftMouseDragged: hit.mouseDragged(with: event)
            case .leftMouseUp: hit.mouseUp(with: event)
            default: throw CheckFailure(message: "coalescing: 지원하지 않는 이벤트 \(event.type)")
            }
        }
        let base = now + 8
        // divider 근처(캔버스 왼쪽 가장자리)는 split view가 hit test를 가져가므로 안쪽 좌표를 씁니다.
        try submitDirect(try mouse(.leftMouseDown, at: windowPoint(20, 21), time: base))
        for index in 0..<dragCount {
            let viewX = 20 + CGFloat(index % 300)
            try submitDirect(
                try mouse(.leftMouseDragged, at: windowPoint(viewX, 22), clicks: 1, time: base + Double(index) * 1e-5))
        }
        try submitDirect(try mouse(.leftMouseUp, at: windowPoint(25, 26), time: base + 1))
        var drained: [InputBytes] = []
        while let bytes = stellarPoll(handle) {
            drained.append(InputBytes(bytes: bytes))
        }
        try check(drained.count == 3, "coalescing: \(dragCount) drag 후 이벤트 \(drained.count)개(기대 3)")
        let lastX = Float(20 + (dragCount - 1) % 300)
        try expectPointer(
            drained[0], "coalescing down", phase: phaseDown, button: buttonPrimary, clicks: 1, modifiers: 0, x: 20, y: 21)
        try expectPointer(
            drained[1], "coalescing drag", phase: phaseMove, button: buttonPrimary, clicks: 0, modifiers: 0, x: lastX,
            y: 22)
        try expectPointer(
            drained[2], "coalescing up", phase: phaseUp, button: buttonPrimary, clicks: 1, modifiers: 0, x: 25, y: 26)
        count += drained.count

        return "\(count) events (coordinate flip, modifiers, click count 1-3, buttons, scroll, keys, flagsChanged, "
            + "\(dragCount) drag→1)"
    }

    private static func expectWheel(
        _ event: InputBytes, _ label: String, phase: UInt8, unit: UInt8, dx: Float, dy: Float
    ) throws {
        try check(event.kind == kindWheel && event.bytes.count == 29, "\(label): wheel 레이아웃 아님")
        try check(event.u8(10) == phase, "\(label): phase \(event.u8(10)) != \(phase)")
        try check(event.u8(11) == unit, "\(label): unit \(event.u8(11)) != \(unit)")
        try check(event.f32(13) == 40 && event.f32(17) == 30, "\(label): 위치 (\(event.f32(13)), \(event.f32(17)))")
        try check(
            event.f32(21) == dx && event.f32(25) == dy,
            "\(label): delta (\(event.f32(21)), \(event.f32(25))) != (\(dx), \(dy))")
    }

    /// CGEvent 원점(주 화면 좌상단)에서 창 좌표 point에 해당하는 위치.
    private static func quartzPoint(window: NSWindow, at windowPoint: NSPoint) throws -> CGPoint {
        guard let primary = NSScreen.screens.first else {
            throw CheckFailure(message: "화면 정보 없음")
        }
        let screen = window.convertPoint(toScreen: windowPoint)
        return CGPoint(x: screen.x, y: primary.frame.height - screen.y)
    }

    private static func otherMouseEvent(button: Int, window: NSWindow, at windowPoint: NSPoint) throws -> NSEvent {
        let position = try quartzPoint(window: window, at: windowPoint)
        guard
            let mouseButton = CGMouseButton(rawValue: UInt32(button)),
            let cg = CGEvent(
                mouseEventSource: nil, mouseType: .otherMouseDown, mouseCursorPosition: position,
                mouseButton: mouseButton)
        else { throw CheckFailure(message: "other mouse 이벤트 생성 실패 \(button)") }
        // 실제 키보드 상태(예: Caps Lock)가 섞이지 않도록 modifier를 비웁니다.
        cg.flags = []
        guard let event = NSEvent(cgEvent: cg) else {
            throw CheckFailure(message: "other mouse NSEvent 변환 실패 \(button)")
        }
        return event
    }

    /// scroll 이벤트(창 없음, 화면 좌표). phase·momentum은 CGEvent 값(began=1, momentum continue=2)입니다.
    private static func scrollEvent(
        window: NSWindow, at windowPoint: NSPoint, pixel: Bool, vertical: Int32, horizontal: Int32,
        phase: Int64 = 0, momentum: Int64 = 0
    ) throws -> NSEvent {
        guard
            let cg = CGEvent(
                scrollWheelEvent2Source: nil, units: pixel ? .pixel : .line, wheelCount: 2,
                wheel1: vertical, wheel2: horizontal, wheel3: 0)
        else { throw CheckFailure(message: "scroll 이벤트 생성 실패") }
        cg.location = try quartzPoint(window: window, at: windowPoint)
        cg.flags = []
        cg.setIntegerValueField(.scrollWheelEventScrollPhase, value: phase)
        cg.setIntegerValueField(.scrollWheelEventMomentumPhase, value: momentum)
        guard let event = NSEvent(cgEvent: cg) else { throw CheckFailure(message: "scroll NSEvent 변환 실패") }
        return event
    }

    /// 입력 대기열에서 다음 InputEvent 바이트를 읽습니다(비었으면 nil).
    private static func stellarPoll(_ handle: UInt64) -> [UInt8]? {
        var needed = 0
        guard stellar_runtime_poll_input(handle, nil, 0, &needed) == STELLAR_OK else { return nil }
        var buffer = [UInt8](repeating: 0, count: needed)
        var written = 0
        let status = buffer.withUnsafeMutableBufferPointer { raw in
            stellar_runtime_poll_input(handle, raw.baseAddress, needed, &written)
        }
        guard status == STELLAR_OK else { return nil }
        return Array(buffer[0..<written])
    }
}
