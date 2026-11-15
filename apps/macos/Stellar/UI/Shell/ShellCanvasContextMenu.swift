import AppKit

// 캔버스 문맥 메뉴(UX.3.2, 명세 5.6). 우클릭·Control-클릭으로 고를 선택은 코어가 정하고(누른 노드나 그 조상이 이미
// 선택에 있으면 유지, 아니면 그 노드만), 그 선택이 게시된 뒤 메뉴 막대와 같은 `ShellCommand`로 지금 동작하는 명령만
// 보입니다. 활성은 메뉴 막대와 같은 코어 능력 mask이고, 셸은 누른 위치의 hit와 메뉴 표시만 맡습니다.
extension ShellProjectWindow {
    func connectCanvasContextMenu() {
        canvasHost.canvasView.contextMenuHandler = { [weak self] event in
            self?.openCanvasContextMenu(for: event) ?? false
        }
    }

    /// 누른 노드(빈 곳이면 nil)를 코어 문맥 선택으로 보내고, 선택이 게시된 뒤 메뉴를 엽니다.
    func openCanvasContextMenu(for event: NSEvent) -> Bool {
        let canvas = canvasHost.canvasView
        guard !router.isTextEditing, let session = windowController.windowSession, !session.isClosed,
              !session.isClosing, let viewport = session.viewInfo, viewport.has_view == 1,
              let info = session.info, let target = session.selectionTarget, let window = canvas.window
        else { return false }
        _ = canvas.cancelFrameGesture?()
        // 메뉴의 Copy·Paste는 key 창 명령입니다. 뒤 창을 우클릭하면 그 창을 먼저 key로 만듭니다.
        if !window.isKeyWindow { window.makeKeyAndOrderFront(nil) }
        window.makeFirstResponder(canvas)
        let point = CanvasInput.viewPoint(of: event, in: canvas)
        let hit = canvas.frameHit(at: point)
        if let hit, info.revision != hit.revision || UUID(uuid: viewport.active_view) != hit.viewID { return true }
        return session.contextSelect(target, node: hit?.id) { [weak self, weak session] event in
            guard let self, let session, self.windowController.windowSession === session else { return }
            self.refreshSessionCommands()
            guard case .windowViewChanged = event else { return }
            // 완료 event 처리 중에 메뉴 추적을 시작하지 않습니다.
            DispatchQueue.main.async { [weak self, weak session] in
                guard let self, let session, self.windowController.windowSession === session, !session.isClosing,
                      !self.router.isTextEditing, self.canvasHost.canvasView.window?.isVisible == true else { return }
                self.popUpCanvasContextMenu(at: point)
            }
        }
    }

    private func popUpCanvasContextMenu(at point: NSPoint) {
        let canvas = canvasHost.canvasView
        var excluded: Set<ShellCommand> = []
        // 앱이 비활성이라 이 창이 key가 되지 못했으면 클립보드 명령은 실행되지 않으므로 넣지 않습니다.
        if canvas.window == nil || NSApp.keyWindow !== canvas.window { excluded.formUnion([.cut, .copy, .paste]) }
        guard let menu = CanvasContextMenu.make(snapshot: toolsModel.snapshot, isTextEditing: router.isTextEditing,
                                                target: router, excluded: excluded) else { return }
        menu.popUp(positioning: nil, at: point, in: canvas)
    }
}
