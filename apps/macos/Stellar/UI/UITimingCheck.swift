import Foundation

/// 공유 CI-MAC에서 Debug 지연 초과를 Actions 경고로 기록합니다. 로컬 기본 검사는 오류를 반환합니다.
enum UITimingCheck {
    struct Exceeded: Error, CustomStringConvertible {
        let description: String
    }

    static func require(_ condition: Bool, _ message: @autoclosure () -> String) throws {
        guard !condition else { return }
        if ProcessInfo.processInfo.environment["STELLAR_UI_TIMING"] == "report" {
            print("::warning::UI timing — \(message())")
        } else {
            throw Exceeded(description: message())
        }
    }
}
