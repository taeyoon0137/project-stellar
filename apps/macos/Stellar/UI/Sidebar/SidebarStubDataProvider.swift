import Foundation

/// 코어 연결 전까지 쓰는 의도된 placeholder provider(계획 규칙: placeholder 이름에 "Stub").
///
/// 메모리 안의 작은 트리로 command를 즉시 적용하고 새 snapshot을 냅니다. 코어 의미(OrderKey,
/// Undo, 저장, 실제 검증)는 흉내 내지 않으며, 이동 검증은 `ReparentError` 규칙만 따라 합니다.
/// 검사와 개발 확인용이므로 성능 계약 대상이 아닙니다.
@MainActor
final class SidebarStubDataProvider: SidebarDataProvider {
    private(set) var snapshot: SidebarSnapshot = .empty
    var snapshotDidChange: (() -> Void)?

    /// 받은 command 기록(검사·디버깅용).
    private(set) var performedCommands: [SidebarCommand] = []

    /// `true`이면 펼친 부모의 자식만 snapshot에 넣어 페이지 단위 로드 경로를 흉내 냅니다.
    let loadsChildrenLazily: Bool

    private struct Node {
        let id: SidebarStableID
        let viewID: SidebarStableID
        var parentID: SidebarStableID?
        let kind: SidebarNodeKind
        let shapeKind: SidebarShapeKind?
        var name: String
        var isVisible = true
        var isLocked = false
        var children: [SidebarStableID] = []
    }

    private struct Page {
        let id: SidebarStableID
        var name: String
        var views: [(id: SidebarStableID, name: String)]
    }

    private var pages: [Page] = []
    private var nodes: [SidebarStableID: Node] = [:]
    /// View별 최상위 행(root Frame 직속), 표시 순서.
    private var topLevel: [SidebarStableID: [SidebarStableID]] = [:]
    private var activePageID: SidebarStableID?
    private var activeViewID: SidebarStableID?
    private var selection: [SidebarStableID] = []
    private var primarySelection: SidebarStableID?
    private var filterQuery = ""
    private var loadState: SidebarLoadState = .ready
    private var loadedParents: Set<SidebarStableID> = []
    private var generation: UInt64 = 0
    private var structureRevision: UInt64 = 1
    private var serial: UInt64 = 0

    init(loadsChildrenLazily: Bool = false) {
        self.loadsChildrenLazily = loadsChildrenLazily
        buildSample()
        rebuildSnapshot(notify: false)
    }

    // MARK: 검사 보조

    /// active View에서 이름이 같은 첫 노드(검사용).
    func nodeID(named name: String) -> SidebarStableID? {
        guard let viewID = activeViewID else { return nil }
        var stack = Array((topLevel[viewID] ?? []).reversed())
        while let id = stack.popLast() {
            guard let node = nodes[id] else { continue }
            if node.name == name { return id }
            stack.append(contentsOf: node.children.reversed())
        }
        return nil
    }

    /// 이름이 같은 첫 View(검사용).
    func viewID(named name: String) -> SidebarStableID? {
        pages.flatMap(\.views).first { $0.name == name }?.id
    }

    /// 로드 상태를 바꿔 loading·failed 화면을 확인합니다(검사용).
    func simulateLoadState(_ state: SidebarLoadState) {
        loadState = state
        commit(structural: true)
    }

    // MARK: SidebarDataProvider

    func validate(_ command: SidebarCommand) -> SidebarCommandValidation {
        switch command {
        case .activatePage(let id):
            return pages.contains { $0.id == id } ? .allowed : .denied(.unknownNode)
        case .activateView(let id), .renameView(let id, _):
            return pages.contains { $0.views.contains { $0.id == id } } ? .allowed : .denied(.unknownNode)
        case .renamePage(let id, _):
            return pages.contains { $0.id == id } ? .allowed : .denied(.unknownNode)
        case .setSelection, .setFilter, .reload:
            return .allowed
        case .loadChildren(let id), .renameNode(let id, _):
            return nodes[id] == nil ? .denied(.unknownNode) : .allowed
        case .setVisible(let ids, _), .setLocked(let ids, _), .duplicate(let ids), .delete(let ids):
            return validateExisting(ids)
        case .group(let ids):
            if case .denied(let reason) = validateExisting(ids) { return .denied(reason) }
            let parents = Set(ids.map { nodes[$0]?.parentID })
            return parents.count == 1 ? .allowed : .denied(.mixedParents)
        case .ungroup(let ids):
            if case .denied(let reason) = validateExisting(ids) { return .denied(reason) }
            let hasGroup = ids.contains { nodes[$0]?.kind == .fragment && !(nodes[$0]?.children.isEmpty ?? true) }
            return hasGroup ? .allowed : .denied(.notGroup)
        case .move(let ids, let target):
            return validateMove(ids, to: target)
        }
    }

