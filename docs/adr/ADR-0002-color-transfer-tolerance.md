# ADR-0002: 색상 transfer 함수와 정확도 예산

## Status

Accepted

## Context

III.4.1에서 `Color`를 색공간(sRGB·Display P3) + 비연관 알파로 저장하도록 정의했습니다.
III.4.2는 저장된 인코딩(감마) 채널 값과 선형광(linear-light) 값 사이를 오가는 transfer
함수와 그 정확도 기준을 요구합니다.

두 가지 결정이 필요합니다.

1. sRGB와 Display P3가 사용할 transfer 곡선.
2. transfer 함수의 정확도 예산. 이 예산은 `config/performance-budgets.yaml`에 기록되며,
   해당 파일은 ADR 보호 경로(I.3.5)이므로 변경에 ADR이 필요합니다.

## Decision

- transfer 곡선은 **IEC 61966-2-1 sRGB 곡선**(임계 0.04045/0.0031308, 선형 기울기 12.92,
  오프셋 0.055, 지수 2.4)을 사용합니다.
- **sRGB와 Display P3는 동일한 sRGB transfer 곡선을 공유**합니다. 두 색공간의 차이는
  primaries(색역)와 white point이며, 이는 선형 도메인의 별도 행렬 변환입니다. 본 결정과
  III.4.2의 범위는 transfer(encode/decode)에 한정하고, 색역(primary) 변환은 후속 작업으로
  분리합니다.
- 정확도 예산은 `config/performance-budgets.yaml`의 `color.transfer_abs_tolerance`에
  기록하고 초기값을 `1.0e-5`(정규화 채널 절대 오차)로 둡니다. reference 대비 오차와
  encode↔decode round-trip 오차 모두 이 값 이하여야 합니다.

`transfer_abs_tolerance = 1.0e-5`는 8-bit 양자화 간격(1/255 ≈ 3.9e-3)보다 약 390배 작고,
f32 transfer 평가의 일반 오차(~1e-6)보다 큰 여유를 둔 상한입니다.

## Alternatives

- Display P3에 순수 2.6 감마(DCI-P3)를 적용: Apple Display P3는 sRGB 곡선을 사용하므로
  실제 디스플레이 파이프라인과 불일치 — 미선택.
- transfer를 단순 2.2 감마로 근사: 어두운 영역 오차가 커지고 8-bit 경계에서 round-trip
  실패 — 미선택.
- 예산을 코드 상수로만 보관: 성능·정확도 계약의 단일 출처를 분산시켜 감사·검증이
  어려워짐. `config/performance-budgets.yaml` 단일 출처 원칙 위반 — 미선택.

## Consequences

- 저장된 색상은 결정론적으로 선형광으로 디코드되고 다시 인코드됩니다(양 플랫폼 동일).
- 색역(primary) 변환은 별도 작업으로 남으며, 그 정확도 예산은 추가될 때 본 ADR을
  보강하거나 새 ADR로 기록합니다.
- `config/performance-budgets.yaml`이 생성되어 이후 타이밍 예산(XVI.4.1)이 같은 파일에
  누적됩니다.

## Compatibility

- transfer 함수는 순수 계산이며 저장 포맷·공개 타입 시그니처를 바꾸지 않습니다.
  `ColorSpace`/`Color`의 직렬화는 III.4.1과 동일합니다.

## Performance

- transfer 함수는 채널당 분기 1회 + `powf` 1회의 O(1) 연산입니다.
- 정확도 예산 `1.0e-5`는 `config/performance-budgets.yaml`에 기록되고, core-types 테스트가
  reference·round-trip 오차를 이 값과 비교해 강제합니다.

## Security

- 외부 입력(파일 디코드) float은 III.3.1의 유한성 검증을 거친 뒤에만 transfer에 들어갑니다.
  비유한 값은 색상 디코드 단계에서 거부됩니다.

## Migration

- 신규 결정이므로 마이그레이션 대상 데이터가 없습니다. 예산 값을 조정할 때는 본 ADR을
  갱신하거나 새 ADR로 supersede 하고 `config/performance-budgets.yaml`을 함께 수정합니다.
