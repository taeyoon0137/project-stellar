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

    /// 한 게시 snapshot의 선택과 단일 선택의 Inspector 값. core가 문서에서 읽습니다(document thread를 기다리지 않음).
    static func selectedNode(handle: UInt64, session: UInt64) -> StellarSelectedNode? {
        var node = StellarSelectedNode()
        return stellar_window_selected_node(handle, session, &node) == STELLAR_OK ? node : nil
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

    static func cloneProject(handle: UInt64, remote: String, target: String) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = Array(remote.utf8).withUnsafeBufferPointer { url in
            Array(target.utf8).withUnsafeBufferPointer { path in
                stellar_project_clone(handle, url.baseAddress, url.count, path.baseAddress, path.count, &request)
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func cancelClone(handle: UInt64, request: UInt64) {
        _ = stellar_project_clone_cancel(handle, request)
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

    static func canvasInfo(handle: UInt64, session: UInt64, size: CGSize) -> StellarCanvasInfo? {
        var info = StellarCanvasInfo()
        let status = stellar_window_canvas(handle, session, Float(size.width), Float(size.height), &info, nil, 0)
        return status == STELLAR_OK ? info : nil
    }

    /// 도형과 glyph outline은 같은 revision/row 식별자와 move·line·curve·close 경로 요소를 사용합니다.
    static func canvasShapePath(handle: UInt64, session: UInt64, query: StellarCanvasTextQuery)
        -> ([StellarCanvasTextElement], UInt32)? {
        var count = 0, flags: UInt32 = 0
        guard stellar_canvas_shape_path(handle, session, query, nil, 0, &count, &flags) == STELLAR_OK else { return nil }
        var elements = [StellarCanvasTextElement](repeating: StellarCanvasTextElement(), count: count)
        let status = elements.withUnsafeMutableBufferPointer {
            stellar_canvas_shape_path(handle, session, query, $0.baseAddress, $0.count, &count, &flags)
        }
        return status == STELLAR_OK ? (elements, flags) : nil
    }

    /// 이 창 session에 제안할 복구본. 파일을 읽으므로 main thread 밖에서 부릅니다. 실패하면 nil입니다.
    static func probeRecovery(handle: UInt64, session: UInt64) -> StellarRecoveryCandidate? {
        var candidate = StellarRecoveryCandidate()
        guard stellar_window_recovery_probe(handle, session, &candidate) == STELLAR_OK,
              candidate.version == UInt32(STELLAR_RECOVERY_CANDIDATE_VERSION) else { return nil }
        return candidate
    }

    static func recoverDocument(handle: UInt64, session: UInt64, candidate: StellarRecoveryCandidate,
                                action: UInt32) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        var candidate = candidate
        let status = stellar_window_recovery_apply(handle, session, &candidate, action, &request)
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
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

    static func zoomAt(handle: UInt64, session: UInt64, anchor: CGPoint, fitFactor: Double?, zoomOut: Bool) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = stellar_window_session_zoom_at(handle, session, anchor.x, anchor.y, fitFactor ?? 0, zoomOut ? 1 : 0, &request)
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    /// 창 session 대상 원시 입력 제출. 새 요청이면 그 id, 요청이 생기지 않았으면 0입니다.
    static func submitWindowEvent(handle: UInt64, session: UInt64, event: StellarMacEvent) -> Result<UInt64, CoreRuntimeCallError> {
        var raw = event, request: UInt64 = 0
        let status = stellar_window_session_submit_mac_event(handle, session, &raw, &request)
        guard status == STELLAR_OK else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    /// 화면 중심 기준 `anchor` point를 고정하고 배율을 `factor`배 합니다(핀치·⌘ 스크롤, ADR-0071).
    static func zoomBy(handle: UInt64, session: UInt64, anchor: CGPoint, factor: Double) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = stellar_window_session_zoom_by(handle, session, anchor.x, anchor.y, factor, &request)
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
    static func copySubtree(handle: UInt64, target: StellarSelectionTarget) -> Data? {
        var target = target
        func read(_ data: inout Data, count: inout Int) -> StellarStatus {
            data.withUnsafeMutableBytes { bytes in
                stellar_window_copy_selection(handle, &target, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, &count)
            }
        }
        var data = Data(count: 4096)
        var count = 0
        var status = read(&data, count: &count)
        if status == STELLAR_BUFFER_TOO_SMALL {
            guard count > data.count, count <= 16 * 1024 * 1024 else { return nil }
            data = Data(count: count)
            status = read(&data, count: &count)
        }
        guard status == STELLAR_OK, count > 0, count <= data.count else { return nil }
        data.count = count
        return data
    }

    static func pasteSubtree(handle: UInt64, target: StellarSelectionTarget, data: Data) -> Result<UInt64, CoreRuntimeCallError> {
        var target = target, request: UInt64 = 0
        let status = data.withUnsafeBytes { bytes in
            stellar_window_paste_selection(handle, &target, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count, &request)
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func placePNG(handle: UInt64, target: StellarSelectionTarget, parent: UUID, position: CGPoint,
                         path: String, digest: Data) -> Result<UInt64, CoreRuntimeCallError> {
        var target = target, request: UInt64 = 0
        let status = withUnsafeBytes(of: parent.uuid) { id in
            digest.withUnsafeBytes { digest in
                Array(path.utf8).withUnsafeBufferPointer { path in
                    stellar_window_place_png(handle, &target, id.bindMemory(to: UInt8.self).baseAddress,
                        Float(position.x), Float(position.y), path.baseAddress, path.count, digest.bindMemory(to: UInt8.self).baseAddress, &request)
                }
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func insertFrame(handle: UInt64, session: UInt64, view: UUID, parent: UUID, revision: UInt64,
                            rect: CGRect) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: parent.uuid) { parentBytes in
                stellar_window_session_insert_frame(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress,
                    parentBytes.bindMemory(to: UInt8.self).baseAddress, revision, Float(rect.minX), Float(rect.minY),
                    Float(rect.width), Float(rect.height), &request)
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    /// 새 도형의 Fill·Stroke는 `paint`(사용자 앱 설정)로 만듭니다. core는 기본 색을 정하지 않습니다.
    static func insertShape(handle: UInt64, session: UInt64, view: UUID, parent: UUID, revision: UInt64,
                            kind: InspectorShapeKind, start: CGPoint, end: CGPoint,
                            paint: CanvasShapeDefaults.Paint) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        var style = StellarShapeStyle()
        style.fill = StellarCanvasColor(paint.fill)
        style.stroke = StellarCanvasColor(paint.stroke)
        style.stroke_width = Float(paint.strokeWidth)
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: parent.uuid) { parentBytes in
                stellar_window_session_insert_shape(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress,
                    parentBytes.bindMemory(to: UInt8.self).baseAddress, revision, kind.rawValue,
                    Float(start.x), Float(start.y), Float(end.x), Float(end.y), &style, &request)
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    /// 그린 행(`leaf`, nil이면 모든 Frame 밖)에서 고를 선택을 코어 규칙으로 정합니다(ADR-0071).
    static func canvasPick(handle: UInt64, session: UInt64, view: UUID, leaf: UUID?,
                           flags: UInt32) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            let viewPointer = viewBytes.bindMemory(to: UInt8.self).baseAddress
            guard let leaf else { return stellar_window_canvas_pick(handle, session, viewPointer, nil, flags, &request) }
            return withUnsafeBytes(of: leaf.uuid) { leafBytes in
                stellar_window_canvas_pick(handle, session, viewPointer, leafBytes.bindMemory(to: UInt8.self).baseAddress,
                                           flags, &request)
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    /// world 영역 두 꼭짓점으로 root의 직계 자식을 고릅니다(ADR-0071).
    static func canvasPickArea(handle: UInt64, session: UInt64, view: UUID, from: CGPoint, to: CGPoint,
                               flags: UInt32) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            stellar_window_canvas_pick_area(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress,
                                            Float(from.x), Float(from.y), Float(to.x), Float(to.y), flags, &request)
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    /// 최신 게시 snapshot의 선택 상자와 그 문서 revision. document thread를 기다리지 않습니다.
    static func canvasSelection(handle: UInt64, session: UInt64) -> (items: [StellarCanvasSelectionItem], revision: UInt64)? {
        var count = 0
        var revision: UInt64 = 0
        guard stellar_window_canvas_selection(handle, session, nil, 0, &count, &revision) == STELLAR_OK else { return nil }
        guard count > 0 else { return ([], revision) }
        var items = [StellarCanvasSelectionItem](repeating: StellarCanvasSelectionItem(), count: count)
        let status = items.withUnsafeMutableBufferPointer {
            stellar_window_canvas_selection(handle, session, $0.baseAddress, $0.count, &count, &revision)
        }
        guard status == STELLAR_OK, count <= items.count else { return nil }
        return (Array(items.prefix(count)), revision)
    }

    /// 하나뿐인 선택 node의 상자를 지금 local 좌표 `rect`로 바꿉니다(크기 핸들, ADR-0071).
    static func canvasResize(handle: UInt64, session: UInt64, view: UUID, node: UUID,
                             rect: CGRect) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: node.uuid) { nodeBytes in
                stellar_window_canvas_resize(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress,
                                             nodeBytes.bindMemory(to: UInt8.self).baseAddress, Float(rect.minX), Float(rect.minY),
                                             Float(rect.width), Float(rect.height), &request)
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    /// 하나뿐인 선택 node의 모서리 반경(corner nil이면 네 모서리, ADR-0071).
    static func canvasRadius(handle: UInt64, session: UInt64, view: UUID, node: UUID, corner: Int?,
                             radius: CGFloat) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: node.uuid) { nodeBytes in
                stellar_window_canvas_radius(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress,
                                             nodeBytes.bindMemory(to: UInt8.self).baseAddress, Int32(corner ?? -1),
                                             Float(radius), &request)
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    /// 하나뿐인 선택 Line의 끝점 하나를 local `point`로 옮깁니다(ADR-0071).
    static func canvasLineEnd(handle: UInt64, session: UInt64, view: UUID, node: UUID, end: Bool,
                              point: CGPoint) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: node.uuid) { nodeBytes in
                stellar_window_canvas_line_end(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress,
                                               nodeBytes.bindMemory(to: UInt8.self).baseAddress, end ? 1 : 0,
                                               Float(point.x), Float(point.y), &request)
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    /// 선택 전부를 `delta`만큼 옮깁니다(ADR-0071). anchor는 드래그를 시작한 행(키 이동은 nil)입니다.
    static func canvasMove(handle: UInt64, session: UInt64, view: UUID, anchor: UUID?, delta: CGVector,
                           flags: UInt32) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            let viewPointer = viewBytes.bindMemory(to: UInt8.self).baseAddress
            guard let anchor else {
                return stellar_window_canvas_move(handle, session, viewPointer, nil, Float(delta.dx), Float(delta.dy),
                                                  flags, &request)
            }
            return withUnsafeBytes(of: anchor.uuid) { anchorBytes in
                stellar_window_canvas_move(handle, session, viewPointer, anchorBytes.bindMemory(to: UInt8.self).baseAddress,
                                           Float(delta.dx), Float(delta.dy), flags, &request)
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
    case projectCreated(request: UInt64, gitState: UInt32)
    case projectOpened(request: UInt64, project: UInt64, projectID: UUID, name: String, root: String)
    case projectClosed(request: UInt64, project: UInt64, closedDocuments: UInt32)
    case projectTrust(request: UInt64, project: UInt64, trusted: Bool)
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
        case .projectCreated(let request, _), .projectOpened(let request, _, _, _, _), .projectClosed(let request, _, _), .projectTrust(let request, _, _),
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

extension StellarSelectionTarget {
    func matches(_ other: StellarSelectionTarget?) -> Bool {
        guard let other else { return false }
        return session == other.session && revision == other.revision && history_sequence == other.history_sequence
            && selection_count == other.selection_count && UUID(uuid: view) == UUID(uuid: other.view)
    }
}
