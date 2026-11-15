import AppKit

// 캔버스 문맥 메뉴(UX.3.2, 명세 5.6). 우클릭·Control-클릭한 노드를 선택한 뒤 메뉴 막대와 같은 `ShellCommand`로
// 그 선택에 동작하는 명령만 보입니다. 선택 변경과 hit 결과는 코어 값이고, 셸은 메뉴를 띄우기만 합니다.
extension ShellProjectWindow {
    func connectCanvasContextMenu() {
        canvasHost.canvasView.contextMenuHandler = { [weak self] event in
            self?.openCanvasContextMenu(for: event) ?? false
        }
    }

    /// 누른 노드가 이미 선택에 있으면 그대로, 아니면 그 노드만 선택합니다. 빈 곳이면 선택을 비웁니다.
    /// 선택 변경은 창 history에 기록하는 코어 요청이므로 결과가 게시된 뒤 메뉴를 만듭니다.
    func openCanvasContextMenu(for event: NSEvent) -> Bool {
        let canvas = canvasHost.canvasView
        guard !router.isTextEditing, let session = windowController.windowSession, !session.isClosed,
              !session.isClosing, let viewport = session.viewInfo, viewport.has_view == 1,
              let info = session.info else { return false }
        _ = canvas.cancelFrameGesture?()
        canvas.window?.makeFirstResponder(canvas)
        let point = CanvasInput.viewPoint(of: event, in: canvas)
        let next: [UUID]?
        if let hit = canvas.frameHit(at: point) {
            guard info.revision == hit.revision, UUID(uuid: viewport.active_view) == hit.viewID else { return true }
            // 다중 선택은 레이어가 같은 게시 시점의 선택 행을 모두 읽었을 때만 압니다. 모르면 그 노드만 고릅니다.
            let selected = (info.selection_count == 1 && canvas.primarySelectionID == hit.id)
                || (session.selectionTarget.flatMap { sidebarSource.selectedIDs(matching: $0) }?.contains(hit.id) ?? false)
            next = selected ? nil : [hit.id]
        } else {
            next = info.selection_count > 0 ? [] : nil
        }
        guard let ids = next else {
            popUpCanvasContextMenu(at: point)
            return true
        }
        let expected = ids.first
        return session.select(ids) { [weak self, weak session] _ in
            guard let self, let session, self.windowController.windowSession === session else { return }
            self.refreshSessionCommands()
            // 완료 event 처리 중에 메뉴 추적을 시작하지 않습니다.
            DispatchQueue.main.async { [weak self, weak session] in
                guard let self, let session, self.windowController.windowSession === session, !session.isClosing,
                      self.canvasHost.canvasView.window?.isVisible == true else { return }
                let current = self.canvasHost.canvasView.primarySelectionID
                guard expected.map({ current == $0 }) ?? (session.info?.selection_count == 0) else { return }
                self.popUpCanvasContextMenu(at: point)
            }
        }
    }

    private func popUpCanvasContextMenu(at point: NSPoint) {
        let canvas = canvasHost.canvasView
        var excluded: Set<ShellCommand> = []
        if !hasSubtreeOnPasteboard { excluded.insert(.paste) }
        let count = windowController.windowSession?.info?.selection_count ?? 0
        if count == 1, canvas.primarySelectionIsRoot {
            // View root는 복사·복제·삭제·숨기기·잠금을 코어가 거부합니다.
            excluded.formUnion([.cut, .copy, .duplicate, .delete, .toggleVisibility, .toggleLock])
        } else if sidebarSource.selectionFlags?.canHide == false {
            // 잠긴 레이어(또는 잠긴 조상 아래)는 복사·복제·삭제를 코어가 거부합니다.
            excluded.formUnion([.cut, .copy, .duplicate, .delete])
        }
        guard let menu = CanvasContextMenu.make(snapshot: toolsModel.snapshot, isTextEditing: router.isTextEditing,
                                                target: router, excluded: excluded) else { return }
        menu.popUp(positioning: nil, at: point, in: canvas)
    }
}
