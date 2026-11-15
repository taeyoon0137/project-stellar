import Foundation

/// core가 문서에서 읽은 선택 Frame의 Inspector 값(`StellarSelectedNode`). 캔버스 투영에서 읽지 않습니다.
struct CanvasInspectorFrame: Equatable {
    let session: UInt64
    let view: UUID
    let node: UUID
    let revision: UInt64
    let size: CGSize
    let background: InspectorColor?
    let radii: [Float]
    let historySequence: UInt64
    let editable: Bool
}

/// 한 게시 snapshot의 선택 요약과, 단일 Frame 선택이면 그 Inspector 값.
struct InspectorCoreSelection: Equatable {
    let session: UInt64
    let view: UUID
    let revision: UInt64
    let historySequence: UInt64
    let summary: InspectorSelectionSummary
    let frame: CanvasInspectorFrame?

    init?(_ info: StellarSelectedNode, session: UInt64) {
        guard info.selection_count > 0 else { return nil }
        self.session = session
        view = UUID(uuid: info.view)
        revision = info.revision
        historySequence = info.history_sequence
        let kind = InspectorNodeKind(rawValue: info.kind)
        summary = InspectorSelectionSummary(count: Int(info.selection_count), nodeKinds: kind.map { [$0] } ?? [],
                                            shapeKind: InspectorShapeKind(rawValue: info.shape), displayName: nil)
        guard info.flags & UInt8(STELLAR_SELECTED_FRAME) != 0 else {
            frame = nil
            return
        }
        let r = info.radii, c = info.background
        let background = InspectorColor.Space(rawValue: UInt8(truncatingIfNeeded: c.space)).map {
            InspectorColor(space: $0, red: Double(c.rgba.0), green: Double(c.rgba.1), blue: Double(c.rgba.2),
                           alpha: Double(c.rgba.3))
        }
        frame = CanvasInspectorFrame(session: session, view: view, node: UUID(uuid: info.node), revision: info.revision,
                                     size: CGSize(width: CGFloat(info.width), height: CGFloat(info.height)),
                                     background: background, radii: [r.0, r.1, r.2, r.3],
                                     historySequence: info.history_sequence,
                                     editable: info.flags & UInt8(STELLAR_SELECTED_EDITABLE) != 0)
    }
}

enum FrameInspectorEdit {
    /// width·height·모서리 longhand(PropertyKey)의 비음수 길이.
    case length(UInt32, Float)
    case background(InspectorColor?)
}

/// 선택 Free Frame의 고정 크기, 모서리 반경과 배경을 편집합니다. 값과 편집 가능 여부는 core snapshot에서 읽고,
/// Frame이 아닌 선택은 선택 요약과 함께 편집 불가 상태로 보입니다.
@MainActor
final class ShellFrameInspectorSource: InspectorDataSource {
    private(set) var snapshot = InspectorSnapshot.placeholder(.noSelection, generation: 0)
    var snapshotHandler: (@MainActor (InspectorSnapshot) -> Void)?
    var editFrame: ((CanvasInspectorFrame, FrameInspectorEdit, @escaping (Bool) -> Void) -> Bool)?
    private(set) var selection: InspectorCoreSelection?
    private var selected: CanvasInspectorFrame? { selection?.frame }
    private var inFlight = false
    private var queued: [InspectorCommand] = []
    private static let white = InspectorColor(space: .srgb, red: 1, green: 1, blue: 1, alpha: 1)

    func refresh(_ next: InspectorCoreSelection?) {
        guard next != selection else { return }
        let previous = selection?.frame, frame = next?.frame
        let selectionHistoryChanged = frame?.revision == previous?.revision && frame?.historySequence != previous?.historySequence
        if frame?.session != previous?.session || frame?.view != previous?.view || frame?.node != previous?.node || selectionHistoryChanged {
            snapshot.selectionToken &+= 1
            queued.removeAll()
        }
        selection = next
        if frame?.editable != true { queued.removeAll() }
        publish()
    }

