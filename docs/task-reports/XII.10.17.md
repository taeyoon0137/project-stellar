# 작업 보고서: XII.10.17 스냅·스마트 가이드 표시 토글과 일시 해제 수식키

- WBS: XII.10.17
- 작업명: 객체·픽셀 그리드 스냅, 스마트 가이드 표시 토글과 스냅 일시 해제 수식키
- 상태: 부분 완료 (`[~]`) — 코어 완료, View 메뉴 `ShellCommand`·`ShortcutPolicy`는 셸 재설계 때
- 결정: ADR-0117 결정 1
- 선행: 계획서 행의 선행 열

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, Apple M3 Max). 다른 작업 여럿이 같은 장비에서 동시에 돌았습니다.

## 변경 파일 목록

- `crates/runtime/src/canvas/view_options.rs`(신규), `crates/runtime/src/canvas.rs`(mod 등록)
- `crates/runtime/src/canvas/snap.rs`(`DEFAULT_TOLERANCE_PT`를 ADR-0104 표 값 `tool_units::SNAP_TOLERANCE_PT`로 다시 내보냄)

## 구현 요약

- `CanvasViewOptions`: 객체 스냅, 픽셀 그리드 스냅, 스마트 가이드, 눈금자, 레이아웃 그리드, 픽셀 그리드 표시 토글. 창 session 상태이며 문서·Git·history 밖입니다. 공유 이름(`view.snap_to_objects` 등)은 `ViewToggle::name`이고 ADR 표와 test가 비교합니다.
- 일시 해제 수식키는 Control입니다. `snapping(modifiers, tolerance)`가 `None`이면 그 update는 가이드·그리드·객체·픽셀 스냅을 모두 하지 않고, `smart_guides_now`가 거짓이라 스마트 가이드도 그리지 않습니다. 결과 타입은 `MoveDrag::update`의 스냅 인자 그대로라 기존 제스처에 바로 넘깁니다.
- 스냅 엔진의 잠정 허용 오차 6pt를 ADR-0104 값 5pt 정의로 바꿔 같은 값이 두 곳에 있지 않게 했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib view_options` | 통과(4): ADR 표 이름·기본값, 토글 후 save bytes·history 불변, Control 일시 해제 중 snap 결과 없음(가이드·그리드·형제 후보), 실제 이동 drag가 원래 이동량 그대로 |
| `cargo test -p stellar-runtime --lib canvas::snap` | 통과(16) |

## 차이(작업 지시와 다른 점)

- View 메뉴 `ShellCommand`와 `ShortcutPolicy.audit()` 검사는 셸 UI가 지워져(ADR-0098) 할 수 없습니다. 공유 이름과 토글 의미만 코어에 두었습니다.
- 토글은 창 session 상태로 두었습니다. 앱 설정 저장(새 창 초기값)은 셸이 `CanvasViewOptions::with`로 넘깁니다. 창 session(`ProjectWindowSession`)에 이 값을 붙이는 일은 ADR-0104 결정 3.1과 같이 셸 연결 때 합니다.

## 남은 위험 / 한계

- `ShortcutPolicy` 문제 0건 검증 기준은 셸 재설계 뒤에 다시 확인해야 합니다.
