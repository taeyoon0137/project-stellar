import AppKit

/// 50,000개 저장 Frame이 있는 일반 프로젝트 창의 첫 bitmap과 warm pan을 같은 fixture로 비교합니다.
@MainActor
enum ShellCanvasCullingCheck {
    static func run(path: String) -> Bool {
        _ = NSApplication.shared
        NSApp.setActivationPolicy(.accessory)
        let host = RuntimeHost()
        defer { host.shutdown() }
        let coordinator = ShellAppCoordinator(runtimeHandle: host.handle)
        coordinator.presentsWindows = false
        let autosave = ShellCheckAutosave(prefix: "StellarCullingCheck")
        defer { autosave.removeRecords() }
        coordinator.windowConfiguration = { autosave.configuration }
        let clock = { ProcessInfo.processInfo.systemUptime }
        let start = clock()
        coordinator.openProject(atPath: path, origin: .application)
        guard let pump = coordinator.pump, pump.waitForPendingRequests(timeout: 90),
              let shell = coordinator.projectWindow(forPath: path), let window = shell.window,
              let session = shell.windowController.windowSession else { return false }
        defer { shell.closeSessions(); _ = pump.waitForPendingRequests(timeout: 30) }
        window.contentView?.layoutSubtreeIfNeeded()
        let canvas = shell.canvasHost.canvasView
        let opened = clock() - start
        var info = StellarCanvasInfo()
        guard stellar_window_canvas(host.handle, session.session, Float(canvas.bounds.width), Float(canvas.bounds.height),
                                    &info, nil, 0) == STELLAR_OK, info.count == 50_000, info.skipped == 0 else { return false }
        let rebuildStart = clock()
        let freshScene = CanvasFrameScene()
        freshScene.refresh(handle: host.handle, session: session.session, size: canvas.bounds.size)
        let rebuilt = clock() - rebuildStart
        let width = Int(canvas.bounds.width), height = Int(canvas.bounds.height)
        guard let space = CGColorSpace(name: CGColorSpace.sRGB),
              let cg = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                                 space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
        cg.translateBy(x: 0, y: CGFloat(height))
        cg.scaleBy(x: 1, y: -1)
        func draw(_ rect: CGRect) -> Double {
            let t = clock()
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(cgContext: cg, flipped: true)
            cg.saveGState()
            cg.clip(to: rect)
            canvas.draw(rect)
            cg.restoreGState()
            cg.flush()
            NSGraphicsContext.restoreGraphicsState()
            return (clock() - t) * 1000
        }
        let bounds = canvas.bounds
        let first = draw(bounds)
        func save(_ name: String) -> Bool {
            guard let dir = ProcessInfo.processInfo.environment["STELLAR_CANVAS_PROFILE_IMAGES"] else { return true }
            guard let img = cg.makeImage(),
                  let data = NSBitmapImageRep(cgImage: img).representation(using: .png, properties: [:]) else { return false }
            do {
                try data.write(to: URL(fileURLWithPath: dir).appendingPathComponent(name + ".png"))
                return true
            } catch {
                return false
            }
        }
        guard save("first") else { return false }
        var durations = [Double](), partial = [Double](), panToBitmap = [Double]()
        for i in 0..<32 {
            let step = i.isMultiple(of: 2) ? 3.0 : -3.0
            let panStarted = clock()
            guard session.pan(dx: step, dy: -step), pump.waitForPendingRequests(timeout: 5) else { return false }
            canvas.refreshFrames()
            durations.append(draw(bounds))
            panToBitmap.append((clock() - panStarted) * 1000)
            partial.append(draw(CGRect(x: bounds.midX, y: bounds.midY, width: 32, height: 32)))
        }
        durations.sort()
        partial.sort()
        panToBitmap.sort()
        guard save("last") else { return false }
        print(String(format: "canvas-profile count=50000 viewport=%dx%d open_ms=%.2f project_scene_ms=%.2f first_draw_ms=%.3f warm_pan_draw_p50_ms=%.3f warm_pan_draw_p95_ms=%.3f dirty32_p50_ms=%.3f pan_to_bitmap_p50_ms=%.3f",
                     width, height, opened * 1000, rebuilt * 1000, first, durations[16], durations[30], partial[16], panToBitmap[16]))
        return true
    }
}
