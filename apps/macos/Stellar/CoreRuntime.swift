import CoreGraphics
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

    static func selectedFrame(handle: UInt64, session: UInt64) -> StellarFrameSelection? {
        var frame = StellarFrameSelection()
        return stellar_window_selected_frame(handle, session, &frame) == STELLAR_OK ? frame : nil
    }

    static func createComponent(handle: UInt64, session: UInt64, name: String,
                                selection: StellarFrameSelection? = nil) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = Array(name.utf8).withUnsafeBufferPointer { bytes in
            guard let selection else {
                return stellar_window_component_create(handle, session, bytes.baseAddress, bytes.count, &request)
            }
            return withUnsafeBytes(of: selection.view) { view in
                withUnsafeBytes(of: selection.node) { node in
                    stellar_window_component_from_selection(handle, session, bytes.baseAddress, bytes.count,
                        view.bindMemory(to: UInt8.self).baseAddress, node.bindMemory(to: UInt8.self).baseAddress,
                        selection.revision, &request)
                }
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func createProject(handle: UInt64, path: String) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let bytes = Array(path.utf8)
        let status = bytes.withUnsafeBufferPointer { raw in
            stellar_project_create(handle, raw.baseAddress, raw.count, &request)
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

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

    static func gitNames(handle: UInt64, session: UInt64, selection: (UInt64, UInt32)?, stage: Bool = true) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = stellar_git_names_request(handle, session, selection == nil ? 0 : (stage ? 1 : 2),
                                              selection?.0 ?? 0, selection?.1 ?? 0, &request)
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func cancelGitNames(handle: UInt64, session: UInt64, request: UInt64) {
        _ = stellar_git_names_cancel(handle, session, request)
    }

    static func gitPanelRow(handle: UInt64, session: UInt64, generation: UInt64, row: UInt32) -> CoreGitChange? {
        var length = 0
        let nameStatus = stellar_git_names_row(handle, session, generation, row, nil, 0, &length)
        let component = nameStatus == STELLAR_NO_ERROR
        let read = component ? stellar_git_component_row : stellar_git_names_row
        if component {
            guard read(handle, session, generation, row, nil, 0, &length) == STELLAR_OK else { return nil }
        } else if nameStatus != STELLAR_OK { return nil }
        guard length > 0, length <= 45 + 3 * Int(STELLAR_EVENT_TEXT_MAX) else { return nil }
        var bytes = [UInt8](repeating: 0, count: length)
        let status = bytes.withUnsafeMutableBufferPointer {
            read(handle, session, generation, row, $0.baseAddress, $0.count, &length)
        }
        guard status == STELLAR_OK, length == bytes.count else { return nil }
        return component ? CoreRuntimeEvent.decodeComponentRow(bytes[...]) : CoreRuntimeEvent.decodeName(bytes[...])
    }

    static func projectTrust(handle: UInt64, project: UInt64, decision: Bool?) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let action: UInt32 = decision.map { $0 ? 1 : 2 } ?? 0
        let status = stellar_project_trust(handle, project, action, &request)
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

    /// 열린 창 session의 view 정보(탭 수, active view, viewport, XII.4.3). 열려 있지 않으면 nil입니다.
    static func windowViewInfo(handle: UInt64, session: UInt64) -> StellarWindowViewInfo? {
        var info = StellarWindowViewInfo()
        guard stellar_window_view_info(handle, session, &info) == STELLAR_OK,
            info.version == UInt32(STELLAR_WINDOW_VIEW_INFO_VERSION)
        else { return nil }
        return info
    }

    /// 공유 command 이름(예: `edit.undo`)의 코어 명령 코드(XII.6.5). 코어 명령이 아니면 nil입니다(handle 불필요).
    static func commandCode(name: String) -> UInt32? {
        var code: UInt32 = 0
        let bytes = Array(name.utf8)
        let status = bytes.withUnsafeBufferPointer { raw in
            stellar_command_code(raw.baseAddress, raw.count, &code)
        }
        guard status == STELLAR_OK, code != 0 else { return nil }
        return code
    }

    /// 창 session에 명령 코드(`STELLAR_COMMAND_*`)를 보냅니다(XII.6.4). 결과는 `.historyReplayed`·`.windowSessionSaved`
    /// 또는 `.requestFailed`입니다. 모르는 코드는 `STELLAR_INVALID_ARGUMENT`(요청 없음)입니다.
    static func perform(handle: UInt64, session: UInt64, command: UInt32) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = stellar_window_session_perform(handle, session, command, &request)
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func fitRoot(handle: UInt64, session: UInt64, size: CGSize) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = stellar_window_session_fit_root(handle, session, size.width, size.height, &request)
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func pan(handle: UInt64, session: UInt64, dx: Double, dy: Double) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = stellar_window_session_pan(handle, session, dx, dy, &request)
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

    static func selectionTarget(handle: UInt64, session: UInt64) -> StellarSelectionTarget? {
        var target = StellarSelectionTarget()
        return stellar_window_selection_target(handle, session, &target) == STELLAR_OK ? target : nil
    }

    static func editSelection(handle: UInt64, target: StellarSelectionTarget, action: UInt32,
                              ids: [UUID]?) -> Result<UInt64, CoreRuntimeCallError> {
        var target = target, request: UInt64 = 0
        let status: StellarStatus
        if let ids {
            guard !ids.isEmpty else { return .failure(CoreRuntimeCallError(status: STELLAR_INVALID_ARGUMENT)) }
            let bytes = ids.map(\.uuid)
            status = bytes.withUnsafeBytes { raw in
                stellar_window_edit_selection(handle, &target, action, raw.bindMemory(to: UInt8.self).baseAddress, ids.count, &request)
            }
        } else {
            status = stellar_window_edit_selection(handle, &target, action, nil, 0, &request)
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    /// Call from a worker queue; the document ticket may wait.
    static func copySubtree(handle: UInt64, session: UInt64, view: UUID, revision: UInt64) -> Data? {
        var data = Data(count: 16 * 1024 * 1024)
        var count = 0
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            data.withUnsafeMutableBytes { bytes in
                stellar_window_session_copy_subtree(handle, session,
                    viewBytes.bindMemory(to: UInt8.self).baseAddress, revision,
                    bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, &count)
            }
        }
        guard status == STELLAR_OK, count > 0 else { return nil }
        data.count = count
        return data
    }

    /// Pasteboard 데이터는 FFI가 반환 전에 복사합니다. 완료는 기존 창 event 경로로 받습니다.
    static func pasteSubtree(handle: UInt64, session: UInt64, view: UUID, parent: UUID,
                             revision: UInt64, data: Data) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: parent.uuid) { parentBytes in
                data.withUnsafeBytes { bytes in
                    stellar_window_session_paste_subtree(handle, session,
                        viewBytes.bindMemory(to: UInt8.self).baseAddress,
                        parentBytes.bindMemory(to: UInt8.self).baseAddress, revision,
                        bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, &request)
                }
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func insertFrame(handle: UInt64, session: UInt64, view: UUID, parent: UUID, revision: UInt64,
                            rect: CGRect, rectangle: Bool) -> Result<UInt64, CoreRuntimeCallError> {
        let insert = rectangle ? stellar_window_session_insert_rectangle : stellar_window_session_insert_frame
        var request: UInt64 = 0
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: parent.uuid) { parentBytes in
                insert(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress,
                    parentBytes.bindMemory(to: UInt8.self).baseAddress, revision, Float(rect.minX), Float(rect.minY),
                    Float(rect.width), Float(rect.height), &request)
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func moveFrame(handle: UInt64, session: UInt64, view: UUID, node: UUID,
                          revision: UInt64, delta: CGVector) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: node.uuid) { nodeBytes in
                stellar_window_session_move_frame(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress,
                    nodeBytes.bindMemory(to: UInt8.self).baseAddress, revision, Float(delta.dx), Float(delta.dy), &request)
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func resizeFrame(handle: UInt64, session: UInt64, view: UUID, node: UUID,
                          revision: UInt64, size: CGSize) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: node.uuid) { nodeBytes in
                stellar_window_session_resize_frame(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress,
                    nodeBytes.bindMemory(to: UInt8.self).baseAddress, revision, Float(size.width), Float(size.height), &request)
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func setFrameBackground(handle: UInt64, session: UInt64, view: UUID, node: UUID,
                          revision: UInt64, background: InspectorColor?) -> Result<UInt64, CoreRuntimeCallError> {
        var color = StellarCanvasColor()
        if let background {
            color.space = UInt32(background.space.rawValue)
            color.rgba = (Float(background.red), Float(background.green), Float(background.blue), Float(background.alpha))
        }
        var request: UInt64 = 0
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: node.uuid) { nodeBytes in
                stellar_window_session_set_frame_background(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress,
                    nodeBytes.bindMemory(to: UInt8.self).baseAddress, revision, color, &request)
            }
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

    #if DEBUG
    /// 저장 fixture의 실제 Rust transaction·재로드 검사. 일반 앱 경로는 호출하지 않습니다.
    static func checkWindowEdit(
        handle: UInt64, session: UInt64, node: UUID, name: String, verify: Bool, delayMS: UInt32
    ) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let bytes = Array(name.utf8)
        let status = withUnsafeBytes(of: node.uuid) { id in
            bytes.withUnsafeBufferPointer { name in
                stellar_window_session_check_edit(
                    handle, session, id.baseAddress?.assumingMemoryBound(to: UInt8.self),
                    name.baseAddress, name.count, verify ? 1 : 0, delayMS, &request
                )
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }
    #endif

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
struct CoreGitChange: Equatable {
    let componentPath: String?
    let document: UUID?
    let node: UUID
    let head: String
    let index: String
    let working: String
    let staged: Bool
    let unstaged: Bool
}

enum CoreRuntimeEvent: Equatable {
    case componentCreated(request: UInt64, session: UInt64, filename: String)
    case projectCreated(request: UInt64, gitState: UInt32)
    case projectOpened(request: UInt64, project: UInt64, projectID: UUID, name: String, root: String)
    case projectClosed(request: UInt64, project: UInt64, closedDocuments: UInt32)
    case projectTrust(request: UInt64, project: UInt64, trusted: Bool)
    case gitNames(request: UInt64, session: UInt64, rows: UInt32, other: Bool, applied: Bool)
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
        case .componentCreated(let request, _, _), .projectCreated(let request, _), .projectOpened(let request, _, _, _, _), .projectClosed(let request, _, _), .projectTrust(let request, _, _),
             .gitNames(let request, _, _, _, _), .documentOpened(let request, _, _, _, _), .documentClosed(let request, _),
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
        case UInt32(STELLAR_EVENT_COMPONENT_CREATED):
            guard let request = reader.u64(), let session = reader.u64(), let filename = reader.text() else { return .undecodable(kind: kind) }
            event = .componentCreated(request: request, session: session, filename: filename)
        case UInt32(STELLAR_EVENT_PROJECT_CREATED):
            guard let request = reader.u64(), let gitState = reader.u32() else { return .undecodable(kind: kind) }
            event = .projectCreated(request: request, gitState: gitState)
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
        case UInt32(STELLAR_EVENT_GIT_NAMES):
            guard let request = reader.u64(), let session = reader.u64(), let rows = reader.u32(),
                  let other = reader.bool(), let applied = reader.bool() else { return .undecodable(kind: kind) }
            event = .gitNames(request: request, session: session, rows: rows, other: other, applied: applied)
        case UInt32(STELLAR_EVENT_PROJECT_TRUST):
            event = reader.u64().flatMap { request in
                reader.u64().flatMap { project in
                    reader.bool().map { .projectTrust(request: request, project: project, trusted: $0) }
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
    static func decodeName(_ bytes: ArraySlice<UInt8>) -> CoreGitChange? {
        var reader = PayloadReader(bytes: bytes)
        guard let document = reader.uuid(), let node = reader.uuid(), let staged = reader.bool(),
              let head = reader.text(), let index = reader.text(), let working = reader.text(), reader.isAtEnd else { return nil }
        return CoreGitChange(componentPath: nil, document: document, node: node, head: head, index: index, working: working,
                             staged: staged, unstaged: index != working)
    }

    static func decodeComponentRow(_ bytes: ArraySlice<UInt8>) -> CoreGitChange? {
        var reader = PayloadReader(bytes: bytes)
        guard let id = reader.uuid(), let staged = reader.bool(), let unstaged = reader.bool(),
              let path = reader.text(), let head = reader.text(), let index = reader.text(),
              let working = reader.text(), reader.isAtEnd else { return nil }
        return CoreGitChange(componentPath: path, document: nil, node: id, head: head, index: index,
                             working: working, staged: staged, unstaged: unstaged)
    }

    struct PayloadReader {
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
