import AppKit

// MARK: - 페이지 캐시

/// 세대별 페이지 캐시(XIV.13.1, 디자인 가이드 6.4·9장).
///
/// 화면에 필요한 행의 페이지만 요청하고, 상한을 넘으면 가장 오래 쓰지 않은 페이지를 버립니다.
/// 세대가 바뀌어도 행 수가 같으면 직전 페이지를 새 페이지가 올 때까지 "stale" 대체값으로 남겨
/// 체크박스 클릭마다 목록이 깜빡이지 않게 합니다. stale 행은 조작할 수 없습니다.
struct GitPagedRowCache<Row> {
    enum Lookup {
        case fresh(Row)
        case stale(Row)
        case missing
    }

    private struct Page {
        var rows: [Row]
        var lastUse: UInt64
    }

    let pageSize: Int
    let capacity: Int
    private(set) var generation: UInt64 = 0
    private(set) var rowCount = 0
    private var pages: [Int: Page] = [:]
    private var stalePages: [Int: [Row]] = [:]
    private var inFlight: Set<Int> = []
    private var clock: UInt64 = 0
    /// 진단: 캐시 생성 이후 요청한 페이지 수 누계.
    private(set) var requestCount = 0

    init(pageSize: Int = GitPanelMetrics.pageSize, capacity: Int = GitPanelMetrics.pageCapacity) {
        self.pageSize = max(1, pageSize)
        self.capacity = max(1, capacity)
    }

    var cachedPageCount: Int { pages.count }

    mutating func reset(generation: UInt64, rowCount: Int) {
        stalePages = rowCount == self.rowCount ? pages.mapValues(\.rows) : [:]
        self.generation = generation
        self.rowCount = max(0, rowCount)
        pages = [:]
        inFlight = []
    }

    /// 행을 찾습니다. fresh 조회는 LRU 시각을 갱신합니다.
    mutating func lookup(_ row: Int) -> Lookup {
        guard row >= 0, row < rowCount else { return .missing }
        let index = row / pageSize
        let offset = row % pageSize
        if var page = pages[index], offset < page.rows.count {
            clock += 1
            page.lastUse = clock
            pages[index] = page
            return .fresh(page.rows[offset])
        }
        if let stale = stalePages[index], offset < stale.count {
            return .stale(stale[offset])
        }
        return .missing
    }

    /// LRU를 갱신하지 않는 fresh 조회.
    func peek(_ row: Int) -> Row? {
        guard row >= 0, row < rowCount else { return nil }
        guard let page = pages[row / pageSize], row % pageSize < page.rows.count else { return nil }
        return page.rows[row % pageSize]
    }

    /// fresh가 없으면 stale 값까지 보는 조회(LRU 갱신 없음). 새 페이지가 올 때까지 표시를 유지할 때 씁니다.
    func peekDisplayed(_ row: Int) -> Row? {
        if let fresh = peek(row) { return fresh }
        guard row >= 0, row < rowCount, let stale = stalePages[row / pageSize], row % pageSize < stale.count else {
            return nil
        }
        return stale[row % pageSize]
    }

    /// 행이 속한 페이지를 아직 갖지도 요청하지도 않았다면 요청 범위를 반환하고 요청 중으로 표시합니다.
    mutating func rangeToRequest(forRow row: Int) -> Range<Int>? {
        guard row >= 0, row < rowCount else { return nil }
        let index = row / pageSize
        guard pages[index] == nil, !inFlight.contains(index) else { return nil }
        inFlight.insert(index)
        requestCount += 1
        let start = index * pageSize
        return start..<min(start + pageSize, rowCount)
    }

    /// 도착한 페이지를 저장합니다. 세대가 다르거나 경계가 맞지 않으면 버리고 nil을 반환합니다.
    mutating func store(generation: UInt64, startRow: Int, rows: [Row]) -> Range<Int>? {
        guard generation == self.generation, startRow >= 0, startRow % pageSize == 0, startRow < rowCount else {
            return nil
        }
        let index = startRow / pageSize
        inFlight.remove(index)
        clock += 1
        pages[index] = Page(rows: rows, lastUse: clock)
        stalePages[index] = nil
        while pages.count > capacity {
            let victim = pages.filter { $0.key != index }.min { $0.value.lastUse < $1.value.lastUse }?.key
            guard let victim else { break }
            pages[victim] = nil
        }
        return startRow..<(startRow + rows.count)
    }

