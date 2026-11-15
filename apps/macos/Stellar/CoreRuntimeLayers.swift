import Foundation

/// 한 부모의 다음 64행. worker가 caller 소유 공간에서 Swift 값으로 바꾼 후 UI에 전달합니다.
struct WindowLayerPage {
    let historySequence: UInt64
    let page: SidebarPageRow
    let rows: [SidebarLayerRow]
    let selected: [SidebarStableID: Int]
    let primary: SidebarStableID?
    let revealChild: SidebarStableID?
    let hasMore: Bool
}

/// 레이어 행에서 켜고 끄는 노드 속성(`node.visible`·`node.locked`).
enum ShellLayerFlag {
    case visible
    case locked

    var code: UInt32 {
        switch self {
        case .visible: return UInt32(STELLAR_LAYER_FLAG_VISIBLE)
        case .locked: return UInt32(STELLAR_LAYER_FLAG_LOCKED)
        }
    }
}

extension CoreRuntime {
    /// 표시한 레이어 행들의 표시 또는 잠금을 한 transaction으로 바꾸는 요청을 접수합니다.
    static func setLayerFlag(handle: UInt64, session: UInt64, view: UUID, revision: UInt64, nodes: [UUID],
                             flag: ShellLayerFlag, value: Bool) -> Result<UInt64, CoreRuntimeCallError> {
        guard !nodes.isEmpty else { return .failure(CoreRuntimeCallError(status: STELLAR_INVALID_ARGUMENT)) }
        var request: UInt64 = 0
        let ids = nodes.map(\.uuid)
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            ids.withUnsafeBytes { nodeBytes in
                stellar_window_layer_set_flag(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress, revision,
                                              nodeBytes.bindMemory(to: UInt8.self).baseAddress, ids.count, flag.code,
                                              value ? 1 : 0, &request)
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func layerOrderFlags(handle: UInt64, target: StellarSelectionTarget) -> UInt32 {
        var target = target
        return stellar_window_layer_order_flags(handle, &target)
    }

    static func renameLayer(handle: UInt64, session: UInt64, view: UUID, node: UUID, revision: UInt64,
                            name: String) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: node.uuid) { nodeBytes in
                Array(name.utf8).withUnsafeBufferPointer { bytes in
                    stellar_window_layer_rename(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress,
                        nodeBytes.bindMemory(to: UInt8.self).baseAddress, revision, bytes.baseAddress, bytes.count, &request)
                }
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func layerPage(handle: UInt64, session: UInt64, view: UUID, revision: UInt64,
                          parent: SidebarStableID?, after: SidebarStableID?) -> WindowLayerPage? {
        precondition(!Thread.isMainThread)
        var query = StellarLayerQuery()
        query.view = view.uuid
        query.revision = revision
        if let parent { query.parent = parent.uuid.uuid }
        if let after { query.after = after.uuid.uuid }
        var info = StellarLayerInfo()
        var history: UInt64 = 0
        var rows = Array(repeating: StellarLayerRow(), count: Int(STELLAR_LAYER_PAGE_SIZE))
        var names = [UInt8](repeating: 0, count: 4096)
        func read() -> StellarStatus {
            rows.withUnsafeMutableBufferPointer { rowBuffer in
                names.withUnsafeMutableBufferPointer { text in
                    stellar_window_layer_page(handle, session, &query, &info, rowBuffer.baseAddress,
                                                          text.baseAddress, text.count, &history)
                }
            }
        }
        var status = read()
        if status == STELLAR_BUFFER_TOO_SMALL {
            names = [UInt8](repeating: 0, count: info.text_len)
            status = read()
        }
        guard status == STELLAR_OK, Int(info.count) <= rows.count else { return nil }
        var offset = 0
        func name(_ count: Int) -> String {
            defer { offset += count }
            return String(decoding: names[offset..<(offset + count)], as: UTF8.self)
        }
        func optionalID(_ bytes: uuid_t) -> SidebarStableID? {
            withUnsafeBytes(of: bytes) { $0.allSatisfy { $0 == 0 } ? nil : SidebarStableID(UUID(uuid: bytes)) }
        }
        let page = SidebarPageRow(id: SidebarStableID(UUID(uuid: info.page)), name: name(info.page_name_len),
                                  views: [SidebarViewRow(id: SidebarStableID(view), name: name(info.view_name_len))])
        var items: [SidebarLayerRow] = [], selected: [SidebarStableID: Int] = [:]
        for row in rows.prefix(Int(info.count)) {
            guard let kind = SidebarNodeKind(rawValue: row.kind) else { return nil }
            let id = SidebarStableID(UUID(uuid: row.id))
            items.append(SidebarLayerRow(id: id, parentID: parent, kind: kind, shapeKind: SidebarShapeKind(rawValue: row.shape),
                name: name(row.name_len), isVisible: row.flags & 1 != 0, isLocked: row.flags & 2 != 0,
                hasChildren: row.flags & 4 != 0, isFilterMatch: true))
            if row.selection_order != 0 { selected[id] = row.selection_order }
        }
        return WindowLayerPage(historySequence: history, page: page, rows: items, selected: selected, primary: optionalID(info.primary),
                               revealChild: optionalID(info.reveal_child), hasMore: info.more != 0)
    }
}
