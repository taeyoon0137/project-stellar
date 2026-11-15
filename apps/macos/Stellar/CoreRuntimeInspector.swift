import Foundation

/// 속성 패널 C ABI(`stellar_window_inspector`, `stellar_window_edit_property`, ADR-0070) 래퍼.
///
/// 셸은 코어가 만든 행(CSS 이름, 정규 값 텍스트, flag)을 그대로 표시하고, 입력 텍스트·수치 delta·편집 단계만
/// 보냅니다. 값 해석과 저장 위치는 코어가 정합니다.
extension CoreRuntime {
    /// 창의 확정 선택에 대한 속성 행. 창이 없으면 nil입니다. 게시된 snapshot을 읽으므로 기다리지 않습니다.
    static func inspector(handle: UInt64, session: UInt64) -> InspectorSnapshot? {
        var info = StellarInspectorInfo()
        var rows = [StellarInspectorRow](repeating: StellarInspectorRow(), count: 64)
        var text = [UInt8](repeating: 0, count: 4096)
        var status = stellar_window_inspector(handle, session, &info, &rows, rows.count, &text, text.count)
        if status == STELLAR_BUFFER_TOO_SMALL {
            rows = [StellarInspectorRow](repeating: StellarInspectorRow(), count: Int(info.row_count))
            text = [UInt8](repeating: 0, count: Int(info.text_len))
            status = stellar_window_inspector(handle, session, &info, &rows, rows.count, &text, text.count)
        }
        guard status == STELLAR_OK else { return nil }
        var offset = 0
        func take(_ length: UInt32) -> String {
            let end = min(offset + Int(length), Int(info.text_len))
            defer { offset = end }
            return String(decoding: text[offset..<end], as: UTF8.self)
        }
        let name = take(info.name_len)
        var parsed: [InspectorRowState] = []
        parsed.reserveCapacity(Int(info.row_count))
        for row in rows.prefix(Int(info.row_count)) {
            let property = take(row.name_len)
            let value = take(row.value_len)
            let choices = take(row.choices_len)
            parsed.append(InspectorRowState(
                name: property, key: row.key, flags: InspectorRowFlags(rawValue: row.flags),
                section: InspectorSectionKind(rawValue: row.section) ?? .kind, value: value,
                choices: choices.isEmpty ? [] : choices.split(separator: " ").map(String.init)
            ))
        }
        return InspectorSnapshot(
            count: Int(info.count), classes: InspectorNodeClasses(rawValue: info.classes), displayName: name,
            revision: info.revision, historySequence: info.history_sequence, view: UUID(uuid: info.view), rows: parsed
        )
    }

    /// 확정 선택 전체에 속성 하나를 적용하도록 요청합니다. 값 문법 오류는 요청 없이 진단 텍스트로 돌아옵니다.
    static func editProperty(handle: UInt64, session: UInt64, basis: InspectorSnapshot, name: String,
                             input: InspectorInput, phase: InspectorEditPhase) -> Result<UInt64, InspectorEditFailure> {
        var edit = StellarPropertyEdit()
        withUnsafeMutableBytes(of: &edit.view) { raw in
            withUnsafeBytes(of: basis.view.uuid) { raw.copyMemory(from: $0) }
        }
        edit.revision = basis.revision
        edit.history_sequence = basis.historySequence
        edit.phase = phase.rawValue
        var value = [UInt8]()
        switch input {
        case .text(let text):
            edit.mode = 0
            value = Array(text.utf8)
        case .adjust(let delta):
            edit.mode = 1
            edit.delta = delta
        }
        let nameBytes = Array(name.utf8)
        var request: UInt64 = 0
        var diagnostic = [UInt8](repeating: 0, count: 512)
        var diagnosticLength = 0
        let status = nameBytes.withUnsafeBufferPointer { namePointer in
            value.withUnsafeBufferPointer { valuePointer in
                stellar_window_edit_property(handle, session, &edit, namePointer.baseAddress, nameBytes.count,
                                             valuePointer.baseAddress, value.count, &request,
                                             &diagnostic, diagnostic.count, &diagnosticLength)
            }
        }
        if status == STELLAR_OK, request != 0 { return .success(request) }
        let length = min(diagnosticLength, diagnostic.count)
        let message = length > 0 ? String(decoding: diagnostic[0..<length], as: UTF8.self) : ""
        return .failure(InspectorEditFailure(status: status, diagnostic: message))
    }
}

extension ShellRuntimeEventPump {
    /// 속성 편집 요청을 보내고 완료 event를 `completion`으로 받습니다.
    func editProperty(session: UInt64, basis: InspectorSnapshot, name: String, input: InspectorInput,
                      phase: InspectorEditPhase, completion: @escaping (CoreRuntimeEvent) -> Void)
        -> Result<UInt64, InspectorEditFailure> {
        let result = CoreRuntime.editProperty(handle: runtimeHandle, session: session, basis: basis, name: name,
                                              input: input, phase: phase)
        _ = register(result.mapError { CoreRuntimeCallError(status: $0.status) }, completion)
        return result
    }
}
