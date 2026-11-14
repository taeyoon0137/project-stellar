import AppKit

/// 한 가지 구성(섹션 구성 + 행·필드 구성)을 표시하는 인스펙터 페이지.
///
/// 선택 요약 머리줄, 잠금·읽기 전용 안내, 속성 섹션, modifier 섹션을 스크롤 안에 세로로 쌓습니다. 내용은 frame으로
/// 배치합니다(`InspectorLayout`). 페이지는 `buildSteps(from:collapsedSections:)`의 단계로 나눠 만들고,
/// 다 만든 뒤에만 보입니다. 같은 구성의 snapshot은 컨트롤을 다시 만들지 않고 바뀐 행만 갱신합니다.
///
/// 활성 페이지만 화면에 보입니다. 대기 페이지는 창 안에 alpha 0으로 두어 만들면서 미리 배치·그려 두고,
/// hit test와 접근성 트리에서 뺍니다(가이드 8.3 "시각적으로 숨긴 요소는 접근성 트리에서도 숨김").
/// 전환은 alpha만 바꾸므로 처음 보이는 페이지도 컨트롤 전체를 그 자리에서 다시 그리지 않습니다.
final class InspectorPageView: NSView {
    /// 이 페이지가 맡은 구성.
    let structureKey: String
    let scrollView = InspectorPageScrollView()
    private let documentView = InspectorPageDocumentView()
    private let headerView = InspectorHeaderView()
    private let bannerView = InspectorBannerView()
    private var sections: [(view: InspectorSectionView, rows: InspectorRowsView)] = []
    private(set) var modifierController: InspectorModifierStackController?
    /// 모든 구성 단계를 마쳤는지. 마친 페이지만 보입니다.
    private(set) var isComplete = false
    private var isShellBuilt = false
    private let send: (InspectorCommand) -> Void
    private let onExpansionChange: (InspectorSectionID, Bool) -> Void
    /// 선택한 modifier처럼 페이지 안 구성이 바뀌어 키보드 순서를 다시 계산해야 할 때 호출됩니다.
    var onStructureChange: (() -> Void)?
    /// 처음 보는 modifier 파라미터 구성을 나눠 만들 때 씁니다.
    var scheduleBuild: ((String, [InspectorBuildStep], @escaping () -> Void) -> Void)?

    /// 화면에 보이는 활성 페이지인지. 대기 페이지는 alpha 0이고 hit test·접근성에서 빠집니다.
    var isActive = false {
        didSet {
            guard isActive != oldValue else { return }
            alphaValue = isActive ? 1 : 0
            NSAccessibility.post(element: self, notification: .layoutChanged)
        }
    }

    /// snapshot의 구성 키. 같은 키면 같은 페이지를 재사용합니다. 섹션 id·행 구성·보조 설명 유무·modifier
    /// 섹션 유무로 정하고, 값·레이블·속성 id는 넣지 않습니다(제자리 갱신).
    static func structureKey(of snapshot: InspectorSnapshot) -> String {
        var parts = snapshot.sections.map { section in
            "\(section.id.rawValue){\(InspectorRowsView.compositionKey(rows: section.rows, note: section.note))}"
        }
        parts.append(snapshot.modifiers == nil ? "-" : "modifiers")
        return parts.joined(separator: "/")
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
        alphaValue = 0
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("InspectorPageView는 코드로만 생성합니다")
    }

    // MARK: - 대기 페이지 격리

    override func hitTest(_ point: NSPoint) -> NSView? {
        isActive ? super.hitTest(point) : nil
    }

    override func accessibilityChildren() -> [Any]? {
        isActive ? super.accessibilityChildren() : []
    }

    // MARK: - 나눠 만들기

