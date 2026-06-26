import Foundation

/// Rust 코어(stellar-ffi)를 감싸는 얇은 Swift 래퍼.
///
/// FFI 규칙(정수 handle, `StellarStatus` 반환, caller buffer 출력)을 그대로 따릅니다.
/// II.5.2 단계에서는 버전 조회만 노출합니다. lifecycle 관리는 II.5.3에서 확장합니다.
enum CoreRuntime {
    /// 코어 runtime을 잠깐 생성해 버전 문자열을 읽고 파괴합니다.
    static func version() -> String? {
        var handle: UInt64 = 0
        guard stellar_runtime_create(&handle) == STELLAR_OK else { return nil }
        defer { _ = stellar_runtime_destroy(handle) }

        // 1차 호출: 필요한 바이트 길이만 조회(buf = NULL).
        var needed = 0
        guard stellar_runtime_version(handle, nil, 0, &needed) == STELLAR_OK else { return nil }

        // 2차 호출: 호출자 buffer에 UTF-8 바이트를 채움.
        var buffer = [UInt8](repeating: 0, count: needed)
        var written = 0
        let status = buffer.withUnsafeMutableBufferPointer { raw in
            stellar_runtime_version(handle, raw.baseAddress, needed, &written)
        }
        guard status == STELLAR_OK else { return nil }
        return String(decoding: buffer[0..<written], as: UTF8.self)
    }
}
