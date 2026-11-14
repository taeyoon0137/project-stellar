import Foundation

// 인스펙터와 코어 사이의 경계.
//
// 코어 → 셸: 선택 snapshot(`InspectorSnapshot`)을 값으로 전달합니다. 코어 연결 시 adapter는
// `stellar_runtime_poll_event`로 받은 이벤트를 snapshot으로 옮겨 메인 스레드에서 `snapshotHandler`를
// 호출합니다(ADR-0009, 아키텍처 §8.2). 셸은 문서 의미를 계산하지 않고 받은 값만 표시합니다.
// 셸 → 코어: 편집은 `InspectorCommand`로만 보냅니다. 코어가 검증·Undo 그룹화·적용을 맡습니다.

/// 편집 단계. 코어 `HistoryGroupingPolicy`가 Undo 단위를 정하는 근거입니다(아키텍처 §23.3).
enum InspectorEditPhase: Equatable {
    /// 확정 입력(Return, 포커스 이동, popup 선택 등). 그 자체로 Undo 한 단위입니다.
    case commit
    /// 연속 조정(화살표 키·stepper 반복, 색 패널 드래그). 같은 속성의 연속 입력은 코어가 병합합니다.
    case continuous
}

/// 셸이 코어로 보내는 인스펙터 편집 명령.
enum InspectorCommand: Equatable {
    /// 선택 전체의 속성 값을 설정합니다. 수치는 모델 단위입니다.
    case setValue(InspectorPropertyID, InspectorValue, phase: InspectorEditPhase)
    /// 선택한 각 node의 수치를 상대적으로 바꿉니다(혼합 값의 화살표 키·stepper). 모델 단위입니다.
    case adjustNumber(InspectorPropertyID, delta: Double, phase: InspectorEditPhase)
    /// 명령 버튼 실행(반전, Path로 변환, Detach 등).
    case perform(InspectorPropertyID)
    /// Value Binding UI 열기 요청(placeholder, 아키텍처 §14.2).
    case showBinding(InspectorPropertyID)
    /// modifier 추가(stack 끝).
    case addModifier(InspectorModifierKind)
    /// modifier 삭제.
    case removeModifier(id: String)
    /// modifier 활성·비활성.
    case setModifierEnabled(id: String, isEnabled: Bool)
    /// modifier를 `toIndex` 위치로 이동(이동 후 기준 index).
    case moveModifier(id: String, toIndex: Int)
    /// 실패한 snapshot을 다시 요청.
    case reload
}

/// 인스펙터 데이터 공급자. 모든 호출과 콜백은 메인 스레드에서 일어납니다.
///
/// `@MainActor`로 컴파일러가 이 규칙을 강제합니다. 코어 adapter는 다른 스레드에서 받은 결과를
/// 메인 actor로 옮긴 뒤 `snapshotHandler`를 호출해야 합니다(Swift 5 모드에서도 명시한 actor 격리는 검사됨).
/// 구현은 무거운 작업(문서 질의, 값 해석)을 메인 스레드에서 하지 않고, 준비된 snapshot만
/// 넘깁니다(아키텍처 §6.3 UI 스레드 2ms 상한, §9.2).
@MainActor
protocol InspectorDataSource: AnyObject {
    /// 가장 최근 snapshot.
    var snapshot: InspectorSnapshot { get }
    /// 새 snapshot이 준비되면 호출할 콜백. 인스펙터가 설정합니다.
    var snapshotHandler: (@MainActor (InspectorSnapshot) -> Void)? { get set }
    /// 편집 명령을 보냅니다. `generation`은 명령의 근거가 된 snapshot 세대입니다.
    /// 코어는 오래된 세대에 근거한 명령을 거부하거나 재해석할 수 있습니다.
    func submit(_ command: InspectorCommand, basedOn generation: UInt64)
}

extension InspectorPropertyID {
    /// 공통 레이어 속성: 표시 이름.
    static let layerName = InspectorPropertyID("layer.name")
    /// 공통 레이어 속성: 표시 여부.
    static let layerVisible = InspectorPropertyID("layer.visible")
    /// 공통 레이어 속성: 잠금. 잠금 상태에서도 편집할 수 있어야 합니다.
    static let layerLocked = InspectorPropertyID("layer.locked")
}