    private func publish() {
        let generation = snapshot.generation &+ 1, token = snapshot.selectionToken
        if let selected {
            let editability: InspectorEditability = selected.editable ? .editable : .locked
            var row = InspectorPropertyCatalog.frameSizeRow.make { field in
                let size = selected.size
                return InspectorFieldState(value: .number(field.id.path == "frame.size.width" ? size.width : size.height),
                                           isEditable: editability == .editable)
            }
            for index in row.fields.indices { row.fields[index].binding = nil }
            var paint = InspectorPropertyCatalog.frameBackgroundRow.make { field in
                let isToggle = field.id.path == "frame.background.enabled"
                let value: InspectorValue = isToggle ? .bool(selected.background != nil) : .color(selected.background ?? Self.white)
                return InspectorFieldState(value: value, isEditable: editability == .editable && (isToggle || selected.background != nil))
            }
            for index in paint.fields.indices { paint.fields[index].binding = nil }
            let radii = InspectorPropertyCatalog.frameRadiusRows.map { template in
                var result = template.make { field in
                    guard let key = field.id.coreKey, let corner = InspectorPropertyCatalog.FrameCorner(rawValue: key)
                    else { return InspectorFieldState(value: .none, isEditable: false) }
                    return InspectorFieldState(value: .number(Double(selected.radii[corner.index])),
                                               isEditable: editability == .editable)
                }
                for index in result.fields.indices { result.fields[index].binding = nil }
                return result
            }
            snapshot = InspectorSnapshot(generation: generation, selectionToken: token, state: .ready,
                selection: InspectorSelectionSummary(count: 1, nodeKinds: [.frame], shapeKind: nil, displayName: nil),
                editability: editability, sections: [
                    InspectorSection(id: .frameLayout, title: "Frame", rows: [row]),
                    InspectorSection(id: .frameAppearance, title: "Appearance", rows: [paint] + radii),
                ],
                modifiers: nil)
        } else if let selection {
            // Frame이 아닌 선택과 다중 선택은 아직 Inspector 편집을 연결하지 않았습니다. "선택 없음"으로 보이지 않습니다.
            snapshot = InspectorSnapshot(generation: generation, selectionToken: token, state: .ready,
                selection: selection.summary,
                editability: .readOnly(reason: String(localized: "Editing this selection isn’t available yet.")),
                sections: [], modifiers: nil)
        } else {
            snapshot = .placeholder(.noSelection, generation: generation, selectionToken: token)
        }
        snapshotHandler?(snapshot)
    }

    func submit(_ command: InspectorCommand, basedOn generation: UInt64) {
        guard generation == snapshot.generation, selected?.editable == true,
              case let .setValue(_, _, phase) = command else {
            publish()
            return
        }
        // 창의 색 패널 드래그와 키 반복 모두 종료 시 한 번만 문서 history에 기록합니다.
        guard phase != .continuous else { return }
        queued.append(command)
        submitNext()
    }

    private func submitNext() {
        guard !inFlight, let selected, selected.editable else { return }
        while !queued.isEmpty {
            guard let change = edit(queued.removeFirst(), on: selected) else {
                publish()
                continue
            }
            inFlight = true
            let accepted = editFrame?(selected, change) { [weak self] success in
                guard let self else { return }
                self.inFlight = false
                if !success {
                    self.queued.removeAll()
                    self.publish()
                }
                self.submitNext()
            } ?? false
            if !accepted {
                inFlight = false
                queued.removeAll()
                publish()
            }
            return
        }
    }

    private func edit(_ command: InspectorCommand, on frame: CanvasInspectorFrame) -> FrameInspectorEdit? {
        guard case let .setValue(id, value, _) = command else { return nil }
        if let key = id.coreKey, let corner = InspectorPropertyCatalog.FrameCorner(rawValue: key),
           id == corner.field.id, case .number(let value) = value {
            let radius = Float(value)
            guard value >= 0, radius.isFinite, radius != frame.radii[corner.index] else { return nil }
            return .length(corner.rawValue, radius)
        }
        switch (id.path, value) {
        case ("frame.size.width", .number(let value)), ("frame.size.height", .number(let value)):
            let length = Float(value)
            guard value >= 0, length.isFinite, let key = id.coreKey else { return nil }
            let current = id.path == "frame.size.width" ? frame.size.width : frame.size.height
            return CGFloat(length) == current ? nil : .length(key, length)
        case ("frame.background.enabled", .bool(let enabled)):
            guard enabled != (frame.background != nil) else { return nil }
            return .background(enabled ? Self.white : nil)
        case ("frame.background.color", .color(let color)):
            guard frame.background != nil, [color.red, color.green, color.blue, color.alpha].allSatisfy({ Float($0).isFinite }),
                  (0...1).contains(color.alpha), color != frame.background else { return nil }
            return .background(color)
        default:
            return nil
        }
    }

}
