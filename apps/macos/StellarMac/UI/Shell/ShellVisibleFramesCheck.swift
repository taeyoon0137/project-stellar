import AppKit

/// Rust에서 저장한 generation을 일반 프로젝트 창으로 열고, 실제 NSView의 bitmap과 picking을 확인합니다.
@MainActor
enum ShellVisibleFramesCheck {
    static func run(_ arguments: [String]) -> Bool {
        guard let flag = arguments.firstIndex(of: "--visible-frames-check"), arguments.indices.contains(flag + 1) else { return false }
        _ = NSApplication.shared
        NSApp.setActivationPolicy(.accessory)
        let host = RuntimeHost()
        defer { host.shutdown() }
        let coordinator = ShellAppCoordinator(runtimeHandle: host.handle)
        coordinator.presentsWindows = false
        let autosave = ShellCheckAutosave(prefix: "StellarVisibleFramesCheck")
        coordinator.windowConfiguration = { autosave.configuration }
        defer { autosave.removeRecords() }
        let root = arguments[flag + 1]
        coordinator.openProject(atPath: root, origin: .application)
        guard let pump = coordinator.pump, pump.waitForPendingRequests(timeout: 10),
              let shell = coordinator.projectWindow(forPath: root), let window = shell.window else { return false }
        defer { shell.closeSessions(); _ = pump.waitForPendingRequests(timeout: 10) }
        let canvas = shell.canvasHost.canvasView
        window.setContentSize(NSSize(width: 1300, height: 800))
        window.contentView?.layoutSubtreeIfNeeded()
        guard canvas.bounds.width > 340, canvas.bounds.height > 200 else { return false }
        let center = CGPoint(x: canvas.bounds.midX, y: canvas.bounds.midY)
        let viewPoint = { (x: CGFloat, y: CGFloat) in CGPoint(x: center.x + x, y: center.y + y) }
        func bitmap() -> NSBitmapImageRep? {
            // 픽셀 참조는 현재 사용자 모니터 ICC의 색역/의도에 영향을 받지 않는 명시적 sRGB destination입니다.
            let scale = window.backingScaleFactor
            let width = Int(ceil(canvas.bounds.width * scale)), height = Int(ceil(canvas.bounds.height * scale))
            guard let space = CGColorSpace(name: CGColorSpace.sRGB),
                  let cg = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                                     space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
            cg.scaleBy(x: scale, y: scale)
            canvas.displayIgnoringOpacity(canvas.bounds, in: NSGraphicsContext(cgContext: cg, flipped: false))
            return cg.makeImage().map(NSBitmapImageRep.init(cgImage:))
        }
        var failures = 0
        func check(_ ok: Bool, _ label: String) {
            if !ok { failures += 1; FileHandle.standardError.write(Data("visible-frames failed: \(label)\n".utf8)) }
        }
        func pixel(_ rep: NSBitmapImageRep, _ x: CGFloat, _ y: CGFloat, _ ref: (CGFloat, CGFloat, CGFloat), _ label: String) {
            let p = viewPoint(x,y)
            let xx = Int((p.x / canvas.bounds.width * CGFloat(rep.pixelsWide)).rounded(.down))
            let yy = Int((p.y / canvas.bounds.height * CGFloat(rep.pixelsHigh)).rounded(.down))
            var samples = [Int](repeating: 0, count: rep.samplesPerPixel)
            rep.getPixel(&samples, atX: xx, y: yy)
            let expected = [ref.0, ref.1, ref.2].map { Int(($0 * 255).rounded()) }
            check(zip(samples.prefix(3), expected).allSatisfy { abs($0 - $1) <= 2 },
                  "\(label) sRGB=\(samples) at (\(xx),\(yy))")
        }
        let redID = UUID(uuidString: "00000000-000b-7000-8000-00000000000b")!
        let blueID = UUID(uuidString: "00000000-000e-7000-8000-00000000000e")!
        guard let first = bitmap() else { return false }
        pixel(first, 6, 8, (1,1,1), "white root")
        pixel(first, 30, 34, (1,0,0), "translated Frame")
        pixel(first, 32, 13, (0,0,0), "inside border")
        pixel(first, 11, 13, (1,1,1), "rounded corner")
        pixel(first, 82, 24, (0,0,1), "nested Frame through Fragment")
        pixel(first, 95, 24, (1,1,1), "ancestor bounds clip")
        pixel(first, 62, 54, (0,1,0), "painter order")
        check(canvas.frameHit(at: viewPoint(82,24))?.id == blueID, "nested hit")
        check(canvas.frameHit(at: viewPoint(95,24))?.id != blueID, "clipped hit")
        check(canvas.frameHit(at: viewPoint(30,34))?.id == redID, "red hit")
        canvas.setFrameDragPreview(id: redID, offset: CGVector(dx: 45, dy: 0))
        guard let moved = bitmap() else { return false }
        pixel(moved, 30, 34, (1,1,1), "preview uncovers root")
        pixel(moved, 127, 24, (0,0,1), "preview moves child and its clip")
        pixel(moved, 62, 54, (0,1,0), "preview keeps sibling painter order")
        canvas.setFrameDragPreview(id: nil, offset: .zero)
        check(shell.performSessionCommand(.zoomIn) && pump.waitForPendingRequests(timeout: 10), "normal zoom command")
        guard let zoom = bitmap() else { return false }
        pixel(zoom, 60, 68, (1,0,0), "zoomed red Frame")
        pixel(zoom, 164, 48, (0,0,1), "zoomed nested Frame")
        check(canvas.frameHit(at: viewPoint(164,48))?.id == blueID, "zoomed hit")
        check(shell.selectNodes([redID]) && pump.waitForPendingRequests(timeout: 10), "selection event")
        guard let selected = bitmap(), let before = zoom.bitmapData, let after = selected.bitmapData else { return false }
        let size = min(zoom.bytesPerRow * zoom.pixelsHigh, selected.bytesPerRow * selected.pixelsHigh)
        check(memcmp(before, after, size) != 0, "selecting produces visible outline")
        check(shell.selectNodes([]) && pump.waitForPendingRequests(timeout: 10), "deselect event")
        guard let cleared = bitmap(), let restored = cleared.bitmapData else { return false }
        check(memcmp(before, restored, size) == 0, "deselect restores exact canvas")
        guard let scroll = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 2,
                                   wheel1: 25, wheel2: -30, wheel3: 0), let event = NSEvent(cgEvent: scroll) else { return false }
        canvas.scrollWheel(with: event)
        check(pump.waitForPendingRequests(timeout: 10), "normal pan event")
        guard let panned = bitmap() else { return false }
        pixel(panned, 164 + event.scrollingDeltaX, 48 + event.scrollingDeltaY, (0,0,1), "panned nested Frame")
        check(canvas.frameHit(at: viewPoint(164 + event.scrollingDeltaX, 48 + event.scrollingDeltaY))?.id == blueID, "panned hit")
        if let path = ProcessInfo.processInfo.environment["STELLAR_TEST_CANVAS_PNG"],
           let data = first.representation(using: .png, properties: [:]) {
            do { try data.write(to: URL(fileURLWithPath: path)) } catch { check(false, "save png: \(error)") }
        }
        if failures == 0 { print("visible-frames ok: persisted 800x600 root, colors, painter order, radius, border, clip, preview, zoom, pan, selection and picking in project NSWindow") }
        return failures == 0
    }
}
