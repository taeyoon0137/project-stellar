import Foundation

// 레이어 표시·잠금(UX.3.4): 사이드바 행 토글과 메뉴 막대·문맥 메뉴의 Hide·Lock을 코어 `SetProperty`
// (`node.visible`·`node.locked`)에 연결합니다. 값 해석과 잠금 규칙(D8)은 코어가 정하고, 셸은 표시한 행으로 미리
// 비활성 여부와 메뉴 제목만 정합니다.
extension ShellProjectWindow {
    func connectLayerFlags() {
        sidebarSource.setFlag = { [weak self] view, revision, ids, flag, value in
            guard let self, let session = self.windowController.windowSession, !session.isClosed, !session.isClosing,
                  let pump = self.dataSource.session?.pump else { return }
            _ = self.canvasHost.canvasView.cancelFrameGesture?()
            session.track({ pump.setLayerFlag(session: session.session, view: view, revision: revision, nodes: ids,
                                              flag: flag, value: value, completion: $0) }) { [weak self, weak session] _ in
                guard let self, let session, self.windowController.windowSession === session else { return }
                self.refreshSessionCommands()
            }
        }
        sidebarSource.selectionStateDidChange = { [weak self] in self?.refreshLayerFlagCommands() }
    }

    /// 선택이 있으면 Hide·Lock을 켭니다. 사이드바가 선택 행을 모두 읽었으면 잠금 규칙과 제목을 그 값으로 정합니다.
    func refreshLayerFlagCommands() {
        guard let target = windowController.windowSession?.selectionTarget, target.selection_count > 0 else {
            toolsModel.setSelectionState(commands: [], hidden: false, locked: false)
            return
        }
        let flags = sidebarSource.selectionFlags
        var commands: Set<ShellCommand> = []
        if flags?.canHide != false { commands.insert(.toggleVisibility) }
        if flags?.canLock != false { commands.insert(.toggleLock) }
        toolsModel.setSelectionState(commands: commands, hidden: flags?.anyHidden ?? false,
                                     locked: flags?.allLocked ?? false)
    }
}
