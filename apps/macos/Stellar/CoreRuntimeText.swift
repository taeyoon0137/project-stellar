import CoreGraphics
import Foundation

/// 캔버스 Text 편집 요청(ADR-0067 §6). 조합 상태는 코어가 소유하고 셸은 token과 문서 scalar 범위만 넘깁니다.
extension CoreRuntime {
    private static func request(_ call: (UnsafeMutablePointer<UInt64>) -> StellarStatus) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = call(&request)
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    /// 조합을 시작합니다. 새 Text면 `node`는 부모 Frame이고 결과의 node가 확정 때 만들 ID입니다.
    static func beginText(handle: UInt64, session: UInt64, view: UUID, node: UUID, revision: UInt64,
                          newAt origin: CGPoint?, start: UInt32, end: UInt32) -> Result<(token: UInt64, node: UUID), CoreRuntimeCallError> {
        var created = UUID().uuid
        let result = withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: node.uuid) { nodeBytes in
                request { out in
                    stellar_text_begin(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress,
                                       nodeBytes.bindMemory(to: UInt8.self).baseAddress, revision, origin == nil ? 0 : 1,
                                       Float(origin?.x ?? 0), Float(origin?.y ?? 0), start, end, &created, out)
                }
            }
        }
        return result.map { ($0, UUID(uuid: created)) }
    }

    static func markText(handle: UInt64, session: UInt64, token: UInt64, text: String) -> Result<UInt64, CoreRuntimeCallError> {
        Array(text.utf8).withUnsafeBufferPointer { bytes in
            request { stellar_text_mark(handle, session, token, bytes.baseAddress, bytes.count, $0) }
        }
    }

    static func commitText(handle: UInt64, session: UInt64, token: UInt64, text: String) -> Result<UInt64, CoreRuntimeCallError> {
        Array(text.utf8).withUnsafeBufferPointer { bytes in
            request { stellar_text_commit(handle, session, token, bytes.baseAddress, bytes.count, $0) }
        }
    }

    static func cancelText(handle: UInt64, session: UInt64, token: UInt64) -> Result<UInt64, CoreRuntimeCallError> {
        request { stellar_text_cancel(handle, session, token, $0) }
    }

    static func replaceText(handle: UInt64, session: UInt64, view: UUID, node: UUID, revision: UInt64,
                            start: UInt32, end: UInt32, text: String) -> Result<UInt64, CoreRuntimeCallError> {
        withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: node.uuid) { nodeBytes in
                Array(text.utf8).withUnsafeBufferPointer { bytes in
                    request { out in
                        stellar_text_replace(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress,
                                             nodeBytes.bindMemory(to: UInt8.self).baseAddress, revision, start, end,
                                             bytes.baseAddress, bytes.count, out)
                    }
                }
            }
        }
    }

    /// 저장된 본문. 셸 worker에서만 부릅니다(코어 document thread 결과를 기다림).
    static func textContent(handle: UInt64, session: UInt64, view: UUID, node: UUID, revision: UInt64) -> String? {
        withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: node.uuid) { nodeBytes in
                let v = viewBytes.bindMemory(to: UInt8.self).baseAddress, n = nodeBytes.bindMemory(to: UInt8.self).baseAddress
                var bytes = [UInt8](repeating: 0, count: 4096)
                var count = 0
                var status = bytes.withUnsafeMutableBufferPointer {
                    stellar_text_content(handle, session, v, n, revision, $0.baseAddress, $0.count, &count)
                }
                if status == STELLAR_BUFFER_TOO_SMALL {
                    bytes = [UInt8](repeating: 0, count: count)
                    status = bytes.withUnsafeMutableBufferPointer {
                        stellar_text_content(handle, session, v, n, revision, $0.baseAddress, $0.count, &count)
                    }
                }
                guard status == STELLAR_OK else { return nil }
                return String(bytes: bytes.prefix(count), encoding: .utf8)
            }
        }
    }
}
