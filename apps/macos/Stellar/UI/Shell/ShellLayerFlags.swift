import Foundation

// 레이어 표시·잠금(UX.3.4): 사이드바 행 토글과 메뉴 막대·문맥 메뉴의 Hide·Lock을 코어 `SetProperty`
// (`node.visible`·`node.locked`)에 연결합니다. 잠금 규칙(D8)·토글 방향·활성은 코어가 정하고(선택 능력 mask와 행 bit),
// 셸은 그 값을 메뉴 활성과 제목에 옮기기만 합니다. 표시·잠금은 문서 편집이라 프로젝트 신뢰와 무관합니다(D1).
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
    }

    /// 표시한 선택의 코어 능력 mask로 Hide·Lock 활성과 제목(Show·Unlock)을 정합니다.
    func refreshLayerFlagCommands() {
        let flags = displayedSelectionFlags
        var commands: Set<ShellCommand> = []
        if flags & UInt32(STELLAR_SELECTION_CAN_HIDE) != 0 { commands.insert(.toggleVisibility) }
        if flags & UInt32(STELLAR_SELECTION_CAN_LOCK) != 0 { commands.insert(.toggleLock) }
        toolsModel.setSelectionState(commands: commands, hidden: flags & UInt32(STELLAR_SELECTION_SHOWS) != 0,
                                     locked: flags & UInt32(STELLAR_SELECTION_UNLOCKS) != 0)
    }
}
