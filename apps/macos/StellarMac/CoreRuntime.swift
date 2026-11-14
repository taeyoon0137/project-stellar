import Foundation

/// Rust 코어(stellar-ffi)를 호출하는 FFI 헬퍼.
///
/// FFI 규칙(정수 handle, `StellarStatus` 반환, caller buffer 출력)을 그대로 따릅니다. 셸의 다른 파일은
/// `stellar_` 함수를 직접 부르지 않고 이 헬퍼를 거칩니다(캔버스 drawable·입력 전달 `CanvasView` 제외).
enum CoreRuntime {
    /// 주어진 handle의 코어 버전 문자열을 읽습니다(길이 조회 → caller buffer 채우기).
    static func version(handle: UInt64) -> String? {
        var needed = 0
        guard stellar_runtime_version(handle, nil, 0, &needed) == STELLAR_OK else { return nil }

        var buffer = [UInt8](repeating: 0, count: needed)
        var written = 0
        let status = buffer.withUnsafeMutableBufferPointer { raw in
            stellar_runtime_version(handle, raw.baseAddress, needed, &written)
        }
        guard status == STELLAR_OK else { return nil }
        return String(decoding: buffer[0..<written], as: UTF8.self)
    }

    // MARK: - 프로젝트 session (XII.6.2)

