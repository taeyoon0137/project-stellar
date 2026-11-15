import CoreGraphics
import Foundation

extension CoreRuntime {
    static func setDocumentFont(handle: UInt64, session: UInt64, revision: UInt64, path: String, face: UInt32,
                                size: UInt32, fill: StellarCanvasColor) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = Array(path.utf8).withUnsafeBufferPointer { bytes in
            stellar_window_set_document_font(handle, session, revision, bytes.baseAddress, bytes.count, face, size, fill, &request)
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func insertText(handle: UInt64, session: UInt64, view: UUID, parent: UUID, revision: UInt64, origin: CGPoint,
                           text: String) -> Result<(request: UInt64, node: UUID), CoreRuntimeCallError> {
        var request: UInt64 = 0, created = UUID().uuid
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: parent.uuid) { parentBytes in
                Array(text.utf8).withUnsafeBufferPointer { bytes in
                    stellar_window_insert_text(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress,
                        parentBytes.bindMemory(to: UInt8.self).baseAddress, revision, Float(origin.x), Float(origin.y),
                        bytes.baseAddress, bytes.count, &created, &request)
                }
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success((request, UUID(uuid: created)))
    }

    static func beginText(handle: UInt64, session: UInt64, view: UUID, node: UUID, revision: UInt64,
                          start: UInt32, end: UInt32) -> (status: StellarStatus, token: UInt64) {
        var token: UInt64 = 0
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: node.uuid) { nodeBytes in
                stellar_text_begin(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress,
                                   nodeBytes.bindMemory(to: UInt8.self).baseAddress, revision, start, end, &token)
            }
        }
        return (status, token)
    }

    static func commitText(handle: UInt64, session: UInt64, token: UInt64, text: String) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = Array(text.utf8).withUnsafeBufferPointer { bytes in
            stellar_text_commit(handle, session, token, bytes.baseAddress, bytes.count, &request)
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func cancelText(handle: UInt64, session: UInt64, token: UInt64) {
        _ = stellar_text_cancel(handle, session, token)
    }

    static func textAtCaret(handle: UInt64, session: UInt64, view: UUID, node: UUID, revision: UInt64, caret: UInt32) -> String? {
        let (status, token) = beginText(handle: handle, session: session, view: view, node: node, revision: revision, start: caret, end: caret)
        guard status == STELLAR_OK else { return nil }
        defer { cancelText(handle: handle, session: session, token: token) }
        var count = 0
        guard stellar_text_preview(handle, session, token, nil, 0, &count) == STELLAR_OK else { return nil }
        var bytes = [UInt8](repeating: 0, count: count)
        let loaded = bytes.withUnsafeMutableBufferPointer {
            stellar_text_preview(handle, session, token, $0.baseAddress, $0.count, &count)
        }
        guard loaded == STELLAR_OK else { return nil }
        return String(bytes: bytes.prefix(count), encoding: .utf8)
    }
}
