import Foundation

extension CoreRuntime {
    static func componentWatchFD(handle: UInt64, session: UInt64) -> Int32 {
        stellar_window_component_watch_fd(handle, session)
    }

    static func reloadComponentCanvas(handle: UInt64, session: UInt64) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = stellar_window_reload_component_canvas(handle, session, &request)
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func selectedDefinition(handle: UInt64, session: UInt64) -> UUID? {
        var id: uuid_t = (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
        let status = withUnsafeMutableBytes(of: &id) {
            stellar_window_selected_definition(handle, session, $0.bindMemory(to: UInt8.self).baseAddress)
        }
        return status == STELLAR_OK ? UUID(uuid: id) : nil
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