    func perform(_ command: SidebarCommand) {
        performedCommands.append(command)
        guard validate(command).isAllowed else { return }
        switch command {
        case .activatePage(let id):
            activePageID = id
            activeViewID = pages.first { $0.id == id }?.views.first?.id
            clearSelection()
            commit(structural: true)
        case .activateView(let id):
            activeViewID = id
            activePageID = pages.first { $0.views.contains { $0.id == id } }?.id
            clearSelection()
            commit(structural: true)
        case .renamePage(let id, let name):
            if let index = pages.firstIndex(where: { $0.id == id }) { pages[index].name = name }
            commit(structural: false)
        case .renameView(let id, let name):
            for pageIndex in pages.indices {
                if let viewIndex = pages[pageIndex].views.firstIndex(where: { $0.id == id }) {
                    pages[pageIndex].views[viewIndex].name = name
                }
            }
            commit(structural: false)
        case .setSelection(let ids, let primary):
            selection = ids.filter { nodes[$0]?.viewID == activeViewID }
            primarySelection = primary.flatMap { selection.contains($0) ? $0 : nil } ?? selection.last
            commit(structural: false)
        case .renameNode(let id, let name):
            nodes[id]?.name = name
            // 필터가 켜져 있으면 이름 변경이 일치 집합을 바꿀 수 있습니다.
            commit(structural: !filterQuery.isEmpty)
        case .setVisible(let ids, let visible):
            for id in ids { nodes[id]?.isVisible = visible }
            commit(structural: false)
        case .setLocked(let ids, let locked):
            for id in ids { nodes[id]?.isLocked = locked }
            commit(structural: false)
        case .duplicate(let ids):
            var copies: [SidebarStableID] = []
            for id in topMost(ids) {
                guard let node = nodes[id] else { continue }
                let copy = cloneSubtree(id, parentID: node.parentID, isRoot: true)
                insert(copy, into: node.parentID, viewID: node.viewID, after: id)
                copies.append(copy)
            }
            setSelection(copies)
            commit(structural: true)
        case .delete(let ids):
            for id in topMost(ids) { removeSubtree(id) }
            setSelection(selection.filter { nodes[$0] != nil })
            commit(structural: true)
        case .group(let ids):
            group(ids)
            commit(structural: true)
        case .ungroup(let ids):
            ungroup(ids)
            commit(structural: true)
        case .move(let ids, let target):
            move(ids, to: target)
            commit(structural: true)
        case .setFilter(let query):
            filterQuery = query.trimmingCharacters(in: .whitespacesAndNewlines)
            commit(structural: true)
        case .loadChildren(let id):
            loadedParents.insert(id)
            commit(structural: true)
        case .reload:
            loadState = .ready
            commit(structural: true)
        }
    }

    // MARK: 검증

    private func validateExisting(_ ids: [SidebarStableID]) -> SidebarCommandValidation {
        if ids.isEmpty { return .denied(.emptyTarget) }
        return ids.allSatisfy { nodes[$0] != nil } ? .allowed : .denied(.unknownNode)
    }

    private func validateMove(_ ids: [SidebarStableID], to target: SidebarDropTarget) -> SidebarCommandValidation {
        if ids.isEmpty { return .denied(.emptyTarget) }
        if !filterQuery.isEmpty { return .denied(.filterActive) }
        guard let viewID = activeViewID else { return .denied(.unsupported) }
        for id in ids {
            guard let node = nodes[id] else { return .denied(.unknownNode) }
            if node.viewID != viewID { return .denied(.crossViewMove) }
        }
        let siblingCount: Int
        if let parentID = target.parentID {
            guard let parent = nodes[parentID] else { return .denied(.unknownNode) }
            if parent.viewID != viewID { return .denied(.crossViewMove) }
            if ids.contains(parentID) { return .denied(.selfParent) }
            if ids.contains(where: { isAncestor($0, of: parentID) }) { return .denied(.descendantParent) }
            if !Self.isContainer(parent.kind) { return .denied(.notContainer) }
            siblingCount = parent.children.count
        } else {
            siblingCount = topLevel[viewID]?.count ?? 0
        }
        return (0...siblingCount).contains(target.index) ? .allowed : .denied(.unsupported)
    }

