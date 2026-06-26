import CoreGraphics
import Foundation

/// 캔버스 drawable 전달 구조를 headless로 검증합니다(II.5.4).
///
/// 여러 (point 크기, scale) fixture에 대해 pixel == point × scale을 확인하고,
/// 같은 값을 코어로 전달(`set_drawable_size`)한 뒤 다시 읽어(`drawable_size`) round-trip을 확인합니다.
enum CanvasCheck {
    static func run() -> Bool {
        func fail(_ message: String) -> Bool {
            FileHandle.standardError.write(Data("canvas 실패: \(message)\n".utf8))
            return false
        }

        let host = RuntimeHost()
        guard host.created else { return fail("runtime create 실패") }
        defer { host.shutdown() }

        let fixtures: [(points: CGSize, scale: Double)] = [
            (CGSize(width: 800, height: 600), 1.0),
            (CGSize(width: 800, height: 600), 2.0),
            (CGSize(width: 1024, height: 768), 2.0),
            (CGSize(width: 640, height: 480), 3.0),
        ]

        for fixture in fixtures {
            let expectedW = UInt32((fixture.points.width * fixture.scale).rounded())
            let expectedH = UInt32((fixture.points.height * fixture.scale).rounded())

            let computed = CanvasMetrics.pixelSize(pointSize: fixture.points, scale: fixture.scale)
            guard computed.width == expectedW, computed.height == expectedH else {
                return fail("pixel != point×scale (\(fixture.points) @\(fixture.scale))")
            }

            guard
                stellar_runtime_set_drawable_size(host.handle, computed.width, computed.height, fixture.scale)
                    == STELLAR_OK
            else {
                return fail("set_drawable_size 실패")
            }

            var width: UInt32 = 0
            var height: UInt32 = 0
            var scale = 0.0
            guard stellar_runtime_drawable_size(host.handle, &width, &height, &scale) == STELLAR_OK else {
                return fail("drawable_size 읽기 실패")
            }
            guard width == expectedW, height == expectedH, scale == fixture.scale else {
                return fail("round-trip mismatch (\(width)x\(height) @\(scale))")
            }
        }

        print("canvas ok: pixel == point×scale, core round-trip for all fixtures")
        return true
    }
}
