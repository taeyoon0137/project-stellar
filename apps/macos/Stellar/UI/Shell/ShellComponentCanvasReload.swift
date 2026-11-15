import AppKit

extension ShellProjectWindow {
    /// 이 창의 컴포넌트 원본을 다시 읽습니다(신뢰 확인·변경과 복구 뒤). 읽기는 신뢰와 무관하고, 신뢰 상태는 프로젝트 밖
    /// symlink 내용을 읽을지만 정합니다. 코어가 그 프로젝트의 원본 감시(FSEvents 하나)를 시작하고, 이후 원본 변경은
    /// 요청 없는 event로 `componentSourcesDidChange`에 옵니다.
    func reloadComponentCanvas() {
        guard let project = dataSource.session, let session = windowController.windowSession,
              !session.isClosing, !session.isClosed else { return }
        // 원본 읽기는 저장 대기 중인 편집 명령에 포함하지 않습니다.
        _ = project.pump.reloadComponentCanvas(session.session) { [weak self, weak session] event in
            guard let self, let session, self.windowController.windowSession === session, !session.isClosed else { return }
            if case .windowViewChanged = event { self.componentSourcesDidChange() }
        }
    }

    /// 코어가 이 창의 컴포넌트 정의를 바꿨습니다. 문서 revision이 같아도 표시 목록을 다시 읽습니다.
    func componentSourcesDidChange() {
        canvasHost.canvasView.invalidateComponentFrames()
    }
}