    /// 코어가 거부한 요청의 요청 중 표시를 지워 다시 요청할 수 있게 합니다.
    mutating func cancelRequest(generation: UInt64, rows: Range<Int>) {
        guard generation == self.generation, !rows.isEmpty else { return }
        inFlight.remove(rows.lowerBound / pageSize)
    }
}

// MARK: - 부모별 자식 페이지 캐시

/// 변경 계층용 페이지 캐시(XIV.13.1). 부모 노드마다 자식 순번을 페이지로 나누어 받습니다.
///
/// - 구조 번호가 바뀌면 모든 페이지를 버립니다(같은 순번이 다른 노드일 수 있음).
/// - 구조가 같고 세대만 바뀌면 직전 페이지를 새 페이지가 올 때까지 stale 대체값으로 남겨 깜빡임을 막습니다.
///   stale 행은 조작할 수 없습니다.
/// - 메모리 상한은 모든 부모의 페이지를 합친 수이며, 넘으면 가장 오래 쓰지 않은 페이지부터 버립니다.
struct GitChangeTreeCache {
    enum Lookup {
        case fresh(GitChangeRow)
        case stale(GitChangeRow)
        case missing
    }

    struct PageKey: Hashable {
        var parent: GitChangeNodeID?
        var page: Int
    }

    private struct Page {
        var rows: [GitChangeRow]
        var lastUse: UInt64
    }

    let pageSize: Int
    let capacity: Int
    private(set) var generation: UInt64 = 0
    private(set) var structure: UInt64 = 0
    /// 최상위(문서) 행 수.
    private(set) var rootCount = 0
    private var pages: [PageKey: Page] = [:]
    private var stalePages: [PageKey: [GitChangeRow]] = [:]
    private var inFlight: Set<PageKey> = []
    private var clock: UInt64 = 0
    /// 진단: 캐시 생성 이후 요청한 페이지 수 누계.
    private(set) var requestCount = 0

    init(pageSize: Int = GitPanelMetrics.pageSize, capacity: Int = GitPanelMetrics.pageCapacity) {
        self.pageSize = max(1, pageSize)
        self.capacity = max(1, capacity)
    }

    var cachedPageCount: Int { pages.count }

    /// 구조가 바뀌었습니다. 모든 페이지와 요청 중 표시를 버립니다.
    mutating func reset(generation: UInt64, structure: UInt64, rootCount: Int) {
        self.generation = generation
        self.structure = structure
        self.rootCount = max(0, rootCount)
        pages = [:]
        stalePages = [:]
        inFlight = []
    }

    /// 구조는 같고 값만 바뀌었습니다. 현재 페이지를 stale로 돌립니다.
    mutating func advance(generation: UInt64) {
        guard generation != self.generation else { return }
        for (key, page) in pages { stalePages[key] = page.rows }
        self.generation = generation
        pages = [:]
        inFlight = []
    }

    private func key(_ parent: GitChangeNodeID?, _ index: Int) -> (PageKey, Int)? {
        guard index >= 0 else { return nil }
        return (PageKey(parent: parent, page: index / pageSize), index % pageSize)
    }

    /// 행을 찾습니다. fresh 조회는 LRU 시각을 갱신합니다.
    mutating func lookup(parent: GitChangeNodeID?, index: Int) -> Lookup {
        guard let (key, offset) = key(parent, index) else { return .missing }
        if var page = pages[key], offset < page.rows.count {
            clock += 1
            page.lastUse = clock
            pages[key] = page
            return .fresh(page.rows[offset])
        }
        if let stale = stalePages[key], offset < stale.count {
            return .stale(stale[offset])
        }
        return .missing
    }

    /// LRU를 갱신하지 않는 fresh 조회.
    func peek(parent: GitChangeNodeID?, index: Int) -> GitChangeRow? {
        guard let (key, offset) = key(parent, index), let page = pages[key], offset < page.rows.count else { return nil }
        return page.rows[offset]
    }

