import Foundation

/// 현재 창에서 이미 투영한 Free Frame 한 개의 고정 크기를 Inspector와 연결합니다.
@MainActor
final class ShellFrameInspectorSource: InspectorDataSource {
    private(set) var snapshot = InspectorSnapshot.placeholder(.noSelection, generation: 0)
    var snapshotHandler: (@MainActor (InspectorSnapshot) -> Void)?
    var resize: ((CanvasFrameDimensions, CGSize, @escaping (Bool) -> Void) -> Bool)?
    private var selected: CanvasFrameDimensions?
    private var trusted = false
    private var inFlight = false
    private var queued: [(width: Bool, value: CGFloat)] = []

    func refresh(_ next: CanvasFrameDimensions?, trusted: Bool) {
        guard next != selected || trusted != self.trusted else { return }
        if next?.session != selected?.session || next?.view != selected?.view || next?.node != selected?.node {
            snapshot.selectionToken &+= 1
            queued.removeAll()
        }
        selected = next
        self.trusted = trusted
        if !trusted || next?.editable != true { queued.removeAll() }
        publish()
    }

    private func publish() {
        let generation = snapshot.generation &+ 1, token = snapshot.selectionToken
        if let selected {
            let editability: InspectorEditability = !trusted
                ? .readOnly(reason: String(localized: "Trust the project to edit this frame."))
                : (selected.editable ? .editable : .locked)
            var row = InspectorPropertyCatalog.frameSizeRow.make { field in
                let size = selected.size
                return InspectorFieldState(value: .number(field.id.path == "frame.size.width" ? size.width : size.height),
                                           isEditable: editability == .editable)
            }
            for index in row.fields.indices { row.fields[index].binding = nil }
            snapshot = InspectorSnapshot(generation: generation, selectionToken: token, state: .ready,
                selection: InspectorSelectionSummary(count: 1, nodeKinds: [.frame], shapeKind: nil, displayName: nil),
                editability: editability, sections: [InspectorSection(id: .frameLayout, title: "Frame", rows: [row])],
                modifiers: nil)
        } else {
            snapshot = .placeholder(.noSelection, generation: generation, selectionToken: token)
        }
        snapshotHandler?(snapshot)
    }

    func submit(_ command: InspectorCommand, basedOn generation: UInt64) {
        guard generation == snapshot.generation, trusted, selected?.editable == true,
              case let .setValue(id, .number(value), phase) = command,
              value.isFinite, value >= 0, Float(value).isFinite,
              ["frame.size.width", "frame.size.height"].contains(id.path) else {
            publish()
            return
        }
        // 화살표 키 반복 중에는 네이티브 필드가 중간값을 표시하고, 키를 놓으면 한 번만 기록합니다.
        guard phase != .continuous else { return }
        queued.append((id.path == "frame.size.width", value))
        submitNext()
    }

    private func submitNext() {
        guard !inFlight, let selected, trusted, selected.editable else { return }
        while !queued.isEmpty {
            let edit = queued.removeFirst()
            let size = CGSize(width: edit.width ? edit.value : selected.size.width,
                              height: edit.width ? selected.size.height : edit.value)
            // 같은 f32로 반올림되는 입력도 실제 문서 값을 다시 표시합니다.
            if Float(size.width) == Float(selected.size.width), Float(size.height) == Float(selected.size.height) {
                publish()
                continue
            }
            inFlight = true
            let accepted = resize?(selected, size) { [weak self] success in
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
}
