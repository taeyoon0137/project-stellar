import Foundation

/// 앱 수명 동안 코어 runtime handle 하나를 소유합니다(II.5.3).
///
/// 생성은 `init`에서 1회, 파괴는 `shutdown()`에서 정확히 1회입니다.
/// `shutdown()`을 여러 번 호출해도 `stellar_runtime_destroy`는 한 번만 실행됩니다
/// (창 닫기·앱 종료 경로가 겹쳐도 double free가 없도록).
final class RuntimeHost {
    private(set) var handle: UInt64 = 0
    private(set) var created: Bool = false
    private var destroyed = false

    init() {
        #if DEBUG
        // 검사 실행(`--…-check`)은 dirty·stale 상태를 시간에 걸쳐 단정하므로 앱 runtime을 자동 저장 없이 시작합니다
        // (ADR-0072 10절). C ABI가 아니라 Debug 코어만 읽는 검사 전용 환경 변수로 알립니다.
        if CommandLine.arguments.contains(where: { $0.hasPrefix("--") && $0.hasSuffix("-check") }) {
            setenv("STELLAR_CHECK_DISABLE_AUTOSAVE", "1", 1)
        }
        #endif
        created = (stellar_runtime_create(&handle) == STELLAR_OK)
    }

    /// 소유한 handle의 코어 버전 문자열.
    func version() -> String? {
        guard created else { return nil }
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

    /// 앱 비활성화 flush(ADR-0072): 저장되지 않은 완료 편집을 debounce 없이 저장하도록 코어에 알립니다.
    /// 기다리지 않습니다. 저장 시점·대상 판단은 코어 runtime이 합니다.
    func flushAutosave() {
        guard created, !destroyed else { return }
        _ = stellar_runtime_flush_autosave(handle)
    }

    /// runtime을 파괴합니다(중복 호출 무시 — destroy는 정확히 1회).
    func shutdown() {
        guard created, !destroyed else { return }
        destroyed = true
        _ = stellar_runtime_destroy(handle)
    }

    deinit {
        shutdown()
    }
}
