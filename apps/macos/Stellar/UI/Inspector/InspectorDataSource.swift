import Foundation

// 속성 패널과 코어 사이의 경계.
//
// 코어 → 셸: 확정 선택의 속성 snapshot(`InspectorSnapshot`)을 값으로 받습니다.
// 셸 → 코어: 속성 이름과 CSS 값 텍스트 또는 수치 delta, 편집 단계만 보냅니다. 해석·검증·Undo 묶음은 코어가 합니다.

/// 속성 패널 데이터 공급자. 모든 호출과 콜백은 메인 스레드에서 일어납니다.
@MainActor
protocol InspectorDataSource: AnyObject {
    /// 가장 최근 snapshot.
    var snapshot: InspectorSnapshot { get }
    /// 새 snapshot이 준비되면 호출할 콜백. 속성 패널이 설정합니다.
    var snapshotHandler: (@MainActor (InspectorSnapshot) -> Void)? { get set }
    /// 확정 선택 전체에 편집을 보냅니다. `completion`은 성공이면 nil, 거부되면 진단 문구입니다.
    func edit(_ name: String, _ input: InspectorInput, phase: InspectorEditPhase,
              completion: @escaping @MainActor (String?) -> Void)
}
