import AppKit

/// 오른쪽 속성 탭(UX 명세 6장). 코어가 만든 확정 선택의 CSS 속성 행을 보이고, 입력을 코어로 보냅니다.
///
/// pane 재질은 시스템이 정하므로 자체 배경을 그리지 않습니다. 같은 구성의 snapshot은 값만 제자리에서 갱신합니다.
@MainActor
final class InspectorViewController: NSViewController {
    let dataSource: InspectorDataSource
    /// Escape로 값 입력을 끝낸 뒤 포커스를 돌려줄 곳(캔버스). 창이 연결합니다.
    var focusCanvas: (() -> Void)?
    /// 캔버스 드래그·크기 조절 중인지. 창이 연결합니다(UX.2.12).
    var isCanvasGestureActive: (() -> Bool)?

    /// 지금 레이블 scrub 중인 행. 행마다 시작·끝을 한 번씩 알리므로 집합으로 둡니다(중복 시작이 쌓이지 않음).
    private var panelGestures: Set<ObjectIdentifier> = []
    /// 제스처 중 미룬 구성 변경 snapshot.
    private var deferredSnapshot: InspectorSnapshot?
    private var deferTimer: Timer?
    /// 아직 없는 섹션을 다음 구성에서 펼칠 것(텍스트 편집 시작 직후 Text 선택이 오기 전).
    private var pendingReveal: InspectorSectionKind?

    private let scrollView = NSScrollView()
    private let content = InspectorFlippedView()
    private let stack = NSStackView()
    private let header = InspectorHeaderView()
    /// 선택마다 다시 만드는 행 영역.
    private let body = NSStackView()
    private var rowViews: [String: InspectorRowView] = [:]
    private var sectionViews: [InspectorSectionKind: InspectorSectionView] = [:]
    /// 펼친 묶음 행(shorthand) 이름. 창 안에서만 기억합니다.
    private var expandedGroups: Set<String> = []
    /// 묶음 행 이름 → 그 longhand 행.
    private var groupMembers: [String: [InspectorRowView]] = [:]
    private var appliedStructure: [String] = []
    private(set) var appliedSnapshot = InspectorSnapshot.empty
    /// 지금 구성에서 가장 긴 레이블 몫(펼침 단추·들여쓰기 포함)과 마지막으로 맞춘 패널 폭.
    private var longestLabel: CGFloat = 0
    private var laidOutWidth: CGFloat = 0

    init(dataSource: InspectorDataSource) {
        self.dataSource = dataSource
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("InspectorViewController는 코드로만 생성합니다") }

