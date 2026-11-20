import Foundation

struct SavedWebDiagnostic: Sendable {
    let node: UUID
    let error: Bool
    let code: String
    let message: String
}

struct SavedWebExport: Sendable {
    let html: String?
    let error: String?
    let diagnostics: [SavedWebDiagnostic]
}

extension CoreRuntime {
    /// 저장된 generation을 읽고 HTML을 만듭니다. UI thread에서는 호출하지 않습니다.
    static func exportSavedWeb(handle: UInt64, session: UInt64) -> SavedWebExport {
        precondition(!Thread.isMainThread)
        var info = StellarWebExportInfo()
        var rows = [StellarWebDiagnostic](repeating: StellarWebDiagnostic(), count: 64)
        var bytes = [UInt8](repeating: 0, count: 64 * 1024)
        func read() -> StellarStatus {
            rows.withUnsafeMutableBufferPointer { rowBuffer in
                bytes.withUnsafeMutableBufferPointer { textBuffer in
                    stellar_window_export_web(handle, session, &info, rowBuffer.baseAddress, rowBuffer.count,
                                              textBuffer.baseAddress, textBuffer.count)
                }
            }
        }
        var status = read()
        if status == STELLAR_BUFFER_TOO_SMALL {
            rows = [StellarWebDiagnostic](repeating: StellarWebDiagnostic(), count: max(info.diagnostic_count, 1))
            bytes = [UInt8](repeating: 0, count: max(info.text_len, 1))
            status = read()
        }
        guard status == STELLAR_OK, info.text_len <= bytes.count, info.diagnostic_count <= rows.count else {
            return SavedWebExport(html: nil, error: "Couldn’t generate HTML (\(status)).", diagnostics: [])
        }
        var offset = 0
        func take(_ count: Int) -> String? {
            guard count >= 0, count <= info.text_len - offset else { return nil }
            defer { offset += count }
            return String(bytes: bytes[offset..<(offset + count)], encoding: .utf8)
        }
        let html = info.has_html != 0 ? take(info.html_len) : nil
        let error = info.error_len > 0 ? take(info.error_len) : nil
        var diagnostics: [SavedWebDiagnostic] = []
        for row in rows.prefix(info.diagnostic_count) {
            guard let code = take(Int(row.code_len)), let message = take(Int(row.message_len)) else {
                return SavedWebExport(html: nil, error: "Invalid HTML diagnostic.", diagnostics: [])
            }
            diagnostics.append(SavedWebDiagnostic(node: UUID(uuid: row.node), error: row.error != 0,
                                                  code: code, message: message))
        }
        return SavedWebExport(html: html, error: error, diagnostics: diagnostics)
    }
}
