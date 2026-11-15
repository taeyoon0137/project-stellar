import AppKit

/// FSEvents가 알린 뒤에만 메인 큐를 깨웁니다. 읽기 가능한 fd 하나가 여러 변경을 합칩니다.
@MainActor
final class ShellComponentFileWatch {
    let session: ShellWindowSessionHandle
    private var source: DispatchSourceRead?
    private var invalidated = false
    private(set) var unavailable = false

    init(runtime: UInt64, session: ShellWindowSessionHandle, changed: @escaping () -> Void) {
        self.session = session
        let handle = session.session
        Task { [weak self] in
            let fd = await Task.detached { CoreRuntime.componentWatchFD(handle: runtime, session: handle) }.value
            guard fd >= 0 else { self?.unavailable = true; return }
            guard let self, !self.invalidated else { Darwin.close(fd); return }
            let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: .main)
            self.source = source
            source.setCancelHandler { Darwin.close(fd) }
            source.setEventHandler { [weak self] in
                guard self?.invalidated == false else { return }
                var signal: UInt8 = 0
                var received = false
                for _ in 0..<64 {
                    guard Darwin.recv(fd, &signal, 1, 0) == 1 else { break }
                    received = true
                }
                if received { changed() }
            }
            source.resume()
            // 최초 장면 읽기와 stream 시작 사이에 완료된 외부 쓰기도 반영합니다.
            changed()
        }
    }

    deinit { source?.cancel() }

    func cancel() {
        invalidated = true
        source?.cancel()
        source = nil
    }
}

extension ShellProjectWindow {
    func updateComponentFileWatch(trusted: Bool) {
        guard trusted, let project = dataSource.session, let session = windowController.windowSession,
              !session.isClosed, !session.isClosing else {
            componentFileWatch?.cancel()
            componentFileWatch = nil
            return
        }
        if let watch = componentFileWatch, watch.session === session, !watch.unavailable { return }
        componentFileWatch?.cancel()
        componentFileWatch = ShellComponentFileWatch(runtime: project.pump.runtimeHandle, session: session) { [weak self, weak session] in
            guard let self, let session, self.windowController.windowSession === session,
                  !session.isClosing, !session.isClosed, self.gitSource.trusted else { return }
            self.reloadComponentCanvas(onWindowReturn: true)
        }
    }
}

extension ShellProjectWindow {
    func reloadComponentCanvas(onWindowReturn: Bool = false) {
        guard let project = dataSource.session, let session = windowController.windowSession, !session.isClosing, !session.isClosed else { return }
        if onWindowReturn {
            guard gitSource.trusted else { return }
            if session.checkingComponentsOnReturn {
                session.recheckComponentsOnReturn = true
                return
            }
        }
        // 원본 읽기는 저장 대기 중인 편집 명령에 포함하지 않습니다.
        let result = project.pump.reloadComponentCanvas(session.session) { [weak self, weak session] event in
            guard let self, let session, self.windowController.windowSession === session, !session.isClosed else { return }
            if case .windowViewChanged = event {
                self.canvasHost.canvasView.invalidateComponentFrames()
            }
            if onWindowReturn {
                session.checkingComponentsOnReturn = false
                if session.recheckComponentsOnReturn {
                    session.recheckComponentsOnReturn = false
                    self.reloadComponentCanvas(onWindowReturn: true)
                }
            }
        }
        if onWindowReturn, case .success = result { session.checkingComponentsOnReturn = true }
    }
}