    override func loadView() {
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder
        // 내용이 짧아도 위에서부터 보이게 clip view를 뒤집고 문서 view를 위·좌우에 붙입니다.
        scrollView.contentView = InspectorClipView()
        content.translatesAutoresizingMaskIntoConstraints = false
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = InspectorMetrics.rowSpacing
        stack.edgeInsets = NSEdgeInsets(top: InspectorMetrics.panelInset, left: InspectorMetrics.panelInset,
                                        bottom: InspectorMetrics.panelInset, right: InspectorMetrics.panelInset)
        stack.translatesAutoresizingMaskIntoConstraints = false
        body.orientation = .vertical
        body.alignment = .leading
        body.spacing = InspectorMetrics.sectionSpacing
        body.translatesAutoresizingMaskIntoConstraints = false
        stack.addArrangedSubview(header)
        stack.addArrangedSubview(body)
        content.addSubview(stack)
        scrollView.documentView = content
        NSLayoutConstraint.activate([
            content.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),
            content.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
            content.widthAnchor.constraint(equalTo: scrollView.contentView.widthAnchor),
            stack.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            stack.topAnchor.constraint(equalTo: content.topAnchor),
            stack.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            header.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -2 * InspectorMetrics.panelInset),
            body.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -2 * InspectorMetrics.panelInset),
        ])
        view = scrollView
        view.setAccessibilityLabel(String(localized: "Properties"))
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        dataSource.snapshotHandler = { [weak self] snapshot in self?.apply(snapshot) }
        apply(dataSource.snapshot)
    }

    /// 제스처(캔버스 드래그·크기 조절·영역 선택, 패널 레이블 scrub) 중인지.
    private var gestureActive: Bool { !panelGestures.isEmpty || isCanvasGestureActive?() == true }

    /// snapshot을 반영합니다. 구성(섹션·행 이름·flag)이 같으면 값만 제자리에서 갱신합니다. 제스처 중에는 구성을
    /// 고정하고(섹션 추가·삭제·스크롤 0) 같은 이름 행의 값만 바꾼 뒤, 제스처가 끝나면 미룬 구성을 반영합니다.
    func apply(_ snapshot: InspectorSnapshot) {
        if snapshot.structure == appliedStructure {
            deferredSnapshot = nil
            appliedSnapshot = snapshot
            header.update(snapshot)
            for row in snapshot.rows { rowViews[row.name]?.update(row) }
            return
        }
        if gestureActive {
            deferredSnapshot = snapshot
            for row in snapshot.rows where rowViews[row.name]?.row.structure == row.structure {
                rowViews[row.name]?.update(row)
            }
            scheduleDeferredApply()
            return
        }
        deferredSnapshot = nil
        appliedSnapshot = snapshot
        header.update(snapshot)
        rebuild(snapshot)
    }

    private func scheduleDeferredApply() {
        guard deferTimer == nil else { return }
        deferTimer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] timer in
            MainActor.assumeIsolated {
                guard let self else { timer.invalidate(); return }
                guard !self.gestureActive else { return }
                timer.invalidate()
                self.deferTimer = nil
                if let pending = self.deferredSnapshot { self.apply(pending) }
            }
        }
    }

    private func gesture(_ row: InspectorRowView, active: Bool) {
        if active {
            panelGestures.insert(ObjectIdentifier(row))
        } else {
            panelGestures.remove(ObjectIdentifier(row))
            if deferredSnapshot != nil { scheduleDeferredApply() }
        }
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        // 패널 폭이 바뀌면 레이블 열을 다시 맞춥니다(구성을 만들 때 폭이 아직 0이었던 경우 포함).
        let width = view.bounds.width
        guard abs(width - laidOutWidth) > 0.5 else { return }
        laidOutWidth = width
        let labelWidth = currentLabelWidth()
        for row in rowViews.values { row.setLabelWidth(labelWidth) }
    }

    /// 레이블 열 폭: 가장 긴 CSS 이름이 잘리지 않는 폭, 패널 안쪽 폭의 60% 상한.
    private func currentLabelWidth() -> CGFloat {
        let available = max(view.bounds.width - 2 * InspectorMetrics.panelInset, 0)
        return min(max(longestLabel + 4, 70), max(available * 0.6, 70))
    }

    /// 속성 이름(CSS longhand)으로 값 컨트롤을 찾습니다(키보드 포커스 이동·접근성).
    func control(identifier: String) -> NSView? {
        rowViews[identifier]?.keyControl
    }

    private func rebuild(_ snapshot: InspectorSnapshot) {
        appliedStructure = snapshot.structure
        for view in body.arrangedSubviews {
            body.removeArrangedSubview(view)
            view.removeFromSuperview()
        }
        rowViews.removeAll()
        sectionViews.removeAll()
        // 섹션은 코어 행의 섹션 순서(속성 모델 12장)대로, 행이 하나라도 있는 섹션만 둡니다.
        let classes = snapshot.classes
        let collapsed = InspectorCollapseMemory.collapsed(for: classes)
        groupMembers.removeAll()
        // 레이블 열은 가장 긴 CSS 이름에 맞춥니다(잘리지 않게, 패널 폭의 60% 상한). 폭이 바뀌면 viewDidLayout이
        // 다시 맞춥니다.
        let attributes: [NSAttributedString.Key: Any] = [.font: InspectorFonts.label]
        longestLabel = snapshot.rows.map {
            ceil(($0.name as NSString).size(withAttributes: attributes).width) + ($0.flags.contains(.longhand) ? 12 : 0)
                + ($0.flags.contains(.group) ? 18 : 0)
        }.max() ?? 0
        let labelWidth = currentLabelWidth()
        var currentGroup: String?
        for row in snapshot.rows {
            let section = sectionViews[row.section] ?? {
                let view = InspectorSectionView(kind: row.section,
                                                title: InspectorPropertyCatalog.title(row.section, classes: classes),
                                                collapsed: collapsed.contains(row.section))
                view.onToggle = { [weak self] _, _ in self?.rememberCollapse(for: classes) }
                sectionViews[row.section] = view
                body.addArrangedSubview(view)
                view.widthAnchor.constraint(equalTo: body.widthAnchor).isActive = true
                return view
            }()
            let rowView = InspectorRowView(row: row, labelWidth: labelWidth, expanded: expandedGroups.contains(row.name))
            rowView.onEdit = { [weak self] name, input, phase, completion in
                self?.dataSource.edit(name, input, phase: phase, completion: completion)
            }
            rowView.onEscape = { [weak self] in self?.focusCanvas?() }
            rowView.onGesture = { [weak self, weak rowView] active in
                guard let self, let rowView else { return }
                self.gesture(rowView, active: active)
            }
            if row.flags.contains(.group) {
                currentGroup = row.name
                let name = row.name
                rowView.onToggleGroup = { [weak self] expanded in self?.setGroup(name, expanded: expanded) }
            } else if row.flags.contains(.longhand), let group = currentGroup {
                groupMembers[group, default: []].append(rowView)
                rowView.isHidden = !expandedGroups.contains(group)
            } else {
                currentGroup = nil
            }
            rowViews[row.name] = rowView
            section.add(rowView)
        }
        if let reveal = pendingReveal, let view = sectionViews[reveal] {
            view.setCollapsed(false)
            pendingReveal = nil
        }
        // Tab은 표시 순서대로 다음 값 컨트롤로 갑니다(숨은·비활성 컨트롤은 AppKit이 건너뜀).
        let controls = snapshot.rows.compactMap { rowViews[$0.name]?.keyControl }
        for (current, next) in zip(controls, controls.dropFirst()) { current.nextKeyView = next }
        controls.last?.nextKeyView = nil
    }

    /// 묶음 행을 펼치면 longhand 행이 보입니다(UX 명세 6.3).
    private func setGroup(_ name: String, expanded: Bool) {
        if expanded { expandedGroups.insert(name) } else { expandedGroups.remove(name) }
        for member in groupMembers[name] ?? [] { member.isHidden = !expanded }
    }

    private func rememberCollapse(for classes: InspectorNodeClasses) {
        let collapsed = Set(sectionViews.values.filter(\.isCollapsed).map(\.kind))
        InspectorCollapseMemory.set(collapsed, for: classes)
    }

    /// 섹션을 펼쳐 보이게 합니다(예: 텍스트 편집 시작 때 Typography). 아직 없으면 다음 구성에서 펼칩니다.
    func reveal(_ section: InspectorSectionKind) {
        if let view = sectionViews[section] {
            view.setCollapsed(false)
            pendingReveal = nil
        } else {
            pendingReveal = section
        }
    }
}