    /// 페이지를 만드는 단계 목록. 각 단계는 컨트롤 하나 정도이고, 처음 재는 비용이 큰 컨트롤은 `heavy`입니다.
    /// 마지막 단계에서 완료로 표시하고, 값은 호출자가 완료 뒤 최신 snapshot으로 `apply`합니다.
    func buildSteps(from snapshot: InspectorSnapshot, collapsedSections: Set<InspectorSectionID>,
                    warmsUpValues: Bool = false) -> [InspectorBuildStep]
    {
        let forceDisabled = Self.isReadOnly(snapshot)
        var steps: [InspectorBuildStep] = [
            .heavy { [weak self] in self?.prepareShell() },
            .heavy { [weak self] in self?.attachShell() },
            .light { [weak self] in self?.headerView.update(snapshot.selection) },
            .heavy { [weak self] in self?.bannerView.prepare() },
        ]
        for section in snapshot.sections {
            let rows = InspectorRowsView(send: send)
            steps.append(.heavy { [weak self] in
                guard let self else { return }
                let view = InspectorSectionView(
                    id: section.id, title: section.title, expanded: !collapsedSections.contains(section.id)
                )
                view.onExpansionChange = self.onExpansionChange
                view.setContent(rows)
                self.documentView.append(view)
                self.sections.append((view, rows))
            })
            steps += rows.buildSteps(rows: section.rows, note: section.note, forceDisabled: forceDisabled,
                                     warmsUpValues: warmsUpValues)
        }
        if snapshot.modifiers != nil {
            steps.append(.heavy { [weak self] in
                guard let self else { return }
                let controller = InspectorModifierStackController(
                    expanded: !collapsedSections.contains(.modifiers), send: self.send
                )
                controller.sectionView.onExpansionChange = self.onExpansionChange
                controller.onStructureChange = { [weak self] in self?.onStructureChange?() }
                controller.scheduleBuild = { [weak self] key, steps, completion in
                    if let scheduleBuild = self?.scheduleBuild {
                        scheduleBuild(key, steps, completion)
                    } else {
                        steps.forEach { $0.run() }
                        completion()
                    }
                }
                self.documentView.append(controller.sectionView)
                self.modifierController = controller
            })
            for index in 0..<InspectorModifierStackController.buildStepCount {
                steps.append(InspectorBuildStep(cost: InspectorModifierStackController.buildStepCost(index)) {
                    [weak self] in self?.modifierController?.runBuildStep(index)
                })
            }
        }
        steps.append(.light { [weak self] in
            self?.isComplete = true
        })
        return steps
    }

    /// 나눠 만들지 않고 지금 모두 만들고 반영합니다.
    func buildNow(from snapshot: InspectorSnapshot, collapsedSections: Set<InspectorSectionID>) {
        buildSteps(from: snapshot, collapsedSections: collapsedSections).forEach { $0.run() }
        apply(snapshot, collapsedSections: collapsedSections)
    }

    /// 스크롤 뷰·문서 뷰를 창 밖에서 구성하고 배치해 둡니다(scroller 생성·첫 tile 비용을 붙이기와 나눔).
    private func prepareShell() {
        guard !isShellBuilt else { return }
        isShellBuilt = true
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder
        scrollView.documentView = documentView
        scrollView.frame = bounds
        scrollView.tile()

        bannerView.onUnlock = { [weak self] in
            self?.send(.setValue(.layerLocked, .bool(false), phase: .commit))
        }
        documentView.append(headerView)
        documentView.append(bannerView)
    }

    private func attachShell() {
        guard scrollView.superview == nil else { return }
        addSubview(scrollView)
    }

    /// 페이지를 없애는 단계(제거 비용을 나눔). 행 → 섹션 → 페이지 순서입니다.
    func teardownSteps() -> [InspectorBuildStep] {
        var steps: [InspectorBuildStep] = []
        for section in sections {
            for row in section.rows.rowViews {
                steps.append(.light { row.removeFromSuperview() })
            }
            steps.append(.light { section.view.removeFromSuperview() })
        }
        if let modifierController {
            steps.append(.light { modifierController.sectionView.removeFromSuperview() })
        }
        steps.append(.light { [self] in
            self.removeFromSuperview()
        })
        return steps
    }

    // MARK: - snapshot 반영

