import CoreGraphics

/// 캔버스 drawable 메트릭 계산(II.5.4).
///
/// drawable(backing) 픽셀 크기 = point 크기 × backing scale.
/// 이 관계가 셸이 코어에 전달하는 값의 정의이며, resize/scale 변경 시 그대로 적용됩니다.
enum CanvasMetrics {
    static func pixelSize(pointSize: CGSize, scale: Double) -> (width: UInt32, height: UInt32) {
        let width = max(0.0, (Double(pointSize.width) * scale).rounded())
        let height = max(0.0, (Double(pointSize.height) * scale).rounded())
        return (UInt32(width), UInt32(height))
    }
}
