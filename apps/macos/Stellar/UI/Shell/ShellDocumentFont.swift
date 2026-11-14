import AppKit
import UniformTypeIdentifiers

extension ShellProjectWindow {
    /// 선택한 파일만 프로젝트가 소유하는 font asset으로 가져옵니다.
    func chooseDocumentFont() {
        guard gitSource.trusted, let window, let session = windowController.windowSession,
            let revision = session.info?.revision
        else { return }
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = ["ttf", "otf", "ttc", "otc"].compactMap { UTType(filenameExtension: $0) }
        panel.title = String(localized: "Document Font")
        panel.prompt = String(localized: "Use Font")
        panel.message = String(
            localized: "Copy a font into this project and use it for the document’s default text style.")
        let face = NSTextField(string: "0")
        let size = NSTextField(string: "16")
        let color = NSColorWell()
        color.color = .black
        let grid = NSGridView(views: [
            [NSTextField(labelWithString: String(localized: "Face index")), face],
            [NSTextField(labelWithString: String(localized: "Size (px)")), size],
            [NSTextField(labelWithString: String(localized: "Text color")), color],
        ])
        grid.column(at: 1).width = 88
        panel.accessoryView = grid
        panel.isAccessoryViewDisclosed = true
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let self, let url = panel.url else { return }
            guard let faceIndex = UInt32(face.stringValue), let px = Double(size.stringValue),
                px.isFinite, px > 0, px <= Double(Int32.max) / 64, (px * 64).rounded() >= 1,
                let chosen = InspectorColor(color.color)
            else {
                self.alertPresenter?(
                    String(localized: "Couldn’t Set Document Font"),
                    String(localized: "Enter a nonnegative face index and a positive font size."), self.window)
                return
            }
            var fill = StellarCanvasColor()
            fill.space = UInt32(chosen.space.rawValue)
            fill.rgba = (
                Float(chosen.red), Float(chosen.green), Float(chosen.blue), Float(chosen.alpha)
            )
            self.applyDocumentFont(
                url.path, face: faceIndex, size: UInt32((px * 64).rounded()), fill: fill, revision: revision)
        }
    }

    @discardableResult
    func applyDocumentFont(_ path: String, face: UInt32, size: UInt32, fill: StellarCanvasColor, revision: UInt64)
        -> Bool
    {
        guard gitSource.trusted, let session = windowController.windowSession, !session.isClosed, !session.isClosing,
            let pump = dataSource.session?.pump
        else { return false }
        return session.track({ done in
            pump.setDocumentFont(
                session: session.session, revision: revision, path: path, face: face, size: size,
                fill: fill, completion: done)
        }) { [weak self, weak session] event in
            guard let self, let session, self.windowController.windowSession === session else { return }
            self.refreshSessionCommands()
            self.canvasHost.canvasView.refreshFrames()
            if case .requestFailed(_, _, let message) = event {
                self.alertPresenter?(String(localized: "Couldn’t Set Document Font"), message, self.window)
            }
        }
    }
}
