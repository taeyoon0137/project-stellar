import Foundation

/// Edit ▸ Select All(⌘A)을 캔버스 선택으로 연결합니다(UX 명세 12.2).
///
/// 활성 View 루트의 자식(레이어 목록의 맨 위 단계) 가운데 보이고 잠기지 않은 노드를 고릅니다. 숨기거나 잠근 노드는
/// 캔버스에서 고를 수 없는 것과 같게 빼므로, 이어지는 Delete·Cut이 보이지 않는 노드를 지우지 않습니다. 목록은 코어
/// 레이어 페이지를 worker에서 끝까지 읽고, 선택은 캔버스·레이어 선택과 같은 경로(`selectNodes`)로 그 창 history에
/// 기록합니다. 읽는 동안 문서·선택이 바뀌면 고르지 않습니다.
extension ShellProjectWindow {
    @discardableResult
    func selectAllLayers() -> Bool {
        guard !router.isTextEditing, let session = windowController.windowSession, !session.isClosing, !session.isClosed,
              let target = session.selectionTarget, target.session == session.session,
              let runtime = dataSource.session?.pump.runtimeHandle else { return false }
        let view = UUID(uuid: target.view)
        Self.selectAllQueue.async { [weak self, weak session] in
            var ids: [UUID] = []
            var after: SidebarStableID?
            repeat {
                guard let page = CoreRuntime.layerPage(handle: runtime, session: target.session, view: view,
                                                      revision: target.revision, parent: nil, after: after),
                      page.historySequence == target.history_sequence else { return }
                ids += page.rows.filter { $0.isVisible && !$0.isLocked }.map(\.id.uuid)
                after = page.hasMore ? page.rows.last?.id : nil
            } while after != nil
            DispatchQueue.main.async { [weak self, weak session] in
                guard let self, let session, self.windowController.windowSession === session, !session.isClosed,
                      !session.isClosing, !self.router.isTextEditing, target.matches(session.selectionTarget),
                      !ids.isEmpty else { return }
                _ = self.canvasHost.canvasView.cancelFrameGesture?()
                self.selectNodes(ids)
            }
        }
        return true
    }

    private static let selectAllQueue = DispatchQueue(label: "xyz.taeyoon.stellar.select-all", qos: .userInitiated)
}