    /// 다 만든 페이지에 같은 구성의 snapshot을 제자리에서 반영합니다.
    func apply(_ snapshot: InspectorSnapshot, collapsedSections: Set<InspectorSectionID>) {
        guard isComplete else { return }
        let readOnly = Self.isReadOnly(snapshot)
        headerView.update(snapshot.selection)
        let canUnlock = snapshot.field(.layerLocked).map { !readOnly && $0.isEditable } ?? false
        bannerView.update(snapshot.editability, canUnlock: canUnlock)
        for (section, existing) in zip(snapshot.sections, sections) {
            existing.view.setTitle(section.title)
            existing.rows.apply(rows: section.rows, note: section.note, forceDisabled: readOnly)
        }
        if let stack = snapshot.modifiers {
            modifierController?.apply(stack, forceDisabled: readOnly)
        }
    }

    private static func isReadOnly(_ snapshot: InspectorSnapshot) -> Bool {
        if case .readOnly = snapshot.editability { return true }
        return false
    }

    /// 진행 중인 연속 조작(stepper·색·화살표 키)을 모두 `.ended`로 끝냅니다.
    func endGestures() {
        sections.forEach { $0.rows.endGestures() }
        modifierController?.endGestures()
    }

    /// 다른 페이지에서 바뀐 접힘 상태를 맞춥니다.
    func syncExpansion(_ collapsedSections: Set<InspectorSectionID>) {
        for section in sectionViews where section.isExpanded == collapsedSections.contains(section.sectionID) {
            section.setExpanded(!collapsedSections.contains(section.sectionID), animated: false)
        }
    }

    /// 대기 페이지를 새 폭에 다시 배치하는 단계(창·pane 폭이 바뀐 뒤 나눠서). 섹션 하나씩 배치합니다.
    func relayoutSteps(to size: NSSize) -> [InspectorBuildStep] {
        var steps: [InspectorBuildStep] = [.heavy { [weak self] in
            guard let self else { return }
            if self.frame.size != size { self.setFrameSize(size) }
            // 페이지·문서 뷰의 frame 산술만 먼저 합니다(섹션 안 배치는 다음 단계들에서 하나씩).
            self.layout()
            self.documentView.layout()
        }]
        for section in sectionViews {
            steps.append(.heavy { section.layoutSubtreeIfNeeded() })
        }
        return steps
    }

    // MARK: - 배치

    override func layout() {
        super.layout()
        if scrollView.frame != bounds { scrollView.frame = bounds }
        let width = scrollView.contentSize.width
        let size = NSSize(width: width, height: documentView.flowHeight(forWidth: width))
        if documentView.frame.size != size { documentView.setFrameSize(size) }
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
}

/// 페이지 문서 뷰: 머리줄·안내·섹션을 패널 여백 안에서 위에서 아래로 쌓습니다.
final class InspectorPageDocumentView: InspectorFlowView {
    private var entries: [InspectorFlowContent] = []

    func append(_ view: InspectorFlowContent) {
        entries.append(view)
        addSubview(view)
        invalidateFlow()
    }

    private var visibleEntries: [InspectorFlowContent] {
        entries.filter { !$0.isHidden && $0.superview === self }
    }

    override func flowHeight(forWidth width: CGFloat) -> CGFloat {
        let inset = InspectorMetrics.panelInset
        let contentWidth = max(width - 2 * inset, 0)
        let visible = visibleEntries
        let heights = visible.reduce(0) { $0 + $1.flowHeight(forWidth: contentWidth) }
        return ceil(2 * inset + heights + CGFloat(max(visible.count - 1, 0)) * InspectorMetrics.rowSpacing)
    }

    override func layout() {
        super.layout()
        let inset = InspectorMetrics.panelInset
        let contentWidth = max(bounds.width - 2 * inset, 0)
        var y = inset
        for entry in visibleEntries {
            let height = entry.flowHeight(forWidth: contentWidth)
            let frame = NSRect(x: inset, y: y, width: contentWidth, height: height)
            if entry.frame != frame { entry.frame = frame }
            y += height + InspectorMetrics.rowSpacing
        }
    }
}

/// 페이지 스크롤 뷰. scroller 표시가 바뀌어 내용 폭이 달라지면 페이지를 다시 배치합니다.
final class InspectorPageScrollView: NSScrollView {
    override func tile() {
        let previous = contentSize.width
        super.tile()
        if contentSize.width != previous {
            superview?.needsLayout = true
        }
    }
}
