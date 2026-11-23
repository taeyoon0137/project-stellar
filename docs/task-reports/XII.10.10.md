# 작업 보고서: XII.10.10 회전 제스처

- WBS: XII.10.10
- 작업명: 회전 제스처
- 상태: 부분 완료 (`[~]`) — 코어 계산·확정 완료, 저장 위치(IX.11.9)·셸 연결 남음
- 결정: ADR-0105 결정 1·2(C3)
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/runtime/src/canvas/transform/rotate.rs`(신규)
- `crates/commands/src/arrange/place.rs`(`rotation`, `about`)

## 구현 요약

- pivot은 단일 선택이면 border box 중심, 다중 선택이면 선택 world 경계 중심입니다.
- Shift는 단일 선택이면 결과 world 각도, 다중 선택이면 회전량을 15도 배수로 맞춥니다.
- 결과는 node마다 `pivot ∘ rotate ∘ pivot⁻¹`을 부모 공간으로 바꾼 affine이고 pivot world 위치가 남습니다.
- 회전 행렬은 O13 규칙(직각 배수 정확한 0·±1)입니다.
- C3 결정: 회전 중심 이동 핸들을 두지 않습니다. 근거는 저장된 `transform-origin`이 없어 핸들이 바꿀 값이 없고, 중심 고정 회전이 CSS `rotate`(origin 50% 50%)와 같은 결과라는 점입니다(ADR-0105 결정 2).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib -- canvas::transform::rotate` | 통과(3): reference 행렬 일치, pivot world 위치 불변, 0도로 되돌리면 처음 행렬, Shift 15도, 다중 선택 위치·회전, 형제 불변 |
| `cargo test -p stellar-commands --lib -- place` | 통과: 직각 배수 bit 결과 |

## 차이(작업 지시와 다른 점)

- 결과는 `rotate` longhand가 아니라 위치 변환 행렬입니다.
- 위치·선형부는 지금 저장 위치(Frame `transform`, 그 밖은 위치 Transform modifier)에 씁니다. IX.11.9 step이 제품 목록에 없고 scene이 layout record 위치를 그리지 않기 때문입니다(ADR-0086 결정 3, ADR-0105 결정 1). step이 등록되면 `stellar_commands::arrange::place` 한 곳만 바꿉니다.
- migration 노드 fixture는 만들 수 없습니다(저장 origin 없음).
- 흐름 자식 회전에서 형제 layout 불변은 회전이 그 node의 변환만 바꾼다는 것으로 확인했습니다. scene이 아직 flex layout을 그리지 않아 layout 결과 비교는 못 했습니다.
- 셸 UI·FFI·메뉴·`ShortcutPolicy` 부분은 사용자 방향(2026-11-23, ADR-0098)에 따라 하지 않았습니다. UI 재설계 때 합니다.

## 남은 위험 / 한계

- IX.11.9 뒤 `rotate` longhand 쓰기로 바꾸고 다시 판정해야 합니다.
