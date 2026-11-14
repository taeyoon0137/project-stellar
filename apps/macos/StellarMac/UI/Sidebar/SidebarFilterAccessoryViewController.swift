import AppKit

/// 사이드바 상단 accessory의 레이어 필터 필드(디자인 가이드 2.3, 6.2).
///
/// `NSSplitViewItemAccessoryViewController`로 사이드바 split view item에 붙여 safe area와 scroll edge effect를
/// 시스템이 처리하게 합니다. 입력은 AppKit `NSSearchField`이며(아키텍처 §11.2) 지우기 버튼과 Escape로 지우기는
/// 시스템 동작을 그대로 씁니다. 질의 실행은 셸이 하지 않고 `.setFilter` command로 코어에 넘깁니다.
final class SidebarFilterAccessoryViewController: NSSplitViewItemAccessoryViewController {
    let searchField = NSSearchField()

    /// 질의가 바뀌면 호출됩니다(빈 문자열은 필터 해제).
    var onFilterChange: ((String) -> Void)?

    override func loadView() {
        searchField.translatesAutoresizingMaskIntoConstraints = false
        searchField.placeholderString = SidebarStrings.filterPlaceholder
        // 접근성 서버가 읽는 요소는 검색 필드 cell이므로 cell에도 레이블을 둡니다.
        SidebarAccessibility.setLabel(SidebarStrings.filterLabel, of: searchField)
        searchField.toolTip = SidebarStrings.filterLabel
        // 입력이 잠시 멈췄을 때 action을 보내는 시스템 지연을 씁니다. 키 입력마다 코어 질의를 만들지 않습니다.
        searchField.sendsSearchStringImmediately = false
        searchField.sendsWholeSearchString = false
        searchField.target = self
        searchField.action = #selector(searchChanged(_:))

        let container = NSView()
        container.addSubview(searchField)
        NSLayoutConstraint.activate([
            searchField.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            searchField.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            searchField.topAnchor.constraint(equalTo: container.topAnchor),
            searchField.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])
        view = container

        // scroll edge effect 스타일 API는 macOS 26.1부터입니다. 26.0에서는 시스템 기본값을 씁니다(디자인 가이드 2.3).
        if #available(macOS 26.1, *) {
            preferredScrollEdgeEffectStyle = .soft
        }
    }

    /// snapshot의 질의와 필드를 맞춥니다. 사용자가 입력 중이면 덮어쓰지 않습니다.
    func sync(query: String) {
        loadViewIfNeeded()
        guard searchField.currentEditor() == nil, searchField.stringValue != query else { return }
        searchField.stringValue = query
    }

    /// 필드를 비우고 필터 해제를 알립니다(상태 화면의 "Clear Filter").
    func clear() {
        loadViewIfNeeded()
        searchField.stringValue = ""
        onFilterChange?("")
    }

    /// 필터 필드로 포커스를 옮깁니다. 메뉴 명령(XII.6.5)이 호출합니다.
    func focus() {
        view.window?.makeFirstResponder(searchField)
    }

    @objc private func searchChanged(_ sender: NSSearchField) {
        onFilterChange?(sender.stringValue)
    }
}