    /// 프로젝트 폴더 열기를 요청합니다. 기다리지 않고 request id를 돌려줍니다.
    ///
    /// 결과는 `pollEvent(handle:)`로 꺼내는 `.projectOpened` 또는 `.requestFailed` 하나입니다. 빈 경로·상대 경로·NUL은
    /// `STELLAR_INVALID_ARGUMENT`, 대기 요청이 가득 차면 `STELLAR_QUEUE_FULL`입니다(요청 없음).
    static func openProject(handle: UInt64, path: String) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let bytes = Array(path.utf8)
        let status = bytes.withUnsafeBufferPointer { raw in
            stellar_project_open(handle, raw.baseAddress, raw.count, &request)
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    /// 프로젝트 session 닫기를 요청합니다(그 아래 문서도 닫음).
    static func closeProject(handle: UInt64, project: UInt64) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = stellar_project_close(handle, project, &request)
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    // MARK: - 창 session (XII.2.2)

    /// 프로젝트 session 아래에서 창 session 열기를 요청합니다. 결과는 `.windowSessionOpened` 또는 `.requestFailed`입니다.
    static func openWindowSession(handle: UInt64, project: UInt64) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = stellar_window_session_open(handle, project, &request)
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    /// 창 session 닫기를 요청합니다(문서 state·history·선택 폐기).
    static func closeWindowSession(handle: UInt64, session: UInt64) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = stellar_window_session_close(handle, session, &request)
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    /// 창 session에 명령 코드(`STELLAR_COMMAND_*`)를 보냅니다(XII.6.4). 결과는 `.historyReplayed`·`.windowSessionSaved`
    /// 또는 `.requestFailed`입니다. 모르는 코드는 `STELLAR_INVALID_ARGUMENT`(요청 없음)입니다.
    static func perform(handle: UInt64, session: UInt64, command: UInt32) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = stellar_window_session_perform(handle, session, command, &request)
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    /// 창 session의 선택을 바꾸고 그 창 history에 기록합니다. 결과는 `.windowViewChanged` 또는 `.requestFailed`입니다.
    static func select(handle: UInt64, session: UInt64, ids: [UUID]) -> Result<UInt64, CoreRuntimeCallError> {
        var bytes: [UInt8] = []
        bytes.reserveCapacity(ids.count * 16)
        for id in ids {
            withUnsafeBytes(of: id.uuid) { bytes.append(contentsOf: $0) }
        }
        var request: UInt64 = 0
        let status = bytes.withUnsafeBufferPointer { raw in
            stellar_window_session_select(handle, session, raw.baseAddress, ids.count, &request)
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    /// 열린 창 session의 최신 정보. 열려 있지 않으면 nil입니다. document thread를 기다리지 않습니다.
    static func windowSessionInfo(handle: UInt64, session: UInt64) -> StellarWindowSessionInfo? {
        var info = StellarWindowSessionInfo()
        guard stellar_window_session_info(handle, session, &info) == STELLAR_OK,
            info.version == UInt32(STELLAR_WINDOW_SESSION_INFO_VERSION)
        else { return nil }
        return info
    }

    /// 최신 상태 snapshot 요약. document thread를 기다리지 않습니다.
    static func state(handle: UInt64) -> StellarRuntimeState? {
        var state = StellarRuntimeState()
        guard stellar_runtime_state(handle, &state) == STELLAR_OK,
            state.version == UInt32(STELLAR_RUNTIME_STATE_VERSION)
        else { return nil }
        return state
    }

    /// 다음 완료 event 하나를 꺼냅니다. 큐가 비었으면 nil입니다.
    ///
    /// 먼저 buffer 없이 kind와 필요한 길이를 읽고(pop하지 않음), 그 크기 buffer로 다시 읽어 pop합니다.
    static func pollEvent(handle: UInt64) -> CoreRuntimeEvent? {
        var kind: UInt32 = 0
        var needed = 0
        let peek = stellar_runtime_poll_event(handle, &kind, nil, 0, &needed)
        guard peek == STELLAR_OK else { return nil }
        var buffer = [UInt8](repeating: 0, count: max(needed, 1))
        var written = 0
        let status = buffer.withUnsafeMutableBufferPointer { raw in
            stellar_runtime_poll_event(handle, &kind, raw.baseAddress, raw.count, &written)
        }
        guard status == STELLAR_OK else { return nil }
        return CoreRuntimeEvent.decode(kind: kind, payload: buffer[0..<written])
    }
}

/// FFI 호출 자체가 요청을 받지 않은 경우(`StellarStatus`가 OK가 아님).
struct CoreRuntimeCallError: Error, Equatable {
    let status: StellarStatus

    static func == (lhs: CoreRuntimeCallError, rhs: CoreRuntimeCallError) -> Bool {
        lhs.status.rawValue == rhs.status.rawValue
    }
}

/// 코어 완료 event(`stellar_ffi.h` "프로젝트·문서 요청의 완료 event"). 요청마다 정확히 하나가 옵니다.
enum CoreRuntimeEvent: Equatable {
    case projectOpened(request: UInt64, project: UInt64, projectID: UUID, name: String, root: String)
    case projectClosed(request: UInt64, project: UInt64, closedDocuments: UInt32)
    case documentOpened(request: UInt64, project: UInt64, document: UInt64, documentID: UUID, alreadyOpen: Bool)
    case documentClosed(request: UInt64, document: UInt64)
    case requestFailed(request: UInt64, failure: UInt32, message: String)
    /// 창 session을 열었습니다. `documentID`는 작업 문서가 없으면(저장한 적 없는 프로젝트) nil입니다.
    case windowSessionOpened(
        request: UInt64, project: UInt64, session: UInt64, windowID: UUID, documentID: UUID?, generation: UInt64
    )
    case windowSessionClosed(request: UInt64, session: UInt64)
    /// 창 view state(탭·active view·viewport·선택)가 바뀌었습니다. 새 값은 창 session 정보에 있습니다.
    case windowViewChanged(request: UInt64, session: UInt64)
    /// 창 history를 undo(`redo == false`)·redo로 재생했습니다. `applied == false`면 재생할 항목이 없었습니다.
    case historyReplayed(request: UInt64, session: UInt64, redo: Bool, applied: Bool, revision: UInt64)
    /// 창 작업 문서를 저장했습니다(Save Now). `written == false`면 바뀐 것이 없어 새 generation을 쓰지 않았습니다.
    case windowSessionSaved(request: UInt64, session: UInt64, generation: UInt64, written: Bool)
    /// 이 셸이 모르는 kind이거나 payload를 해석하지 못한 event. 요청 id를 알 수 없으므로 버립니다.
    case undecodable(kind: UInt32)

    /// event가 완료한 요청 id. 해석하지 못한 event는 nil입니다.
    var request: UInt64? {
        switch self {
        case .projectOpened(let request, _, _, _, _), .projectClosed(let request, _, _),
             .documentOpened(let request, _, _, _, _), .documentClosed(let request, _),
             .requestFailed(let request, _, _), .windowSessionOpened(let request, _, _, _, _, _),
             .windowSessionClosed(let request, _), .windowViewChanged(let request, _),
             .historyReplayed(let request, _, _, _, _), .windowSessionSaved(let request, _, _, _):
            return request
        case .undecodable:
            return nil
        }
    }

    /// little-endian payload를 해석합니다. schema version이 다르거나 길이가 맞지 않으면 `.undecodable`입니다.
    static func decode(kind: UInt32, payload: ArraySlice<UInt8>) -> CoreRuntimeEvent {
        var reader = PayloadReader(bytes: payload)
        guard reader.u32() == UInt32(STELLAR_EVENT_SCHEMA_VERSION) else { return .undecodable(kind: kind) }
        let event: CoreRuntimeEvent?
        switch kind {
        case UInt32(STELLAR_EVENT_PROJECT_OPENED):
            event = reader.u64().flatMap { request in
                reader.u64().flatMap { project in
                    reader.uuid().flatMap { id in
                        reader.text().flatMap { name in
                            reader.text().map { root in
                                .projectOpened(request: request, project: project, projectID: id, name: name, root: root)
                            }
                        }
                    }
                }
            }
        case UInt32(STELLAR_EVENT_PROJECT_CLOSED):
            event = reader.u64().flatMap { request in
                reader.u64().flatMap { project in
                    reader.u32().map { .projectClosed(request: request, project: project, closedDocuments: $0) }
                }
            }
        case UInt32(STELLAR_EVENT_DOCUMENT_OPENED):
            event = reader.u64().flatMap { request in
                reader.u64().flatMap { project in
                    reader.u64().flatMap { document in
                        reader.uuid().flatMap { id in
                            reader.u8().map {
                                .documentOpened(
                                    request: request, project: project, document: document, documentID: id,
                                    alreadyOpen: $0 != 0
                                )
                            }
                        }
                    }
                }
            }
        case UInt32(STELLAR_EVENT_DOCUMENT_CLOSED):
            event = reader.u64().flatMap { request in
                reader.u64().map { .documentClosed(request: request, document: $0) }
            }
        case UInt32(STELLAR_EVENT_WINDOW_SESSION_OPENED):
            event = reader.u64().flatMap { request in
                reader.u64().flatMap { project in
                    reader.u64().flatMap { session in
                        reader.uuid().flatMap { windowID in
                            reader.uuid().flatMap { documentID in
                                reader.u64().map {
                                    .windowSessionOpened(
                                        request: request, project: project, session: session, windowID: windowID,
                                        documentID: documentID == Self.nilID ? nil : documentID, generation: $0
                                    )
                                }
                            }
                        }
                    }
                }
            }
        case UInt32(STELLAR_EVENT_WINDOW_SESSION_CLOSED):
            event = reader.u64().flatMap { request in
                reader.u64().map { .windowSessionClosed(request: request, session: $0) }
            }
        case UInt32(STELLAR_EVENT_WINDOW_VIEW_CHANGED):
            event = reader.u64().flatMap { request in
                reader.u64().map { .windowViewChanged(request: request, session: $0) }
            }
        case UInt32(STELLAR_EVENT_HISTORY_REPLAYED):
            event = reader.u64().flatMap { request in
                reader.u64().flatMap { session in
                    reader.bool().flatMap { redo in
                        reader.bool().flatMap { applied in
                            reader.u64().map {
                                .historyReplayed(
                                    request: request, session: session, redo: redo, applied: applied, revision: $0
                                )
                            }
                        }
                    }
                }
            }
        case UInt32(STELLAR_EVENT_WINDOW_SESSION_SAVED):
            event = reader.u64().flatMap { request in
                reader.u64().flatMap { session in
                    reader.u64().flatMap { generation in
                        reader.bool().map {
                            .windowSessionSaved(request: request, session: session, generation: generation, written: $0)
                        }
                    }
                }
            }
        case UInt32(STELLAR_EVENT_REQUEST_FAILED):
            event = reader.u64().flatMap { request in
                reader.u32().flatMap { failure in
                    reader.text().map { .requestFailed(request: request, failure: failure, message: $0) }
                }
            }
        default:
            event = nil
        }
        guard let event, reader.isAtEnd else { return .undecodable(kind: kind) }
        return event
    }

    /// NIL StableId(0 16바이트).
    static let nilID = UUID(uuid: (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0))

    /// event payload 순차 reader(little-endian, 범위 밖이면 nil).
    private struct PayloadReader {
        let bytes: ArraySlice<UInt8>
        var offset: Int

        init(bytes: ArraySlice<UInt8>) {
            self.bytes = bytes
            offset = bytes.startIndex
        }

        var isAtEnd: Bool { offset == bytes.endIndex }

        private mutating func take(_ count: Int) -> ArraySlice<UInt8>? {
            guard count >= 0, bytes.endIndex - offset >= count else { return nil }
            defer { offset += count }
            return bytes[offset..<(offset + count)]
        }

        private mutating func integer<T: FixedWidthInteger>(_: T.Type) -> T? {
            guard let raw = take(MemoryLayout<T>.size) else { return nil }
            var value: T = 0
            for (shift, byte) in raw.enumerated() {
                value |= T(byte) << (8 * shift)
            }
            return value
        }

        mutating func u8() -> UInt8? { integer(UInt8.self) }
        /// 0/1 바이트. 다른 값이면 nil(해석 실패)입니다.
        mutating func bool() -> Bool? {
            switch u8() {
            case 0: return false
            case 1: return true
            default: return nil
            }
        }
        mutating func u32() -> UInt32? { integer(UInt32.self) }
        mutating func u64() -> UInt64? { integer(UInt64.self) }

        mutating func uuid() -> UUID? {
            guard let raw = take(16) else { return nil }
            let b = Array(raw)
            return UUID(uuid: (b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                               b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]))
        }

        mutating func text() -> String? {
            guard let length = u32(), let raw = take(Int(length)) else { return nil }
            return String(bytes: raw, encoding: .utf8)
        }
    }
}