    /// 자식을 가질 수 있는 종류(Stub 규칙). 실제 규칙은 코어 Capability가 정합니다(아키텍처 §12.3, §12.4).
    private static func isContainer(_ kind: SidebarNodeKind) -> Bool {
        kind == .frame || kind == .fragment
    }

    private func isAncestor(_ ancestor: SidebarStableID, of id: SidebarStableID) -> Bool {
        var current = nodes[id]?.parentID
        while let node = current {
            if node == ancestor { return true }
            current = nodes[node]?.parentID
        }
        return false
    }

    /// 조상이 함께 지정된 항목을 뺍니다(부모와 자식을 함께 옮기거나 지울 때 자식은 부모를 따라감).
    private func topMost(_ ids: [SidebarStableID]) -> [SidebarStableID] {
        let set = Set(ids)
        return ids.filter { id in !set.contains { $0 != id && isAncestor($0, of: id) } }
    }

    // MARK: 변경

    private func clearSelection() {
        selection = []
        primarySelection = nil
    }

    private func setSelection(_ ids: [SidebarStableID]) {
        selection = ids
        primarySelection = ids.last
    }

    private func siblings(of parentID: SidebarStableID?, viewID: SidebarStableID) -> [SidebarStableID] {
        if let parentID { return nodes[parentID]?.children ?? [] }
        return topLevel[viewID] ?? []
    }

    private func setSiblings(_ list: [SidebarStableID], of parentID: SidebarStableID?, viewID: SidebarStableID) {
        if let parentID {
            nodes[parentID]?.children = list
        } else {
            topLevel[viewID] = list
        }
    }

    private func insert(
        _ id: SidebarStableID,
        into parentID: SidebarStableID?,
        viewID: SidebarStableID,
        after sibling: SidebarStableID
    ) {
        var list = siblings(of: parentID, viewID: viewID)
        let index = list.firstIndex(of: sibling).map { $0 + 1 } ?? list.count
        list.insert(id, at: index)
        setSiblings(list, of: parentID, viewID: viewID)
    }

    private func cloneSubtree(_ id: SidebarStableID, parentID: SidebarStableID?, isRoot: Bool) -> SidebarStableID {
        guard let source = nodes[id] else { return id }
        let copyID = makeID()
        nodes[copyID] = Node(
            id: copyID,
            viewID: source.viewID,
            parentID: parentID,
            kind: source.kind,
            shapeKind: source.shapeKind,
            name: isRoot && !source.name.isEmpty ? source.name + " copy" : source.name,
            isVisible: source.isVisible,
            isLocked: source.isLocked
        )
        // 하위 복제가 nodes를 바꾸므로 결과를 먼저 모은 뒤 대입합니다(배타 접근).
        let copiedChildren = source.children.map { cloneSubtree($0, parentID: copyID, isRoot: false) }
        nodes[copyID]?.children = copiedChildren
        return copyID
    }

    private func removeSubtree(_ id: SidebarStableID) {
        guard let node = nodes[id] else { return }
        for child in node.children { removeSubtree(child) }
        var list = siblings(of: node.parentID, viewID: node.viewID)
        list.removeAll { $0 == id }
        setSiblings(list, of: node.parentID, viewID: node.viewID)
        nodes[id] = nil
        loadedParents.remove(id)
    }

    private func group(_ ids: [SidebarStableID]) {
        guard let first = ids.first, let anchor = nodes[first] else { return }
        let parentID = anchor.parentID
        let viewID = anchor.viewID
        var list = siblings(of: parentID, viewID: viewID)
        let members = list.filter { ids.contains($0) }
        guard let insertIndex = list.firstIndex(where: { ids.contains($0) }) else { return }
        list.removeAll { ids.contains($0) }
        let groupID = makeID()
        nodes[groupID] = Node(
            id: groupID,
            viewID: viewID,
            parentID: parentID,
            kind: .fragment,
            shapeKind: nil,
            name: "Group",
            children: members
        )
        for member in members { nodes[member]?.parentID = groupID }
        list.insert(groupID, at: insertIndex)
        setSiblings(list, of: parentID, viewID: viewID)
        loadedParents.insert(groupID)
        setSelection([groupID])
    }