    /// fresh가 없으면 stale 값까지 보는 조회(LRU 갱신 없음).
    func peekDisplayed(parent: GitChangeNodeID?, index: Int) -> GitChangeRow? {
        if let fresh = peek(parent: parent, index: index) { return fresh }
        guard let (key, offset) = key(parent, index), let stale = stalePages[key], offset < stale.count else {
            return nil
        }
        return stale[offset]
    }

    /// 행이 속한 페이지를 아직 갖지도 요청하지도 않았다면 요청 범위를 반환하고 요청 중으로 표시합니다.
    /// `childCount`는 그 부모의 자식 수입니다(페이지 끝을 자름).
    mutating func rangeToRequest(parent: GitChangeNodeID?, index: Int, childCount: Int) -> Range<Int>? {
        guard index >= 0, index < childCount, let (key, _) = key(parent, index) else { return nil }
        guard pages[key] == nil, !inFlight.contains(key) else { return nil }
        inFlight.insert(key)
        requestCount += 1
        let start = key.page * pageSize
        return start..<min(start + pageSize, childCount)
    }

    /// 도착한 페이지를 저장합니다. 세대가 다르거나 경계가 맞지 않으면 버리고 nil을 반환합니다.
    mutating func store(_ page: GitChangePage) -> Range<Int>? {
        guard page.generation == generation, page.startIndex >= 0, page.startIndex % pageSize == 0 else { return nil }
        let key = PageKey(parent: page.parent, page: page.startIndex / pageSize)
        inFlight.remove(key)
        clock += 1
        pages[key] = Page(rows: page.rows, lastUse: clock)
        stalePages[key] = nil
        while pages.count > capacity {
            let victim = pages.filter { $0.key != key }.min { $0.value.lastUse < $1.value.lastUse }?.key
            guard let victim else { break }
            pages[victim] = nil
        }
        return page.startIndex..<(page.startIndex + page.rows.count)
    }

    /// 코어가 거부한 요청의 요청 중 표시를 지워 다시 요청할 수 있게 합니다.
    mutating func cancelRequest(generation: UInt64, parent: GitChangeNodeID?, children: Range<Int>) {
        guard generation == self.generation, let (key, _) = key(parent, children.lowerBound) else { return }
        inFlight.remove(key)
    }
}

// MARK: - 세로 배치

/// 자식을 위에서 아래로 쌓고 폭을 채우는 단순 컨테이너입니다.
///
/// 숨긴 자식은 `isHidden`으로 감춰 접근성 트리에서도 빠지게 하고(디자인 가이드 8.3), 보이는 자식만
/// 위아래로 잇습니다. 숨긴 자식은 계층에서 떼지 않고 위쪽에 고정해 두어 제약 엔진 비용과 모호한
/// Auto Layout을 함께 피합니다. `flexibleView`가 보이면 남는 높이를 그 뷰가 가져갑니다.
final class GitColumnView: NSView {
    var spacing: CGFloat
    var insets: NSEdgeInsets
    private var items: [NSView] = []
    private var hiddenItems: Set<ObjectIdentifier> = []
    private var managedConstraints: [NSLayoutConstraint] = []

    weak var flexibleView: NSView? {
        didSet { setNeedsRebuild() }
    }
    private var batchDepth = 0
    private var needsRebuild = false

    init(spacing: CGFloat = GitPanelMetrics.rowSpacing, insets: NSEdgeInsets = NSEdgeInsets()) {
        self.spacing = spacing
        self.insets = insets
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("GitColumnView는 코드로만 생성합니다")
    }

    func setItems(_ views: [NSView]) {
        items.forEach { $0.removeFromSuperview() }
        items = views
        hiddenItems = []
        setNeedsRebuild()
    }

    func setItem(_ view: NSView, visible: Bool) {
        let key = ObjectIdentifier(view)
        guard visible == hiddenItems.contains(key) else { return }
        if visible { hiddenItems.remove(key) } else { hiddenItems.insert(key) }
        setNeedsRebuild()
    }

