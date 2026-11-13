import AppKit

/// 가운데 콘텐츠 슬롯: 기존 `CanvasView`(II.5.4)를 그대로 감싸는 캔버스 호스트.
///
/// `CanvasView`는 수정하지 않고 생성만 합니다. drawable 크기·scale 전달은 `CanvasView`의 기존
/// 경로(`stellar_runtime_set_drawable_size`)가 그대로 맡습니다(가이드 2.3, 9장).
///
/// 캔버스는 safe area 안에만 둡니다. 툴바 glass와 사이드바·인스펙터 아래로 확장할지는 열린 결정
/// O2이고, 결정 전에는 겹치지 않게 둡니다. 겹치면 drawable 크기, hit test 영역, 렌더 비용 계산이
/// 달라지기 때문입니다(가이드 2.3, 아키텍처 §6.2).
final class ShellCanvasHostViewController: NSViewController, ShellContentSlot {
    /// 감싼 캔버스 뷰.
    let canvasView: CanvasView

    var paneViewController: NSViewController { self }
    let paneTitle = "Canvas"

    /// - Parameter runtimeHandle: drawable 변경을 전달할 코어 runtime handle. nil이면 전달하지 않습니다.
    init(runtimeHandle: UInt64?) {
        canvasView = CanvasView(frame: .zero)
        canvasView.runtimeHandle = runtimeHandle
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("ShellCanvasHostViewController는 코드로만 생성합니다")
    }

    override func loadView() {
        let root = NSView()
        root.setAccessibilityElement(true)
        root.setAccessibilityRole(.group)
        root.setAccessibilityLabel(paneTitle)

        // 캔버스 상태(선택·도구) 노출은 XII.8.1 범위입니다. 여기서는 역할과 이름만 지정합니다.
        canvasView.setAccessibilityElement(true)
        canvasView.setAccessibilityRole(.layoutArea)
        canvasView.setAccessibilityLabel(paneTitle)
        canvasView.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(canvasView)

        let safeArea = root.safeAreaLayoutGuide
        NSLayoutConstraint.activate([
            canvasView.topAnchor.constraint(equalTo: safeArea.topAnchor),
            canvasView.bottomAnchor.constraint(equalTo: safeArea.bottomAnchor),
            canvasView.leadingAnchor.constraint(equalTo: safeArea.leadingAnchor),
            canvasView.trailingAnchor.constraint(equalTo: safeArea.trailingAnchor),
        ])
        view = root
    }
}