    private func ungroup(_ ids: [SidebarStableID]) {
        var released: [SidebarStableID] = []
        for id in ids {
            guard let node = nodes[id], node.kind == .fragment, !node.children.isEmpty else { continue }
            var list = siblings(of: node.parentID, viewID: node.viewID)
            guard let index = list.firstIndex(of: id) else { continue }
            list.remove(at: index)
            list.insert(contentsOf: node.children, at: index)
            for child in node.children { nodes[child]?.parentID = node.parentID }
            setSiblings(list, of: node.parentID, viewID: node.viewID)
            released.append(contentsOf: node.children)
            nodes[id] = nil
            loadedParents.remove(id)
        }
        setSelection(released)
    }

    private func move(_ ids: [SidebarStableID], to target: SidebarDropTarget) {
        guard let viewID = activeViewID else { return }
        let moving = topMost(ids)
        let targetList = siblings(of: target.parentID, viewID: viewID)
        // 이동 대상 제거 전 index를 제거 후 index로 보정합니다.
        let removedBefore = targetList.prefix(target.index).filter { moving.contains($0) }.count
        for id in moving {
            guard let node = nodes[id] else { continue }
            var list = siblings(of: node.parentID, viewID: viewID)
            list.removeAll { $0 == id }
            setSiblings(list, of: node.parentID, viewID: viewID)
        }
        var list = siblings(of: target.parentID, viewID: viewID)
        let index = min(max(0, target.index - removedBefore), list.count)
        list.insert(contentsOf: moving, at: index)
        setSiblings(list, of: target.parentID, viewID: viewID)
        for id in moving { nodes[id]?.parentID = target.parentID }
        if let parentID = target.parentID { loadedParents.insert(parentID) }
    }

    // MARK: Snapshot

    private func commit(structural: Bool) {
        if structural { structureRevision += 1 }
        rebuildSnapshot(notify: true)
    }

    private func rebuildSnapshot(notify: Bool) {
        generation += 1
        snapshot = SidebarSnapshot(
            generation: generation,
            loadState: loadState,
            pages: pages.map { page in
                SidebarPageRow(
                    id: page.id,
                    name: page.name,
                    views: page.views.map { SidebarViewRow(id: $0.id, name: $0.name) }
                )
            },
            activePageID: activePageID,
            activeViewID: activeViewID,
            layers: loadState == .ready ? buildLayerTree() : .empty,
            selection: selection,
            primarySelection: primarySelection,
            filter: filterQuery.isEmpty ? nil : SidebarFilterState(query: filterQuery, matchCount: matchCount())
        )
        if notify { snapshotDidChange?() }
    }

    private func matches(_ node: Node) -> Bool {
        node.name.localizedStandardContains(filterQuery)
    }

    private func matchCount() -> Int {
        nodes.values.filter { $0.viewID == activeViewID && matches($0) }.count
    }

    private func buildLayerTree() -> SidebarLayerTree {
        guard let viewID = activeViewID else {
            return SidebarLayerTree(structureRevision: structureRevision, topLevelIDs: [], childIDs: [:], rows: [:])
        }
        var rows: [SidebarStableID: SidebarLayerRow] = [:]
        var childIDs: [SidebarStableID: [SidebarStableID]] = [:]
        let filtering = !filterQuery.isEmpty

        // 필터: 일치 노드와 그 조상만 남깁니다.
        var included: Set<SidebarStableID>?
        if filtering {
            var set = Set<SidebarStableID>()
            for node in nodes.values where node.viewID == viewID && matches(node) {
                var current: SidebarStableID? = node.id
                while let id = current, set.insert(id).inserted {
                    current = nodes[id]?.parentID
                }
            }
            included = set
        }

        func visibleChildren(_ list: [SidebarStableID]) -> [SidebarStableID] {
            guard let included else { return list }
            return list.filter { included.contains($0) }
        }

        // 선택된 노드의 조상은 지연 로드 모드에서도 자식을 실어 사이드바가 선택을 펼쳐 보일 수 있게 합니다.
        var forcedLoaded = Set<SidebarStableID>()
        for id in selection {
            var current = nodes[id]?.parentID
            while let parent = current, forcedLoaded.insert(parent).inserted {
                current = nodes[parent]?.parentID
            }
        }

        func add(_ id: SidebarStableID) {
            guard let node = nodes[id] else { return }
            let children = visibleChildren(node.children)
            rows[id] = SidebarLayerRow(
                id: id,
                parentID: node.parentID,
                kind: node.kind,
                shapeKind: node.shapeKind,
                name: node.name,
                isVisible: node.isVisible,
                isLocked: node.isLocked,
                hasChildren: !children.isEmpty,
                isFilterMatch: !filtering || matches(node)
            )
            // 지연 로드 모드에서는 요청받은 부모(필터 중에는 전체)만 자식을 싣습니다.
            let loaded = filtering || !loadsChildrenLazily || loadedParents.contains(id) || forcedLoaded.contains(id)
            if !children.isEmpty, loaded {
                childIDs[id] = children
                children.forEach(add)
            }
        }

        let top = visibleChildren(topLevel[viewID] ?? [])
        top.forEach(add)
        return SidebarLayerTree(structureRevision: structureRevision, topLevelIDs: top, childIDs: childIDs, rows: rows)
    }