    /// 여러 표시 변경을 묶어 제약을 한 번만 다시 만듭니다(메인 스레드 점유 축소, 아키텍처 §6.3).
    func batchUpdates(_ body: () -> Void) {
        batchDepth += 1
        body()
        batchDepth -= 1
        if batchDepth == 0, needsRebuild {
            needsRebuild = false
            rebuild()
        }
    }

    private func setNeedsRebuild() {
        if batchDepth > 0 {
            needsRebuild = true
        } else {
            rebuild()
        }
    }

    var hasVisibleItems: Bool { items.contains { !hiddenItems.contains(ObjectIdentifier($0)) } }

    private func rebuild() {
        NSLayoutConstraint.deactivate(managedConstraints)
        managedConstraints = []
        let visible = items.filter { !hiddenItems.contains(ObjectIdentifier($0)) }
        for view in items {
            view.translatesAutoresizingMaskIntoConstraints = false
            if view.superview !== self { addSubview(view) }
            let hidden = hiddenItems.contains(ObjectIdentifier(view))
            if view.isHidden != hidden { view.isHidden = hidden }
            if hidden {
                managedConstraints += [
                    view.topAnchor.constraint(equalTo: topAnchor, constant: insets.top),
                    view.leadingAnchor.constraint(equalTo: leadingAnchor, constant: insets.left),
                    view.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -insets.right),
                ]
            }
        }
        var previous = topAnchor
        var gap = insets.top
        for view in visible {
            managedConstraints += [
                view.topAnchor.constraint(equalTo: previous, constant: gap),
                view.leadingAnchor.constraint(equalTo: leadingAnchor, constant: insets.left),
                view.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -insets.right),
            ]
            previous = view.bottomAnchor
            gap = spacing
        }
        if let last = visible.last {
            let fills = flexibleView.map { flexible in visible.contains { $0 === flexible } } ?? false
            let bottom = fills
                ? last.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -insets.bottom)
                : last.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor, constant: -insets.bottom)
            managedConstraints.append(bottom)
            if !fills {
                // 남는 높이가 있으면 위쪽에 붙입니다. 내용이 커지면 컨테이너가 늘어나게 둡니다.
                let hug = last.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -insets.bottom)
                hug.priority = .defaultLow
                managedConstraints.append(hug)
            }
        } else {
            let height = heightAnchor.constraint(equalToConstant: 0)
            height.priority = .defaultLow
            managedConstraints.append(height)
        }
        NSLayoutConstraint.activate(managedConstraints)
    }
}

// MARK: - 변경 없는 갱신 생략

extension NSTextField {
    /// 값이 같으면 다시 설정하지 않아 불필요한 크기 재계산을 피합니다.
    func setStringIfChanged(_ value: String) {
        if stringValue != value { stringValue = value }
    }
}

// MARK: - 알림 구독 토큰

/// 해제될 때 알림 구독을 끊는 토큰. view controller의 deinit 격리 문제를 피합니다.
final class GitNotificationToken {
    private let center: NotificationCenter
    private let token: NSObjectProtocol

    init(center: NotificationCenter, token: NSObjectProtocol) {
        self.center = center
        self.token = token
    }

    deinit {
        center.removeObserver(token)
    }
}

// MARK: - 접근성 표시 옵션

/// 시스템 접근성 표시 옵션(디자인 가이드 8.4).
struct GitDisplayOptions: Equatable {
    var reduceMotion: Bool
    var increaseContrast: Bool
    var reduceTransparency: Bool

    static func current() -> GitDisplayOptions {
        let workspace = NSWorkspace.shared
        return GitDisplayOptions(
            reduceMotion: workspace.accessibilityDisplayShouldReduceMotion,
            increaseContrast: workspace.accessibilityDisplayShouldIncreaseContrast,
            reduceTransparency: workspace.accessibilityDisplayShouldReduceTransparency
        )
    }

    /// 보조 문구 색. 대비 높이기에서는 label 색으로 올립니다.
    var secondaryTextColor: NSColor { increaseContrast ? .labelColor : .secondaryLabelColor }

