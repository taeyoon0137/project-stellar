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
        created = (stellar_runtime_create(&handle) == STELLAR_OK)
    }

    /// 소유한 handle의 코어 버전 문자열.
    func version() -> String? {
        guard created else { return nil }
        return CoreRuntime.version(handle: handle)
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
