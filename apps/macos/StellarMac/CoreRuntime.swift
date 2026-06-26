import Foundation

/// Rust 코어(stellar-ffi)를 호출하는 FFI 헬퍼.
///
/// FFI 규칙(정수 handle, `StellarStatus` 반환, caller buffer 출력)을 그대로 따릅니다.
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
}
