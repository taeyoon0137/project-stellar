import AppKit

/// 메뉴·툴바 명령을 `ToolsMenuModel`로 보내는 응답자(XII.6.4, XII.6.5).
///
/// - 창 수준: `install(in:)`으로 창 응답자 체인의 content view와 창 사이에 끼웁니다. 그러면 텍스트 편집 중에는
///   field editor가 표준 편집 action을 먼저 받고, 캔버스·목록 포커스에서는 이 객체가 받아 활성 창 session에
///   연결합니다(디자인 가이드 8.2). `NSWindow`도 `undo:`에 응답하므로 반드시 창보다 앞에 둡니다.
/// - 앱 수준: 창이 없어도 동작해야 하는 명령(New/Open/Clone Project, Settings)은 `MainMenuBuilder`가 앱 수준
///   router를 명시 target으로 지정합니다.
///
/// 메뉴 검증은 snapshot 조회와 제목 비교만 하므로 메인 스레드 예산(아키텍처 §6.3) 안에서 끝납니다.
final class ShellCommandRouter: NSResponder, NSMenuItemValidation {
    let model: ToolsMenuModel
    private(set) weak var window: NSWindow?

    init(model: ToolsMenuModel) {
        self.model = model
        super.init()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("ShellCommandRouter는 코드로만 생성합니다")
    }

    /// 창 응답자 체인(content view → router → window)에 끼웁니다.
    ///
    /// content view를 바꾸면 다시 호출해야 합니다. 이미 끼워져 있으면 그대로 둡니다.
    @discardableResult
    func install(in window: NSWindow) -> Bool {
        guard let contentView = window.contentView else { return false }
        if contentView.nextResponder === self { return true }
        nextResponder = contentView.nextResponder
        contentView.nextResponder = self
        self.window = window
        return true
    }

    /// 텍스트 컨트롤(field editor 포함)이 first responder인지.
    var isTextEditing: Bool {
        window?.firstResponder is NSText
    }

    /// 현재 snapshot 기준으로 명령을 보냅니다. 비활성 명령은 보내지 않습니다.
    @discardableResult
    func dispatch(_ command: ShellCommand) -> Bool {
        let presentation = ShellCommandPresentation.make(
            command, snapshot: model.snapshot, isTextEditing: isTextEditing
        )
        guard presentation.isEnabled else { return false }
        model.send(command)
        return true
    }

    // MARK: Action

    /// 표준 action이 없는 모든 셸 명령의 진입점. sender(메뉴 항목)에서 명령을 찾습니다.
    @objc func performShellCommand(_ sender: Any?) {
        guard let command = ShellCommand.resolve(from: sender) else { return }
        dispatch(command)
    }

    // 표준 편집 action: 텍스트 편집 중에는 field editor가 먼저 받으므로 여기 오지 않습니다.
    @objc func undo(_ sender: Any?) { dispatch(.undo) }
    @objc func redo(_ sender: Any?) { dispatch(.redo) }
    @objc func cut(_ sender: Any?) { dispatch(.cut) }
    @objc func copy(_ sender: Any?) { dispatch(.copy) }
    @objc func paste(_ sender: Any?) { dispatch(.paste) }
    @objc func delete(_ sender: Any?) { dispatch(.delete) }
    @objc override func selectAll(_ sender: Any?) { dispatch(.selectAll) }

    // MARK: NSMenuItemValidation

    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        guard let command = ShellCommand.resolve(from: menuItem) else { return false }
        let presentation = ShellCommandPresentation.make(
            command, snapshot: model.snapshot, isTextEditing: isTextEditing
        )
        // 같은 값이면 다시 쓰지 않아 메뉴 다시 그리기를 줄입니다.
        if menuItem.title != presentation.title { menuItem.title = presentation.title }
        let state: NSControl.StateValue = presentation.isOn ? .on : .off
        if menuItem.state != state { menuItem.state = state }
        return presentation.isEnabled
    }
}
