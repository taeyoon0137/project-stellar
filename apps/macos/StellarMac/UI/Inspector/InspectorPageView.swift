import AppKit

/// 한 가지 섹션 구성(예: Layer·Frame·Appearance·Modifiers)을 표시하는 인스펙터 페이지.
///
/// 선택 요약 머리줄, 잠금·읽기 전용 안내, 속성 섹션, modifier 섹션을 스크롤 안에 세로로 쌓습니다.
/// 같은 구성의 snapshot은 컨트롤을 다시 만들지 않고 바뀐 행만 갱신합니다.
final class InspectorPageView: NSView {
    /// 이 페이지가 맡은 구성(섹션 id 목록 + modifier 섹션 여부).
    let structureKey: String
    let scrollView = NSScrollView()
    private let documentView = InspectorFlippedView()
    private let contentStack = NSStackView()
    private let headerView = InspectorHeaderView()
    private let bannerView = InspectorBannerView()
    private var sections: [(view: InspectorSectionView, rows: InspectorRowsView)] = []
    private(set) var modifierController: InspectorModifierStackController?
    private let send: (InspectorCommand) -> Void
    private let onExpansionChange: (InspectorSectionID, Bool) -> Void
    /// 선택한 modifier처럼 페이지 안 구성이 바뀌어 키보드 순서를 다시 계산해야 할 때 호출됩니다.
    var onStructureChange: (() -> Void)?

    /// snapshot의 구성 키. 같은 키면 같은 페이지를 재사용합니다.
    static func structureKey(of snapshot: InspectorSnapshot) -> String {
        (snapshot.sections.map(\.id.rawValue) + [snapshot.modifiers == nil ? "-" : "modifiers"])
            .joined(separator: "/")
    }

    init(
        structureKey: String,
        send: @escaping (InspectorCommand) -> Void,
        onExpansionChange: @escaping (InspectorSectionID, Bool) -> Void
    ) {
        self.structureKey = structureKey
        self.send = send
        self.onExpansionChange = onExpansionChange
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        build()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("InspectorPageView는 코드로만 생성합니다")
    }

