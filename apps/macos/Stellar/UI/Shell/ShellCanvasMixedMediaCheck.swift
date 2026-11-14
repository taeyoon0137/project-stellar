import AppKit

/// 네 종류의 레이어가 겹친 QA 문서를 한 창과 하나의 painter에서 검증합니다.
@MainActor
enum ShellCanvasMixedMediaCheck {
    static func run(project: ShellProjectWindow, directory: URL, text: UUID, reopen: () -> ShellProjectWindow?) -> Bool {
        guard let session = project.windowController.windowSession, let pump = project.dataSource.session?.pump,
              let view = session.viewInfo else { return false }
        let canvas = project.canvasHost.canvasView
        let handle = pump.runtimeHandle, windowID = session.session
        func wait() -> Bool { pump.waitForPendingRequests(timeout: 15) }
        func until(_ condition: () -> Bool) -> Bool {
            let deadline = Date().addingTimeInterval(8)
            while !condition() && Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.02)) }
            return condition()
        }
        func fail(_ detail: String) -> Bool {
            FileHandle.standardError.write(Data("mixed canvas: \(detail)\n".utf8))
            return false
        }
        func point(_ owner: ShellProjectWindow, _ x: CGFloat, _ y: CGFloat) -> CGPoint {
            let target = owner.canvasHost.canvasView
            let camera = owner.windowController.windowSession!.viewInfo!
            return CGPoint(x: target.bounds.midX + (x - camera.center_x) * camera.zoom,
                           y: target.bounds.midY + (y - camera.center_y) * camera.zoom)
        }
        // 실제 프로젝트 안에 크기와 채우기가 있는 독립 원본과 인스턴스를 만듭니다.
        var created = false
        _ = pump.createComponent(session: windowID, name: "Mixed") { if case .componentCreated = $0 { created = true } }
        guard wait(), created else { return fail("create source") }
        var entry: LibraryComponent?
        DispatchQueue.global(qos: .userInitiated).async {
            func read() -> LibraryComponent? {
                if case .loaded(let rows, _) = CoreRuntime.componentLibrary(handle: handle, session: windowID) {
                    return rows.first { $0.name == "Mixed" }
                }
                return nil
            }
            let result: LibraryComponent? = {
                guard let blank = read(), CoreRuntime.resizeComponent(handle: handle, session: windowID, entry: blank, width: 20, height: 20) == nil,
                      let sized = read(), CoreRuntime.editComponent(handle: handle, session: windowID, entry: sized,
                          change: .background(.init(space: .srgb, red: 0, green: 1, blue: 0, alpha: 1))) == nil else { return nil }
                return read()
            }()
            DispatchQueue.main.async { entry = result }
        }
        guard until({ entry != nil }), let entry, let revision = session.info?.revision else { return fail("size or color source") }
        var placed = false
        _ = pump.placeInstance(session: windowID, view: UUID(uuid: view.active_view), revision: revision,
                               entry: entry, x: 200, y: 37) { if case .windowViewChanged = $0 { placed = true } }
        guard wait(), placed else { return fail("place instance") }
        project.reloadComponentCanvas()
        guard wait(), canvas.frameHit(at: point(project, 205, 45))?.isInstance == true else { return fail("instance projection") }
        guard let parent = canvas.frameHit(at: point(project, 154, 32), onlyFrames: true) else { return fail("hit Header") }
        var rectangle = false
        _ = pump.insertFrame(session: windowID, parent: parent, rect: CGRect(x: 150, y: 32, width: 18, height: 20),
                             rectangle: true) { if case .windowViewChanged = $0 { rectangle = true } }
        guard wait(), rectangle else { return fail("rectangle insertion") }
        project.refreshSessionCommands()
        canvas.refreshFrames()
        let file = directory.appendingPathComponent("Mixed.png")
        do { try ShellImagePlacementCheck.sample.write(to: file) } catch { return fail("write PNG") }
        project.choosePNG = { _, _, complete in complete(file) }
        var notice: String?
        project.alertPresenter = { _, message, _ in notice = message }
        guard project.router.dispatch(.selectImageTool), until({ project.pickedPNG != nil || notice != nil }),
              project.pickedPNG != nil else { return fail("choose PNG: " + (notice ?? "not armed")) }
        guard ShellCanvasMoveCheck.mouse(canvas, .leftMouseDown, at: point(project, 92, 30)), wait(),
              let image = canvas.frameHit(at: point(project, 104, 41)), image.isImage
        else { return fail("place PNG over glyphs: " + (notice ?? "missing image hit")) }
        guard project.selectNodes([]), wait() else { return fail("clear selection") }

        func verify(_ owner: ShellProjectWindow) -> Bool {
            let target = owner.canvasHost.canvasView
            guard let window = owner.window,
                  target.frameHit(at: point(owner, 104, 41))?.id == image.id,
                  target.frameHit(at: point(owner, 125, 41))?.id == text,
                  target.frameHit(at: point(owner, 156, 41))?.isRectangle == true,
                  target.frameHit(at: point(owner, 205, 41))?.isInstance == true,
                  target.frameHit(at: point(owner, 104, 41), onlyFrames: true)?.id == parent.id
            else { return false }
            let start = point(owner, 88, 26), end = point(owner, 226, 61)
            let dirty = CGRect(x: start.x.rounded(.down), y: start.y.rounded(.down),
                               width: (end.x - start.x + 2).rounded(.up), height: (end.y - start.y + 2).rounded(.up))
            guard let full = ShellVisibleFramesCheck.bitmap(target, in: window, dirtyRect: target.bounds),
                  let part = ShellVisibleFramesCheck.bitmap(target, in: window, dirtyRect: dirty) else { return false }
            func pixel(_ rep: NSBitmapImageRep, _ p: CGPoint) -> [Int] {
                let x = Int(p.x * CGFloat(rep.pixelsWide) / target.bounds.width)
                let y = Int(p.y * CGFloat(rep.pixelsHigh) / target.bounds.height)
                var rgba = [Int](repeating: 0, count: rep.samplesPerPixel)
                rep.getPixel(&rgba, atX: x, y: y)
                return rgba
            }
            let red = pixel(full, point(owner, 104, 34)), blue = pixel(full, point(owner, 104, 42))
            let black = pixel(full, point(owner, 158, 41)), green = pixel(full, point(owner, 207, 45))
            guard red[0] > 170, red[2] < 80, blue[0] < 80, blue[2] > 170, black.prefix(3).allSatisfy({ $0 < 8 }),
                  green[0] < 8, green[1] > 245, green[2] < 8,
                  pixel(part, point(owner, 240, 41)).last == 0 else { return false }
            var textInk = 0
            for y in 29..<58 {
                for x in 90..<223 {
                    let p = point(owner, CGFloat(x), CGFloat(y))
                    let fullPixel = pixel(full, p)
                    guard fullPixel == pixel(part, p) else { return false }
                    if (118..<148).contains(x), fullPixel.prefix(3).allSatisfy({ $0 < 65 }) { textInk += 1 }
                }
            }
            return textInk > 20
        }
        guard until({ verify(project) }) else { return fail("mixed pixels, order, picking or partial redraw") }
        guard project.performSessionCommand(.saveNow), wait() else { return fail("save") }
        do { try FileManager.default.removeItem(at: file) } catch { return fail("delete PNG source") }
        guard let next = reopen(), until({ next.gitSource.trusted && verify(next) }) else { return fail("reopen") }
        print("mixed canvas ok: same scene glyph ink, rectangle, component and PNG; frontmost hits, parent hit, dirty pixels equal full render, source-free reopen")
        return true
    }
}
