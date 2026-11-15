import Foundation

// 사이드바 머리 줄 Page › View 팝업(UX.3.5)의 View 전환. 탭·active View는 코어 창 view state이고 문서 history에는
// 들어가지 않습니다.
extension ShellProjectWindow {
    func connectPagePath() {
        sidebarSource.openView = { [weak self] view in self?.openView(view) }
    }

    /// 고른 View를 이 창의 active View로 엽니다. 처음 연 View는 기본 카메라라 그 root Frame에 맞춥니다.
    func openView(_ view: UUID) {
        guard let session = windowController.windowSession, !session.isClosed, !session.isClosing,
              let pump = dataSource.session?.pump, finishInputBeforeClosing() else { return }
        _ = canvasHost.canvasView.cancelFrameGesture?()
        let tabs = session.viewInfo?.tab_count ?? 0
        session.track({ pump.openView(session: session.session, view: view, completion: $0) }) { [weak self, weak session] event in
            guard let self, let session, self.windowController.windowSession === session else { return }
            self.refreshSessionCommands()
            if case .windowViewChanged = event, (session.viewInfo?.tab_count ?? 0) > tabs {
                _ = self.performSessionCommand(.zoomToFit)
            }
        }
    }
}
