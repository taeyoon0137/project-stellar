# 작업 보고서: X.3.1 Stack 설정 타입

- WBS: X.3.1
- 작업명: Stack axis, direction, gap, padding, main/cross alignment 타입 정의
- 상태: 완료 (`[x]`)
- 선행: X.1.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/stack.rs`(신규) — `StackLayout`, `StackDirection`, `MainAlignment`, `CrossAlignment`, canonical tag·bytes, gap 검증, golden 테스트.
- `crates/layout/src/input.rs` — `ContainerLayout::Stack(StackLayout)`(container tag 1), 검증·인코딩 연결.
- `crates/layout/src/engine.rs` — Stack 배치 알고리즘 전까지 `UnsupportedFeature` 명시 오류.
- `crates/layout/src/error.rs` — `LayoutError::UnsupportedFeature`.
- `crates/layout/src/lib.rs` — `pub mod stack;`.
- `docs/task-reports/X.3.1.md`(신규), 계획서(X.3.1 `[ ]`→`[x]`).

## 구현 요약

- **axis**: 공통 `Axis`(Horizontal 0, Vertical 1)를 main axis로 사용합니다.
- **direction**: `Forward` 0 / `Reverse` 1(main 끝부터).
- **gap**: main 방향 자식 간격 `Length`(비음수, 음수는 `NegativeValue`).
- **main alignment**: `Start` 0, `Center` 1, `End` 2, `SpaceBetween` 3, `SpaceAround` 4, `SpaceEvenly` 5.
- **cross alignment**: `Start` 0, `Center` 1, `End` 2, `Stretch` 3.
- **padding**: 모든 배치 방식이 공유하는 node padding(`LayoutNode::padding`, X.1.1)을 Stack content box에
  그대로 사용합니다(별도 필드를 두지 않음).
- canonical bytes: `axis u8, direction u8, gap f32 LE, main u8, cross u8`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 54 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| enum golden: 모든 enum의 tag 값 고정, 역변환, 미지 tag 거부, 기본값(`enum_tags_are_golden`) | 통과 |
| Stack bytes golden `[1, 1, 00 00 08 41, 5, 3]`과 round-trip, node 입력 전체 round-trip(`stack_bytes_are_golden_and_round_trip`) | 통과 |
| 음수 gap 거부(`negative_gap_is_rejected`) | 통과 |

## 차이(작업 지시와 다른 점)

- padding은 Stack 전용 필드가 아니라 공통 node padding을 사용합니다(Free·Stack·Grid 등 모든 container가 같은
  content box 규칙을 공유하도록).
- baseline cross 정렬은 X.3.6(선행 X.10.4 미완료) 소관이라 `CrossAlignment`에 넣지 않았습니다.

## 남은 위험 / 한계

- 배치 알고리즘은 X.3.2부터 구현합니다. 그 전까지 Stack container 계산은 `UnsupportedFeature` 오류입니다.
- wrap·line 정렬 필드는 X.3.4에서 추가합니다.
