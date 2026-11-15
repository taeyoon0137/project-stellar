import AppKit

/// 오른쪽 속성 탭(UX 명세 6장). 코어가 만든 확정 선택의 CSS 속성 행을 보이고, 입력을 코어로 보냅니다.
///
/// pane 재질은 시스템이 정하므로 자체 배경을 그리지 않습니다. 같은 구성의 snapshot은 값만 제자리에서 갱신합니다.
@MainActor
final class InspectorViewController: NSViewController {
    let dataSource: InspectorDataSource
    /// Escape로 값 입력을 끝낸 뒤 포커스를 돌려줄 곳(캔버스). 창이 연결합니다.
    var focusCanvas: (() -> Void)?

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

    /// snapshot을 반영합니다. 구성(섹션·행 이름·flag)이 같으면 값만 갱신합니다.
    func apply(_ snapshot: InspectorSnapshot) {
        appliedSnapshot = snapshot
        header.update(snapshot)
        if snapshot.structure == appliedStructure {
            for row in snapshot.rows { rowViews[row.name]?.update(row) }
            return
        }
        rebuild(snapshot)
    }

    /// 속성 이름(CSS longhand)으로 값 컨트롤을 찾습니다(키보드 포커스 이동·접근성).
    func control(identifier: String) -> NSView? {
        rowViews[identifier]?.field
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
        // 레이블 열은 가장 긴 CSS 이름에 맞춥니다(잘리지 않게, 패널 폭의 60% 상한).
        let attributes: [NSAttributedString.Key: Any] = [.font: InspectorFonts.label]
        let longest = snapshot.rows.map {
            ceil(($0.name as NSString).size(withAttributes: attributes).width) + ($0.flags.contains(.longhand) ? 12 : 0)
                + ($0.flags.contains(.group) ? 18 : 0)
        }.max() ?? 0
        let available = max(view.bounds.width - 2 * InspectorMetrics.panelInset, 260)
        let labelWidth = min(max(longest + 4, 70), available * 0.6)
        var currentGroup: String?
        for row in snapshot.rows {
            let section = sectionViews[row.section] ?? {
                let view = InspectorSectionView(kind: row.section, collapsed: collapsed.contains(row.section))
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

    /// 섹션을 펼쳐 보이게 합니다(예: 텍스트 편집 시작 때 Typography).
    func reveal(_ section: InspectorSectionKind) {
        sectionViews[section]?.setCollapsed(false)
    }
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
            icon.image = NSImage(systemSymbolName: InspectorSymbols.noSelection, accessibilityDescription: nil)
            title.stringValue = String(localized: "No Selection")
            detail.stringValue = String(localized: "Select a layer on the canvas or in Layers to edit its CSS properties. V Move · F Frame · R Rectangle · O Ellipse · L Line · P Polygon · T Text")
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
            detail.stringValue = String(localized: "Mixed values apply to every selected layer.")
            detail.isHidden = false
        }
    }
}

extension InspectorNodeClasses {
    /// 단일 분류 목록.
    static let allSingle: [InspectorNodeClasses] = [.frame, .rectangle, .ellipse, .line, .polygon, .text, .image, .other]
}