    /// 상태 표시 전환 시간. 동작 줄이기에서는 즉시 전환합니다.
    var revealDuration: TimeInterval { reduceMotion ? 0 : GitPanelMetrics.revealDuration }
}

// MARK: - 컨트롤 팩토리

@MainActor
enum GitControls {
    /// 한 줄, 끝 말줄임 label.
    static func label(
        _ text: String = "",
        style: NSFont.TextStyle = .body,
        color: NSColor = .labelColor,
        truncation: NSLineBreakMode = .byTruncatingTail
    ) -> NSTextField {
        let field = NSTextField(labelWithString: text)
        field.font = NSFont.preferredFont(forTextStyle: style)
        field.textColor = color
        field.lineBreakMode = truncation
        field.maximumNumberOfLines = 1
        field.cell?.truncatesLastVisibleLine = true
        field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return field
    }

    /// 여러 줄로 감기는 label.
    static func wrappingLabel(
        _ text: String = "",
        style: NSFont.TextStyle = .callout,
        color: NSColor = .secondaryLabelColor
    ) -> NSTextField {
        let field = NSTextField(wrappingLabelWithString: text)
        field.font = NSFont.preferredFont(forTextStyle: style)
        field.textColor = color
        field.isSelectable = false
        field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return field
    }

    /// 아이콘만 있는 버튼. 도움말 태그와 접근성 레이블을 함께 지정합니다(디자인 가이드 8.1, 8.3).
    static func iconButton(symbol: String, label: String, target: AnyObject?, action: Selector) -> NSButton {
        let image = GitPanelSymbols.image(symbol, label: label) ?? NSImage()
        let button = NSButton(image: image, target: target, action: action)
        button.bezelStyle = .accessoryBarAction
        button.isBordered = false
        button.imagePosition = .imageOnly
        button.toolTip = label
        button.setAccessibilityLabel(label)
        return button
    }

    /// 시스템 심볼을 표시하는 image view.
    static func symbolView(_ symbol: String, label: String, style: NSFont.TextStyle = .body) -> NSImageView {
        let view = NSImageView()
        view.image = GitPanelSymbols.image(symbol, label: label)
        view.symbolConfiguration = NSImage.SymbolConfiguration(textStyle: style)
        view.contentTintColor = .secondaryLabelColor
        view.setAccessibilityLabel(label)
        view.setContentHuggingPriority(.required, for: .horizontal)
        view.setContentCompressionResistancePriority(.required, for: .horizontal)
        return view
    }

    /// 구분선.
    static func separator() -> NSBox {
        let box = NSBox()
        box.boxType = .separator
        return box
    }

    /// 버튼을 앞쪽에 모으는 가로 줄. 빈 공간은 gravity 영역 사이에 둡니다.
    static func leadingRow(_ views: [NSView], trailing: [NSView] = []) -> NSStackView {
        let row = NSStackView()
        row.orientation = .horizontal
        row.distribution = .gravityAreas
        row.alignment = .centerY
        row.spacing = GitPanelMetrics.inlineSpacing
        views.forEach { row.addView($0, in: .leading) }
        trailing.forEach { row.addView($0, in: .trailing) }
        return row
    }
}

// MARK: - 상태 배너

/// 패널 상단의 상태 안내 배너. 배경을 직접 칠하지 않고 아이콘·문구·버튼만 둡니다(디자인 가이드 3.2).
final class GitBannerView: NSView {
    let iconView: NSImageView
    let spinner = NSProgressIndicator()
    let messageField = GitControls.wrappingLabel(style: .callout, color: .labelColor)
    let buttonRow = GitControls.leadingRow([])
    private var buttons: [NSButton] = []
    private var bottomToMessage: NSLayoutConstraint!
    private var bottomToButtons: NSLayoutConstraint!

