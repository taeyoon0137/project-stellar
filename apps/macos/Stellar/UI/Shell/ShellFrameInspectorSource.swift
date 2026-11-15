import Foundation

enum FrameInspectorEdit {
    case size(CGSize)
    case background(InspectorColor?)
    case corner(InspectorPropertyCatalog.FrameCorner, Float)
}

/// 현재 창에서 이미 투영한 Free Frame의 고정 크기, 모서리 반경과 배경을 편집합니다.
@MainActor
final class ShellFrameInspectorSource: InspectorDataSource {
    private(set) var snapshot = InspectorSnapshot.placeholder(.noSelection, generation: 0)
    var snapshotHandler: (@MainActor (InspectorSnapshot) -> Void)?
    var editFrame: ((CanvasInspectorFrame, FrameInspectorEdit, @escaping (Bool) -> Void) -> Bool)?
    private var selected: CanvasInspectorFrame?
    private var inFlight = false
    private var queued: [InspectorCommand] = []
    private static let white = InspectorColor(space: .srgb, red: 1, green: 1, blue: 1, alpha: 1)

    func refresh(_ next: CanvasInspectorFrame?) {
        guard next != selected else { return }
        let selectionHistoryChanged = next?.revision == selected?.revision && next?.historySequence != selected?.historySequence
        if next?.session != selected?.session || next?.view != selected?.view || next?.node != selected?.node || selectionHistoryChanged {
            snapshot.selectionToken &+= 1
            queued.removeAll()
        }
        selected = next
        if next?.editable != true { queued.removeAll() }
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
            return .corner(corner, radius)
        }
        switch (id.path, value) {
        case ("frame.size.width", .number(let value)), ("frame.size.height", .number(let value)):
            guard value >= 0, Float(value).isFinite else { return nil }
            var size = frame.size
            if id.path == "frame.size.width" { size.width = CGFloat(Float(value)) }
            else { size.height = CGFloat(Float(value)) }
            return size == frame.size ? nil : .size(size)
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