    // MARK: 예시 데이터

    private func makeID() -> SidebarStableID {
        serial += 1
        // 결정적 값: 검사 재현성을 위해 일련번호를 하위 바이트에 둡니다(UUIDv7 형식은 흉내 내지 않음).
        var bytes = [UInt8](repeating: 0, count: 16)
        var value = serial
        for index in (8..<16).reversed() {
            bytes[index] = UInt8(value & 0xFF)
            value >>= 8
        }
        bytes[0] = 0x57  // 'W': Stub 표식
        let uuid = UUID(uuid: (
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
        ))
        return SidebarStableID(uuid)
    }

    /// 예시 트리 명세.
    private struct Spec {
        let kind: SidebarNodeKind
        var shapeKind: SidebarShapeKind?
        let name: String
        var visible = true
        var locked = false
        var children: [Spec] = []

        static func frame(_ name: String, _ children: [Spec] = []) -> Spec {
            Spec(kind: .frame, name: name, children: children)
        }
        static func fragment(_ name: String, _ children: [Spec]) -> Spec {
            Spec(kind: .fragment, name: name, children: children)
        }
        static func shape(_ kind: SidebarShapeKind, _ name: String, visible: Bool = true, locked: Bool = false) -> Spec {
            Spec(kind: .shape, shapeKind: kind, name: name, visible: visible, locked: locked)
        }
        static func leaf(_ kind: SidebarNodeKind, _ name: String) -> Spec {
            Spec(kind: kind, name: name)
        }
    }

    private func buildSample() {
        let welcome: [Spec] = [
            .frame("Header", [.leaf(.text, "Title"), .leaf(.text, "Subtitle"), .leaf(.image, "Logo")]),
            .fragment("Icons", [.leaf(.path, "Star"), .leaf(.path, "Heart"), .shape(.polygon, "Badge")]),
            .leaf(.componentInstance, "Primary Button"),
            .shape(.rectangle, "Background", locked: true),
            .shape(.ellipse, "Glow", visible: false),
            .shape(.line, "Divider"),
            .frame("Footer", [.frame("Links", [.leaf(.text, "Privacy"), .leaf(.text, "Terms")])]),
            .leaf(.text, ""),
        ]
        let signIn: [Spec] = [
            .frame("Form", [
                .leaf(.text, "Email Label"),
                .frame("Email Field"),
                .leaf(.componentInstance, "Submit"),
            ]),
        ]

        let onboarding = makeID()
        let welcomeView = makeID()
        let signInView = makeID()
        let settings = makeID()
        let generalView = makeID()
        pages = [
            Page(id: onboarding, name: "Onboarding", views: [(welcomeView, "Welcome"), (signInView, "Sign In")]),
            Page(id: settings, name: "Settings", views: [(generalView, "General")]),
        ]
        topLevel[welcomeView] = welcome.map { add($0, parentID: nil, viewID: welcomeView) }
        topLevel[signInView] = signIn.map { add($0, parentID: nil, viewID: signInView) }
        topLevel[generalView] = []
        activePageID = onboarding
        activeViewID = welcomeView
    }

    private func add(_ spec: Spec, parentID: SidebarStableID?, viewID: SidebarStableID) -> SidebarStableID {
        let id = makeID()
        nodes[id] = Node(
            id: id,
            viewID: viewID,
            parentID: parentID,
            kind: spec.kind,
            shapeKind: spec.shapeKind,
            name: spec.name,
            isVisible: spec.visible,
            isLocked: spec.locked
        )
        let children = spec.children.map { add($0, parentID: id, viewID: viewID) }
        nodes[id]?.children = children
        return id
    }
}
