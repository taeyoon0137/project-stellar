import AppKit
import UniformTypeIdentifiers

struct PickedPNG {
    let url: URL
    let digest: Data
    let target: StellarSelectionTarget
}

extension ShellProjectWindow {
    func imageToolChanged(_ tool: CanvasTool) {
        imagePickGeneration &+= 1
        pickedPNG = nil
        guard tool == .image, gitSource.trusted, let owner = window, let target = windowController.windowSession?.selectionTarget else { return }
        let generation = imagePickGeneration
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.png]
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.prompt = String(localized: "Choose Image")
        choosePNG(panel, owner) { [weak self] url in
            guard let self, self.imagePickGeneration == generation else { return }
            guard let url else { _ = self.router.dispatch(.selectMoveTool); return }
            self.prepareImage(url, target: target, generation: generation)
        }
    }

    func prepareImage(_ url: URL, target: StellarSelectionTarget, generation: UInt64) {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let digest = CanvasPNG.picked(url)
            DispatchQueue.main.async { [weak self] in
                guard let self, self.gitSource.trusted, self.imagePickGeneration == generation,
                      self.toolsModel.snapshot.activeTool == .image, target.matches(self.windowController.windowSession?.selectionTarget) else { return }
                if let digest { self.pickedPNG = PickedPNG(url: url, digest: digest, target: target) }
                else {
                    _ = self.router.dispatch(.selectMoveTool)
                    if let win = self.window { self.alertPresenter?(String(localized: "Couldn’t open image"), String(localized: "Choose a valid PNG within 32 MiB and 16 megapixels."), win) }
                }
            }
        }
    }

    func placePickedImage(at point: CGPoint, canvas: CanvasView) -> Bool {
        guard let chosen = pickedPNG, gitSource.trusted, let project = dataSource.session,
              let session = windowController.windowSession, let hit = canvas.frameHit(at: point, onlyFrames: true),
              !session.isClosing, !session.isClosed, chosen.target.matches(session.selectionTarget),
              UUID(uuid: chosen.target.view) == hit.viewID, chosen.target.revision == hit.revision else { return true }
        let transform = hit.localToWorld.concatenating(hit.worldToView)
        let det = transform.a * transform.d - transform.b * transform.c
        guard det.isFinite, det != 0 else { return true }
        let position = point.applying(transform.inverted())
        pickedPNG = nil
        return session.track({ done in
            project.pump.placePNG(chosen, parent: hit.id, at: position, completion: done)
        }, completion: { [weak self] event in
            guard let self else { return }
            if case .requestFailed(_, _, let message) = event, let win = self.window {
                self.alertPresenter?(String(localized: "Couldn’t place image"), message, win)
            }
            _ = self.router.dispatch(.selectMoveTool)
            self.refreshSessionCommands()
        })
    }
}
