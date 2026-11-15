import AppKit
import UniformTypeIdentifiers

struct PickedPNG {
    let url: URL
    let digest: Data
    let target: StellarSelectionTarget
}

/// Image 도구(tools_menus 표 8): 캔버스를 누르면 PNG 선택 패널을 열고, 고른 이미지를 누른 위치에 배치합니다.
/// 취소하면 이전 도구로 돌아갑니다. 배치·표시는 프로젝트 신뢰와 무관합니다(D1).
extension ShellProjectWindow {
    func imageToolChanged(_ tool: CanvasTool) {
        // 진행 중인 선택 패널의 늦은 결과를 버립니다.
        imagePickGeneration &+= 1
        if tool != .image { imageReturnTool = tool }
    }

    /// 누른 점의 편집 가능한 Frame을 대상으로 선택 패널을 엽니다. 이벤트를 소비하면 true입니다.
    func placeImage(at point: CGPoint, canvas: CanvasView) -> Bool {
        guard let owner = window, let session = windowController.windowSession, !session.isClosing, !session.isClosed,
              let target = session.selectionTarget, let hit = canvas.frameHit(at: point, onlyFrames: true),
              UUID(uuid: target.view) == hit.viewID, target.revision == hit.revision else { return true }
        let transform = hit.localToWorld.concatenating(hit.worldToView)
        let det = transform.a * transform.d - transform.b * transform.c
        guard det.isFinite, det != 0 else { return true }
        let position = point.applying(transform.inverted())
        imagePickGeneration &+= 1
        let generation = imagePickGeneration
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.png]
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.prompt = String(localized: "Choose Image")
        choosePNG(panel, owner) { [weak self] url in
            guard let self, self.imagePickGeneration == generation else { return }
            guard let url else { return self.leaveImageTool() }
            self.prepareImage(url, target: target, parent: hit.id, at: position, generation: generation)
        }
        return true
    }

    private func prepareImage(_ url: URL, target: StellarSelectionTarget, parent: UUID, at position: CGPoint, generation: UInt64) {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let digest = CanvasPNG.picked(url)
            DispatchQueue.main.async { [weak self] in
                guard let self, self.imagePickGeneration == generation, self.toolsModel.snapshot.activeTool == .image else { return }
                guard let digest else {
                    if let win = self.window {
                        self.alertPresenter?(String(localized: "Couldn’t open image"),
                                             String(localized: "Choose a valid PNG within 32 MiB and 16 megapixels."), win)
                    }
                    return self.leaveImageTool()
                }
                self.submitImage(PickedPNG(url: url, digest: digest, target: target), parent: parent, at: position)
            }
        }
    }

    private func submitImage(_ chosen: PickedPNG, parent: UUID, at position: CGPoint) {
        guard let project = dataSource.session, let session = windowController.windowSession,
              !session.isClosing, !session.isClosed else { return }
        // 패널이 열린 동안 문서·선택이 바뀌었으면 코어가 대상 확인에서 거절합니다.
        let accepted = session.track({ done in
            project.pump.placePNG(chosen, parent: parent, at: position, completion: done)
        }, completion: { [weak self] event in
            guard let self else { return }
            if case .requestFailed(_, _, let message) = event, let win = self.window {
                self.alertPresenter?(String(localized: "Couldn’t place image"), message, win)
            }
            _ = self.router.dispatch(.selectMoveTool)
            self.refreshSessionCommands()
        })
        if !accepted { leaveImageTool() }
    }

    private func leaveImageTool() {
        _ = router.dispatch(imageReturnTool.command)
    }
}
