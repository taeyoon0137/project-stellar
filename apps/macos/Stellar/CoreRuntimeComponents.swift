import Foundation

extension CoreRuntime {
    /// 요청 없는 event 준비 신호 fd(호출자 소유). 실패하면 -1입니다.
    static func eventReadyFD(handle: UInt64) -> Int32 {
        stellar_runtime_event_ready_fd(handle)
    }

    static func reloadComponentCanvas(handle: UInt64, session: UInt64) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = stellar_window_reload_component_canvas(handle, session, &request)
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }
}

extension StellarCanvasColor {
    init(_ background: InspectorColor?) {
        self.init()
        if let background {
            space = UInt32(background.space.rawValue)
            rgba = (Float(background.red), Float(background.green), Float(background.blue), Float(background.alpha))
        }
    }
}
