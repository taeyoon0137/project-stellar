import AppKit

extension ShellProjectWindow {
    /// 새 정의는 독립 프로젝트 파일이며 현재 캔버스의 선택·문서 history를 사용하지 않습니다.
    func promptForComponent() {
        guard let window, let project = dataSource.session, let session = windowController.windowSession,
              !session.isClosing, !session.isClosed, gitSource.trusted else { return }
        let sheet = NSAlert()
        sheet.messageText = String(localized: "New Component")
        sheet.informativeText = String(localized: "Name the new component to save it in design/components.")
        let name = NSTextField(frame: NSRect(x: 0, y: 0, width: 280, height: 24))
        name.placeholderString = String(localized: "Component name")
        name.setAccessibilityLabel(String(localized: "Component name"))
        sheet.accessoryView = name
        sheet.addButton(withTitle: String(localized: "Create"))
        sheet.addButton(withTitle: String(localized: "Cancel"))
        sheet.window.initialFirstResponder = name
        sheet.beginSheetModal(for: window) { [weak self] response in
            guard response == .alertFirstButtonReturn, let self, self.dataSource.session === project,
                  !session.isClosed, !session.isClosing, self.gitSource.trusted else { return }
            let value = name.stringValue
            let failed = String(localized: "Couldn’t Create Component")
            guard !value.isEmpty, value.utf8.count <= 255 else {
                self.alertPresenter?(failed, String(localized: "Enter a component name that fits in a file name."), window)
                return
            }
            let submitted = session.track({ done in
                project.pump.createComponent(session: session.session, name: value, completion: done)
            }, completion: { [weak self] event in
                guard let self, self.dataSource.session === project else { return }
                switch event {
                case .componentCreated(_, session.session, let filename):
                    if self.componentLibrary?.panel.isVisible == true { self.componentLibrary?.refresh() }
                    self.alertPresenter?(String(localized: "Component Created"), "design/components/" + filename, window)
                case .requestFailed(_, _, let message):
                    self.alertPresenter?(failed, message, window)
                default:
                    self.alertPresenter?(failed, String(localized: "Unexpected response from the project."), window)
                }
            })
            if !submitted { self.alertPresenter?(failed, String(localized: "The project is busy. Try again."), window) }
        }
    }
}