    init(symbol: String, symbolLabel: String, tint: NSColor) {
        iconView = GitControls.symbolView(symbol, label: symbolLabel, style: .callout)
        iconView.contentTintColor = tint
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        spinner.style = .spinning
        spinner.controlSize = .small
        spinner.isIndeterminate = true
        spinner.isDisplayedWhenStopped = false
        spinner.setAccessibilityLabel(GitPanelStrings.loading)

        for view in [iconView, spinner, messageField, buttonRow] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        bottomToMessage = messageField.bottomAnchor.constraint(equalTo: bottomAnchor)
        bottomToButtons = buttonRow.bottomAnchor.constraint(equalTo: bottomAnchor)
        NSLayoutConstraint.activate([
            iconView.leadingAnchor.constraint(equalTo: leadingAnchor),
            iconView.firstBaselineAnchor.constraint(equalTo: messageField.firstBaselineAnchor),
            spinner.centerXAnchor.constraint(equalTo: iconView.centerXAnchor),
            spinner.centerYAnchor.constraint(equalTo: iconView.centerYAnchor),
            messageField.topAnchor.constraint(equalTo: topAnchor),
            messageField.leadingAnchor.constraint(equalTo: iconView.trailingAnchor, constant: GitPanelMetrics.inlineSpacing * 2),
            messageField.trailingAnchor.constraint(equalTo: trailingAnchor),
            buttonRow.topAnchor.constraint(equalTo: messageField.bottomAnchor, constant: GitPanelMetrics.rowSpacing),
            buttonRow.leadingAnchor.constraint(equalTo: messageField.leadingAnchor),
            buttonRow.trailingAnchor.constraint(equalTo: trailingAnchor),
            bottomToMessage,
        ])
        buttonRow.isHidden = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("GitBannerView는 코드로만 생성합니다")
    }

    /// 아이콘과 색을 바꿉니다. 상태는 아이콘 모양과 문구로도 전달합니다(디자인 가이드 4.2).
    func setSymbol(_ symbol: String, label: String, tint: NSColor) {
        iconView.image = GitPanelSymbols.image(symbol, label: label)
        iconView.setAccessibilityLabel(label)
        iconView.contentTintColor = tint
    }

    /// 문구와 버튼을 바꿉니다. `busy`이면 아이콘 자리에 시스템 진행 표시를 보여 줍니다.
    func configure(message: String, buttons newButtons: [NSButton] = [], busy: Bool = false) {
        messageField.setStringIfChanged(message)
        if buttons.map(ObjectIdentifier.init) != newButtons.map(ObjectIdentifier.init) {
            buttons.forEach { buttonRow.removeView($0) }
            buttons = newButtons
            newButtons.forEach {
                $0.controlSize = .small
                buttonRow.addView($0, in: .leading)
            }
        }
        let hasButtons = !newButtons.isEmpty
        buttonRow.isHidden = !hasButtons
        bottomToMessage.isActive = !hasButtons
        bottomToButtons.isActive = hasButtons
        iconView.isHidden = busy
        if busy { spinner.startAnimation(nil) } else { spinner.stopAnimation(nil) }
        setAccessibilityElement(false)
    }
}

// MARK: - 빈 상태·로딩 오버레이

/// 목록 자리에 겹쳐 보여 주는 빈 상태·로딩·사용 불가 안내(디자인 가이드 7.1).
final class GitStateOverlayView: NSView {
    let iconView = GitControls.symbolView(GitPanelSymbols.clean, label: "", style: .title2)
    let spinner = NSProgressIndicator()
    let titleField = GitControls.label(style: .headline, truncation: .byTruncatingTail)
    let messageField = GitControls.wrappingLabel(style: .callout)
    let actionButton = NSButton(title: "", target: nil, action: nil)
    private let stack = NSStackView()

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false
        spinner.style = .spinning
        spinner.controlSize = .regular
        spinner.isIndeterminate = true
        spinner.isDisplayedWhenStopped = false
        spinner.setAccessibilityLabel(GitPanelStrings.loading)
        titleField.alignment = .center
        messageField.alignment = .center
        actionButton.bezelStyle = .push
        actionButton.controlSize = .regular