/// 위에서 시작하는 clip view.
final class InspectorClipView: NSClipView {
    override var isFlipped: Bool { true }
}

/// 위에서 아래로 쌓는 문서 view.
final class InspectorFlippedView: NSView {
    override var isFlipped: Bool { true }
}

/// 머리: 종류 글리프, 이름, 다중 선택 수. 선택이 없으면 안내만 보입니다.
@MainActor
final class InspectorHeaderView: NSView {
    private let icon = NSImageView()
    private let title = NSTextField(labelWithString: "")
    private let detail = NSTextField(wrappingLabelWithString: "")

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false
        title.font = InspectorFonts.headline
        title.lineBreakMode = .byTruncatingTail
        detail.font = InspectorFonts.caption
        detail.textColor = .secondaryLabelColor
        icon.symbolConfiguration = .init(textStyle: .headline)
        let line = NSStackView(views: [icon, title])
        line.spacing = 6
        let column = NSStackView(views: [line, detail])
        column.orientation = .vertical
        column.alignment = .leading
        column.spacing = 2
        column.translatesAutoresizingMaskIntoConstraints = false
        addSubview(column)
        NSLayoutConstraint.activate([
            column.leadingAnchor.constraint(equalTo: leadingAnchor),
            column.trailingAnchor.constraint(equalTo: trailingAnchor),
            column.topAnchor.constraint(equalTo: topAnchor),
            column.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -4),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("코드로만 생성합니다") }

    func update(_ snapshot: InspectorSnapshot) {
        let single = InspectorNodeClasses.allSingle.first { snapshot.classes == $0 }
        if snapshot.count == 0 {
            // 선택이 없으면 현재 Page 요약(코어가 준 `Page · View`)과 자주 쓰는 키만 보입니다(UX 명세 6.1).
            icon.image = NSImage(systemSymbolName: InspectorSymbols.noSelection, accessibilityDescription: nil)
            title.stringValue = snapshot.displayName.isEmpty ? String(localized: "No Selection") : snapshot.displayName
            detail.stringValue = String(localized: "Select a layer to edit its CSS properties.") + "\n" + Self.toolKeys
            detail.isHidden = false
        } else if snapshot.count == 1 {
            icon.image = NSImage(systemSymbolName: InspectorSymbols.node(single ?? .other), accessibilityDescription: nil)
            title.stringValue = snapshot.displayName.isEmpty ? InspectorSymbols.kindName(single ?? .other) : snapshot.displayName
            detail.stringValue = InspectorSymbols.kindName(single ?? .other)
            detail.isHidden = false
        } else {
            icon.image = NSImage(systemSymbolName: single.map(InspectorSymbols.node) ?? InspectorSymbols.mixedSelection,
                                 accessibilityDescription: nil)
            title.stringValue = String(localized: "\(snapshot.count) Layers")
            detail.stringValue = ""
            detail.isHidden = true
        }
    }

    /// 도구 단축키 안내. 표시 키는 캔버스가 해석하는 키(`CanvasTool.singleKey`)에서 만듭니다.
    static var toolKeys: String {
        CanvasTool.allCases.compactMap { tool in
            tool.singleKey.map { "\(String($0).uppercased()) \(tool.label)" }
        }.joined(separator: " · ")
    }
}

extension InspectorNodeClasses {
    /// 단일 분류 목록.
    static let allSingle: [InspectorNodeClasses] = [.frame, .rectangle, .ellipse, .line, .polygon, .text, .image, .other]
}
