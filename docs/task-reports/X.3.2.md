# 작업 보고서: X.3.2 Stack 고정 크기 자식 순방향 배치와 gap

- WBS: X.3.2
- 작업명: 고정 크기 child의 순방향 배치와 gap 계산
- 상태: 완료 (`[x]`)
- 선행: X.3.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/stack.rs` — Stack `measure`(Hug)·`place` 알고리즘, main/cross 정렬, Reverse, fixture 테스트.
- `crates/layout/src/distribute.rs`(신규, 내부) — 균등·가중 분배와 visual order 잔여 subpixel 규칙.
- `crates/layout/src/engine.rs` — `ChildConstraint::forced`(Stretch 등 부모가 정한 크기), Stack 분기 연결, `Padding::start`.
- `crates/layout/src/free.rs` — `forced` 필드 반영.
- `crates/layout/src/error.rs` — X.3.1의 임시 `UnsupportedFeature` 제거.
- `crates/layout/src/lib.rs` — `mod distribute;`.
- `docs/task-reports/X.3.2.md`(신규), 계획서(X.3.2 `[ ]`→`[x]`).

## 구현 요약

- **순방향 배치**: 자식(visual order) 크기 해석 → main 합계 = 크기 합 + `gap × (n − 1)` → 남는 공간 =
  content main − 합계 → main 정렬로 시작 offset·자식 앞뒤 공간 결정 → visual order대로 누적 배치.
- **main 정렬**: Start/Center/End, SpaceBetween(사이 n−1칸), SpaceAround(자식마다 앞뒤 반 칸 = 2n칸),
  SpaceEvenly(n+1칸). 균등 분배의 잔여 subpixel은 visual order 앞 칸부터 1씩. 남는 공간 ≤ 0이면 SpaceBetween은
  Start, SpaceAround·SpaceEvenly는 Center(넘침 허용).
- **Reverse**: 정방향 결과를 content main 축에서 거울상(`main − 위치 − 크기`)으로 뒤집습니다.
- **cross**: Start(·Stretch) 0, Center `(cross − 크기)/2`(ties away from zero), End `cross − 크기`. cross `Fill`은
  cross 크기를 채우고, `Stretch` 정렬은 cross가 HugContent인 자식을 cross 크기로 늘립니다(`forced`).
- **Hug 측정**: main extent = 위 합계, cross extent = 자식 cross 최댓값.
- main 축 `Fill` 늘림과 grow/shrink는 X.3.3 범위라, 이 작업에서는 main 축 `Fill`을 콘텐츠 크기로 해석합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 59 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| horizontal·vertical × main 정렬 6종(Forward) + Reverse 3종 × cross 정렬 3종 = 54 fixture: 고정 자식 3개(삽입 순서 ≠ visual order), gap 8, padding 반영 위치(`horizontal_vertical_and_reverse_fixtures`) | 통과 |
| 잔여 65 subpixel SpaceEvenly 분배(17,16,16,16 순서), 넘침 시 SpaceBetween→Start·SpaceEvenly/Around→Center와 overflow bounds(`residual_subpixels_follow_visual_order_and_overflow_falls_back`) | 통과 |
| Hug 세로 Stack 크기, Stretch로 Hug 자식 늘림, cross Fill 자식(`hug_stack_stretch_and_cross_fill`) | 통과 |
| 분배 helper: 균등 잔여 순서, 가중 분배 합 보존·가중치 0 칸 제외(`distribute::tests`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. (main 축 Fill·grow·shrink는 X.3.3, wrap은 X.3.4, hidden 자식은 X.3.5에서 다룹니다.)

## 남은 위험 / 한계

- 넘침 시 정렬 fallback(SpaceBetween→Start, SpaceAround/Evenly→Center)은 이 엔진의 고정 정책입니다.
