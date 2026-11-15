import AppKit

/// 사이드바 머리 줄 `Page › View ▾`(UX.3.5, 명세 7.1).
///
/// 활성 Page와 View를 한 줄로 보이고, 누르면 문서의 Page별 View 목록이 열립니다. View를 고르면 그 View로
/// 전환합니다(`.activateView`). 목록에는 provider가 실행할 수 있는 항목만 둡니다. Page·View 이름 변경과 추가는
/// 코어 Command가 생긴 뒤 같은 메뉴에 붙입니다(명세 7.1 "Command가 없는 항목은 만들지 않습니다").
@MainActor
final class SidebarPagePopup: NSObject {
    let button = NSPopUpButton(frame: .zero, pullsDown: true)
    private let provider: SidebarDataProvider
    private(set) var snapshot: SidebarSnapshot = .empty

    init(provider: SidebarDataProvider) {
        self.provider = provider
        super.init()
        button.translatesAutoresizingMaskIntoConstraints = false
        button.isBordered = false
        button.controlSize = .regular
        button.font = .systemFont(ofSize: NSFont.systemFontSize, weight: .semibold)
        button.lineBreakMode = .byTruncatingMiddle
        button.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        (button.cell as? NSPopUpButtonCell)?.arrowPosition = .arrowAtBottom
        SidebarAccessibility.setLabel(SidebarStrings.pagePath, of: button)
        button.isHidden = true
    }

    /// 경로 제목(`Page › View`). 이름이 비어 있으면 종류 이름을 씁니다.
    var title: String {
        guard let page = snapshot.pages.first(where: { $0.id == snapshot.activePageID }),
              let view = page.views.first(where: { $0.id == snapshot.activeViewID }) else { return "" }
        return "\(Self.name(page.name, placeholder: SidebarStrings.pageKind)) › \(Self.name(view.name, placeholder: SidebarStrings.viewKind))"
    }

    func apply(_ new: SidebarSnapshot) {
        guard new.generation >= snapshot.generation else { return }
        // 같은 창이 편집 뒤 레이어를 다시 읽는 동안(.loading) 머리 줄 경로를 비우지 않습니다.
        if new.loadState == .loading, new.activeViewID == nil, !snapshot.pages.isEmpty { return }
        let changed = new.pages != snapshot.pages || new.activePageID != snapshot.activePageID
            || new.activeViewID != snapshot.activeViewID
        snapshot = new
        guard changed else { return }
        rebuild()
    }

    private func rebuild() {
        let title = self.title
        button.isHidden = title.isEmpty
        let menu = NSMenu()
        // pull-down 버튼의 첫 항목은 버튼 제목입니다.
        menu.addItem(NSMenuItem(title: title, action: nil, keyEquivalent: ""))
        for page in snapshot.pages {
            menu.addItem(.sectionHeader(title: Self.name(page.name, placeholder: SidebarStrings.pageKind)))
            for view in page.views {
                let item = NSMenuItem(title: Self.name(view.name, placeholder: SidebarStrings.viewKind),
                                      action: #selector(chooseView(_:)), keyEquivalent: "")
                item.target = self
                item.representedObject = view.id.uuid
                item.state = view.id == snapshot.activeViewID ? .on : .off
                menu.addItem(item)
            }
        }
        button.menu = menu
        button.toolTip = title
    }

    /// 메뉴에서 고른 View로 전환합니다. 이미 활성인 View는 바뀌는 것이 없습니다.
    @objc func chooseView(_ sender: NSMenuItem) {
        guard let uuid = sender.representedObject as? UUID, SidebarStableID(uuid) != snapshot.activeViewID else { return }
        let command = SidebarCommand.activateView(SidebarStableID(uuid))
        guard provider.validate(command).isAllowed else { return }
        provider.perform(command)
    }

    private static func name(_ name: String, placeholder: String) -> String {
        name.isEmpty ? placeholder : name
    }
}
