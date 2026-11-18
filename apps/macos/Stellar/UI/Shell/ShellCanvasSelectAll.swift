import Foundation

/// Edit ▸ Select All(⌘A)을 활성 View의 코어 선택 요청으로 연결합니다.
extension ShellProjectWindow {
    @discardableResult
    func selectAllLayers() -> Bool {
        guard !router.isTextEditing, let session = windowController.windowSession, !session.isClosing, !session.isClosed,
              let target = session.selectionTarget, target.session == session.session,
              let pump = dataSource.session?.pump else { return false }
        _ = canvasHost.canvasView.cancelFrameGesture?()
        return session.track({ pump.editSelection(target: target, action: UInt32(STELLAR_SELECTION_SELECT_ALL),
                                                  ids: nil, completion: $0) }) { [weak self, weak session] _ in
            guard let self, let session, self.windowController.windowSession === session else { return }
            self.refreshSessionCommands()
        }
    }
}