        stack.orientation = .vertical
        stack.alignment = .centerX
        stack.spacing = GitPanelMetrics.rowSpacing
        stack.translatesAutoresizingMaskIntoConstraints = false
        [iconView, spinner, titleField, messageField, actionButton].forEach { stack.addArrangedSubview($0) }
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.centerXAnchor.constraint(equalTo: centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor),
            stack.leadingAnchor.constraint(greaterThanOrEqualTo: leadingAnchor, constant: GitPanelMetrics.outerInset),
            stack.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -GitPanelMetrics.outerInset),
            stack.topAnchor.constraint(greaterThanOrEqualTo: topAnchor),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor),
            messageField.widthAnchor.constraint(lessThanOrEqualTo: widthAnchor, constant: -GitPanelMetrics.outerInset * 2),
        ])
        let preferredWidth = messageField.widthAnchor.constraint(equalTo: widthAnchor, constant: -GitPanelMetrics.outerInset * 2)
        preferredWidth.priority = .defaultLow
        preferredWidth.isActive = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("GitStateOverlayView는 코드로만 생성합니다")
    }

    /// 로딩 표시.
    func showLoading(_ message: String) {
        iconView.isHidden = true
        spinner.isHidden = false
        spinner.startAnimation(nil)
        titleField.isHidden = true
        messageField.stringValue = message
        messageField.isHidden = false
        actionButton.isHidden = true
    }

    /// 빈 상태·안내 표시. 다음 동작 버튼을 함께 둡니다(디자인 가이드 7.1).
    func showMessage(
        symbol: String,
        title: String,
        message: String,
        action: String?,
        target: AnyObject?,
        selector: Selector?
    ) {
        spinner.stopAnimation(nil)
        spinner.isHidden = true
        iconView.isHidden = false
        iconView.image = GitPanelSymbols.image(symbol, label: title)
        iconView.setAccessibilityLabel(title)
        titleField.isHidden = false
        titleField.stringValue = title
        messageField.isHidden = false
        messageField.stringValue = message
        if let action, let selector {
            actionButton.isHidden = false
            actionButton.title = action
            actionButton.target = target
            actionButton.action = selector
        } else {
            actionButton.isHidden = true
        }
    }
}

// MARK: - 상세 행

/// 상세 요약의 "이름: 값" 한 줄. 값은 선택·복사할 수 있고 잘리면 도움말 태그로 전체를 보여 줍니다.
final class GitDetailRowView: NSView {
    let nameField: NSTextField
    let valueField: NSTextField

    init(name: String, nameWidth: CGFloat, monospaced: Bool = false) {
        nameField = GitControls.label(name, style: .callout, color: .secondaryLabelColor)
        valueField = GitControls.label(style: .callout, truncation: .byTruncatingMiddle)
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        nameField.alignment = .right
        nameField.setContentCompressionResistancePriority(.required, for: .horizontal)
        valueField.isSelectable = true
        if monospaced, let size = valueField.font?.pointSize {
            valueField.font = NSFont.monospacedSystemFont(ofSize: size, weight: .regular)
        }
        valueField.setAccessibilityLabel(name)
        for field in [nameField, valueField] {
            field.translatesAutoresizingMaskIntoConstraints = false
            addSubview(field)
        }
        NSLayoutConstraint.activate([
            nameField.leadingAnchor.constraint(equalTo: leadingAnchor),
            nameField.widthAnchor.constraint(equalToConstant: nameWidth),
            nameField.topAnchor.constraint(equalTo: topAnchor),
            nameField.bottomAnchor.constraint(equalTo: bottomAnchor),
            valueField.leadingAnchor.constraint(equalTo: nameField.trailingAnchor, constant: GitPanelMetrics.inlineSpacing * 2),
            valueField.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor),
            valueField.firstBaselineAnchor.constraint(equalTo: nameField.firstBaselineAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("GitDetailRowView는 코드로만 생성합니다")
    }

    func setValue(_ value: String) {
        valueField.setStringIfChanged(value)
        if valueField.toolTip != value { valueField.toolTip = value }
    }

    /// 여러 이름 중 가장 넓은 폭(측정값). 고정 수치를 쓰지 않고 실제 글꼴로 잽니다.
    static func nameColumnWidth(for names: [String]) -> CGFloat {
        let probe = GitControls.label(style: .callout)
        return names.reduce(0) { width, name in
            probe.stringValue = name
            return max(width, ceil(probe.intrinsicContentSize.width))
        }
    }
}