    private func build() {
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.documentView = documentView
        documentView.translatesAutoresizingMaskIntoConstraints = false

        contentStack.orientation = .vertical
        contentStack.alignment = .leading
        contentStack.spacing = InspectorMetrics.rowSpacing
        contentStack.edgeInsets = NSEdgeInsets(
            top: InspectorMetrics.panelInset, left: InspectorMetrics.panelInset,
            bottom: InspectorMetrics.panelInset, right: InspectorMetrics.panelInset
        )
        contentStack.translatesAutoresizingMaskIntoConstraints = false
        documentView.addSubview(contentStack)

        bannerView.onUnlock = { [weak self] in
            self?.send(.setValue(.layerLocked, .bool(false), phase: .commit))
        }

        addSubview(scrollView)
        let clip = scrollView.contentView
        NSLayoutConstraint.activate([
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: trailingAnchor),
            scrollView.topAnchor.constraint(equalTo: topAnchor),
            scrollView.bottomAnchor.constraint(equalTo: bottomAnchor),
            documentView.leadingAnchor.constraint(equalTo: clip.leadingAnchor),
            documentView.trailingAnchor.constraint(equalTo: clip.trailingAnchor),
            documentView.topAnchor.constraint(equalTo: clip.topAnchor),
            contentStack.leadingAnchor.constraint(equalTo: documentView.leadingAnchor),
            contentStack.trailingAnchor.constraint(equalTo: documentView.trailingAnchor),
            contentStack.topAnchor.constraint(equalTo: documentView.topAnchor),
            contentStack.bottomAnchor.constraint(equalTo: documentView.bottomAnchor),
        ])
        addContent(headerView)
        addContent(bannerView)
    }

    // MARK: - snapshot 반영

    /// 준비된(`.ready`) snapshot을 반영합니다. 구성 키가 같은 snapshot만 받습니다.
    func apply(_ snapshot: InspectorSnapshot, collapsedSections: Set<InspectorSectionID>) {
        let readOnly: Bool
        if case .readOnly = snapshot.editability { readOnly = true } else { readOnly = false }
        headerView.update(snapshot.selection)
        let canUnlock = snapshot.field(.layerLocked).map { !readOnly && $0.isEditable } ?? false
        bannerView.update(snapshot.editability, canUnlock: canUnlock)
        applySections(snapshot.sections, collapsedSections: collapsedSections, forceDisabled: readOnly)
        applyModifiers(snapshot.modifiers, collapsedSections: collapsedSections, forceDisabled: readOnly)
    }

    private func applySections(
        _ newSections: [InspectorSection], collapsedSections: Set<InspectorSectionID>, forceDisabled: Bool
    ) {
        if newSections.map(\.id) == sections.map(\.view.sectionID) {
            for (section, existing) in zip(newSections, sections) {
                existing.view.setTitle(section.title)
                existing.rows.apply(rows: section.rows, note: section.note, forceDisabled: forceDisabled)
            }
            return
        }
        // 구성 키가 같으면 여기에 오지 않습니다. 처음 만들 때만 섹션을 쌓습니다.
        sections.forEach { $0.view.removeFromSuperview() }
        sections.removeAll()
        for section in newSections {
            let sectionView = InspectorSectionView(
                id: section.id, title: section.title, expanded: !collapsedSections.contains(section.id)
            )
            sectionView.onExpansionChange = onExpansionChange
            let rows = InspectorRowsView(send: send)
            rows.apply(rows: section.rows, note: section.note, forceDisabled: forceDisabled)
            sectionView.setContent([rows])
            addContent(sectionView)
            sections.append((sectionView, rows))
        }
        if let modifierSection = modifierController?.sectionView {
            // modifier 섹션은 항상 마지막입니다.
            contentStack.removeArrangedSubview(modifierSection)
            modifierSection.removeFromSuperview()
            addContent(modifierSection)
        }
    }

    private func applyModifiers(
        _ stack: InspectorModifierStack?, collapsedSections: Set<InspectorSectionID>, forceDisabled: Bool
    ) {
        guard let stack else {
            modifierController?.sectionView.removeFromSuperview()
            modifierController = nil
            return
        }
        let controller: InspectorModifierStackController
        if let existing = modifierController {
            controller = existing
        } else {
            controller = InspectorModifierStackController(
                expanded: !collapsedSections.contains(.modifiers), send: send
            )
            controller.sectionView.onExpansionChange = onExpansionChange
            controller.onStructureChange = { [weak self] in self?.onStructureChange?() }
            addContent(controller.sectionView)
            modifierController = controller
        }
        controller.apply(stack, forceDisabled: forceDisabled)
    }

    /// 다른 페이지에서 바뀐 접힘 상태를 맞춥니다.
    func syncExpansion(_ collapsedSections: Set<InspectorSectionID>) {
        for section in sectionViews where section.isExpanded == collapsedSections.contains(section.sectionID) {
            section.setExpanded(!collapsedSections.contains(section.sectionID), animated: false)
        }
    }

    // MARK: - 조회

    /// 화면 순서의 섹션 뷰(modifier 섹션 포함).
    var sectionViews: [InspectorSectionView] {
        sections.map(\.view) + (modifierController.map { [$0.sectionView] } ?? [])
    }

    /// 보이는 컨트롤을 화면 순서대로 모은 키보드 포커스 순서(가이드 8.2). 접힌 섹션 본문은 뺍니다.
    var keyViews: [NSView] {
        var chain: [NSView] = []
        if !bannerView.isHidden, !bannerView.actionButton.isHidden {
            chain.append(bannerView.actionButton)
        }
        for section in sections {
            chain.append(section.view.disclosureButton)
            if section.view.isExpanded {
                chain += section.rows.keyViews
            }
        }
        if let modifierController {
            chain += modifierController.keyViews
        }
        return chain
    }

    // MARK: - 배치 도우미

    private func addContent(_ view: NSView) {
        contentStack.addArrangedSubview(view)
        view.widthAnchor.constraint(
            equalTo: contentStack.widthAnchor, constant: -2 * InspectorMetrics.panelInset
        ).isActive = true
    }
}
