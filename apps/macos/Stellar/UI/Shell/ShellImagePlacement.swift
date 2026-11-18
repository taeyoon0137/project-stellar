import AppKit
import UniformTypeIdentifiers

struct PickedPNG {
    let url: URL
    let digest: Data
    let target: StellarSelectionTarget
}

/// Image 도구: 고르면 PNG를 먼저 선택하고, 캔버스 클릭은 원본 크기·드래그는 그린 크기로 배치합니다.
/// 취소하면 이전 도구로 돌아갑니다. 배치·표시는 프로젝트 신뢰와 무관합니다(D1).
extension ShellProjectWindow {
    func imageToolChanged(_ tool: CanvasTool) {
        // 진행 중인 선택 패널의 늦은 결과를 버립니다.
        imagePickGeneration &+= 1
        pendingImage = nil
        if tool != .image { imageReturnTool = tool; return }
        chooseImageFile()
    }

    private func chooseImageFile() {
        guard let owner = window else { return }
        let generation = imagePickGeneration
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.png]
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.prompt = String(localized: "Choose Image")
        choosePNG(panel, owner) { [weak self] url in
            guard let self, self.imagePickGeneration == generation else { return }
            guard let url else { return self.leaveImageTool() }
            self.prepareImage(url, generation: generation)
        }
    }

    /// 누른 Frame의 local 공간에서 클릭 또는 드래그로 이미지를 배치합니다.
    func placeImage(from start: CGPoint, to end: CGPoint, canvas: CanvasView) -> Bool {
        guard let chosen = pendingImage, let session = windowController.windowSession, !session.isClosing, !session.isClosed,
              let target = session.selectionTarget, let hit = canvas.frameHit(at: start, onlyFrames: true),
              UUID(uuid: target.view) == hit.viewID, target.revision == hit.revision else { return true }
        let transform = hit.localToWorld.concatenating(hit.worldToView)
        let det = transform.a * transform.d - transform.b * transform.c
        guard det.isFinite, det != 0 else { return true }
        let inverse = transform.inverted()
        let a = start.applying(inverse), b = end.applying(inverse)
        guard a.x.isFinite, a.y.isFinite, b.x.isFinite, b.y.isFinite else { return true }
        let dragged = hypot(end.x - start.x, end.y - start.y) >= 3
        let position = dragged ? CGPoint(x: min(a.x, b.x), y: min(a.y, b.y)) : a
        let size = dragged ? CGSize(width: abs(b.x - a.x), height: abs(b.y - a.y)) : nil
        if let size, size.width <= 0 || size.height <= 0 { return true }
        submitImage(PickedPNG(url: chosen.url, digest: chosen.digest, target: target), parent: hit.id, at: position,
                    size: size, returnToMove: true)
        return true
    }

    private func prepareImage(_ url: URL, generation: UInt64) {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let digest = CanvasPNG.picked(url)
            DispatchQueue.main.async { [weak self] in
                guard let self, self.imagePickGeneration == generation, self.toolsModel.snapshot.activeTool == .image else { return }
                guard let digest else {
                    // 받지 않는 파일은 막다른 alert 대신 도구 막대 위 한 줄 안내로 알립니다(UX 명세 3.2·3.5).
                    self.windowController.toolbarController.hintView.show(String(
                        localized: "Couldn’t open the image. Choose a PNG within 32 MiB and 16 megapixels.",
                        comment: "Canvas hint when the picked image file isn't accepted"
                    ))
                    return self.leaveImageTool()
                }
                self.pendingImage = (url, digest)
            }
        }
    }

    func connectImageImport() {
        let canvas = canvasHost.canvasView
        canvas.registerForDraggedTypes([.fileURL])
        canvas.imageDropHandler = { [weak self, weak canvas] url, point in
            guard let self, let canvas else { return false }
            return self.importImage(url, at: point, canvas: canvas)
        }
    }

    /// Finder 끌어 놓기와 파일 URL 붙여넣기가 Image 도구와 같은 PNG import를 사용합니다.
    func importImage(_ url: URL, at point: CGPoint, canvas: CanvasView) -> Bool {
        guard let session = windowController.windowSession, !session.isClosing, !session.isClosed,
              let target = session.selectionTarget, let hit = canvas.frameHit(at: point, onlyFrames: true),
              UUID(uuid: target.view) == hit.viewID, target.revision == hit.revision else { return false }
        let transform = hit.localToWorld.concatenating(hit.worldToView)
        let det = transform.a * transform.d - transform.b * transform.c
        guard det.isFinite, det != 0 else { return false }
        let position = point.applying(transform.inverted())
        guard position.x.isFinite, position.y.isFinite else { return false }
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let digest = CanvasPNG.picked(url)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                guard let digest else {
                    self.windowController.toolbarController.hintView.show(String(
                        localized: "Couldn’t open the image. Choose a PNG within 32 MiB and 16 megapixels.",
                        comment: "Canvas hint when a dropped or pasted PNG isn't accepted"
                    ))
                    return
                }
                self.submitImage(PickedPNG(url: url, digest: digest, target: target), parent: hit.id, at: position,
                                 size: nil, returnToMove: false)
            }
        }
        return true
    }

    func pasteImageFile() -> Bool {
        let urls = subtreePasteboard.readObjects(forClasses: [NSURL.self],
            options: [.urlReadingFileURLsOnly: true]) as? [URL]
        guard let url = urls?.first, url.pathExtension.lowercased() == "png" else { return false }
        let canvas = canvasHost.canvasView
        return importImage(url, at: CGPoint(x: canvas.bounds.midX, y: canvas.bounds.midY), canvas: canvas)
    }

    var hasImageFileOnPasteboard: Bool {
        let urls = subtreePasteboard.readObjects(forClasses: [NSURL.self],
            options: [.urlReadingFileURLsOnly: true]) as? [URL]
        return urls?.first?.pathExtension.lowercased() == "png"
    }

    private func submitImage(_ chosen: PickedPNG, parent: UUID, at position: CGPoint, size: CGSize?, returnToMove: Bool) {
        guard let project = dataSource.session, let session = windowController.windowSession,
              !session.isClosing, !session.isClosed else { return }
        // 패널이 열린 동안 문서·선택이 바뀌었으면 코어가 대상 확인에서 거절합니다.
        let accepted = session.track({ done in
            project.pump.placePNG(chosen, parent: parent, at: position, size: size, completion: done)
        }, completion: { [weak self] event in
            guard let self else { return }
            if case .requestFailed(_, _, let message) = event {
                self.windowController.toolbarController.hintView.show(String(
                    localized: "Couldn’t place the image: \(message)",
                    comment: "Canvas hint when placing an image fails; the argument is the reason"
                ))
            }
            if returnToMove { _ = self.router.dispatch(.selectMoveTool) }
            self.refreshSessionCommands()
        })
        if !accepted && returnToMove { leaveImageTool() }
    }

    private func leaveImageTool() {
        _ = router.dispatch(imageReturnTool.command)
    }
}
