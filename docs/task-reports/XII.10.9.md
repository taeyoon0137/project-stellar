# 작업 보고서: XII.10.9 크기 조절 제스처

- WBS: XII.10.9
- 작업명: 크기 조절 제스처
- 상태: 부분 완료 (`[~]`) — 코어 계산·확정 완료, 저장 위치(IX.11.9)·셸 연결 남음
- 결정: ADR-0105 결정 1·2
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/runtime/src/canvas/transform.rs`(신규), `crates/runtime/src/canvas/transform/{resize,fixtures}.rs`(신규)
- `crates/commands/src/arrange/place.rs`(배치 쓰기)
- 소유 밖 최소 수정: `crates/runtime/src/app.rs`(`canvas_gesture` crate 공개), `crates/runtime/src/app/canvas_gesture.rs`(함수 4개 `pub(crate)`)

## 구현 요약

- `ResizeGesture::begin/update/commands/commit`: update는 `gesture_math::preview`와 같은 상자 계산이고 문서를 바꾸지 않습니다.
- Shift 비율 유지, 명시 `aspect-ratio`가 있으면 기본 비율 유지와 Shift 일시 해제, Option 중심 기준.
- 반대편(Option이면 중심)을 넘으면 그 축을 뒤집습니다. 단일 선택은 local 축 기준이고 고정 변의 world 위치가 남습니다.
- 다중 선택은 공통 부모 공간의 선택 경계를 늘리고 node마다 자기 축 길이 비율로 크기를 바꿉니다(회전 node가 기울지 않음).
- 흐름 자식은 조절한 축의 `width`·`height` 값만 씁니다(위치·layout record 불변).
- 확정은 CommandTransaction 하나(창 history 1건)입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib -- canvas::transform::resize` | 통과(6): 수식키 matrix, 뒤집기와 고정 변 world 위치, 두 번 뒤집기 원복, 다중 선택 비례 배치, 공통 부모 아님 거부, 500 update 뒤 history 1건·확정 안 하면 0건, 흐름 자식 크기만, `aspect-ratio` 기본 비율 유지 |
| `cargo test -p stellar-commands --lib -- arrange` | 통과(place 포함) |
| `cargo clippy -p stellar-commands -p stellar-runtime --all-targets --locked -- -D warnings` | 경고 0 |

## 차이(작업 지시와 다른 점)

- 뒤집기는 Frame `scale` 부호가 아니라 위치 변환 선형부 부호 반전입니다(`scale` longhand 없음).
- 위치·선형부는 지금 저장 위치(Frame `transform`, 그 밖은 위치 Transform modifier)에 씁니다. IX.11.9 step이 제품 목록에 없고 scene이 layout record 위치를 그리지 않기 때문입니다(ADR-0086 결정 3, ADR-0105 결정 1). step이 등록되면 `stellar_commands::arrange::place` 한 곳만 바꿉니다.
- migration 노드(`transform-origin: 0 0`) fixture는 저장 origin이 없어 만들 수 없습니다.
- Text는 저장 고정 폭이 없어 단일 크기 조절을 거부합니다("0에서 멈춤"은 XII.10.33 고정 폭 Text 뒤).
- 셸 UI·FFI·메뉴·`ShortcutPolicy` 부분은 사용자 방향(2026-11-23, ADR-0098)에 따라 하지 않았습니다. UI 재설계 때 합니다.

## 남은 위험 / 한계

- IX.11.9 step 등록 뒤 `place` 쓰기와 완료 기준(`scale` 부호만 반전, inset·`translate` 보정)을 다시 판정해야 합니다.
- 인스턴스는 다중 선택 크기 조절에서 위치만 바뀝니다.
