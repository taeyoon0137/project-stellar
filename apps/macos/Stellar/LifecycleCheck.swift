import Foundation

/// 앱 lifecycle의 create/destroy 1:1 계약을 headless로 검증합니다(II.5.3).
///
/// `RuntimeHost`(AppDelegate가 실제 launch/terminate에서 쓰는 것과 동일한 타입)를
/// 생성·파괴하며 `stellar_runtime_live_count()`로 살아 있는 handle 수를 확인합니다.
enum LifecycleCheck {
    static func run() -> Bool {
        func fail(_ message: String) -> Bool {
            FileHandle.standardError.write(Data("lifecycle 실패: \(message)\n".utf8))
            return false
        }

        let baseline = stellar_runtime_live_count()

        let host = RuntimeHost()
        guard host.created else { return fail("runtime create 실패") }
        guard stellar_runtime_live_count() == baseline + 1 else {
            return fail("create 후 live_count가 +1이 아님")
        }

        host.shutdown()
        guard stellar_runtime_live_count() == baseline else {
            return fail("destroy 후 live_count가 baseline으로 복귀하지 않음")
        }

        // 중복 shutdown은 추가 destroy를 일으키지 않아야 함(정확히 1회).
        host.shutdown()
        guard stellar_runtime_live_count() == baseline else {
            return fail("중복 shutdown이 count를 바꿈")
        }

        print("lifecycle ok: create/destroy 1:1 (baseline \(baseline))")
        return true
    }
}
