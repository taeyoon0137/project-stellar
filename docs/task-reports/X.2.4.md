# 작업 보고서: X.2.4 local transform 적용 초기 정책

- WBS: X.2.4
- 작업명: local transform은 layout rect 계산 후 적용하고 flow size에 영향을 주지 않는 초기 정책
- 상태: 완료 (`[x]`)
- 선행: X.2.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/transform.rs`(신규) — 정책 문서, `placement_transform`, 테스트.
- `crates/layout/src/input.rs` — `LayoutNode::transform`(기본 identity), `with_transform`, canonical bytes(6 × f32, 비유한 거부).
- `crates/layout/src/property.rs` — `LayoutProperty::Transform`.
- `crates/layout/src/lib.rs` — `pub mod transform;`.
- `docs/task-reports/X.2.4.md`(신규), 계획서(X.2.4 `[ ]`→`[x]`).

## 구현 요약

- **비참여**: 엔진은 transform을 읽지 않습니다. 크기 해석, 형제 flow(Stack 배치), 부모 Hug 크기, overflow bounds는
  변환 전 layout rect로 계산합니다.
- **적용 순서**: layout rect 확정 뒤 `parent = translate(rect.origin) ∘ T`(자식 border box 좌상단 기준 local
  transform 먼저, 그 다음 rect origin 이동). `placement_transform(box, T)`가 이 합성을 반환합니다.
- 변환된 시각 bounds는 scene 단계 책임이며 layout overflow에 넣지 않습니다(문서 명시).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 62 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| Hug 가로 Stack(gap 5)과 Free에서 가운데 자식을 90° 회전·2배 확대해도 결과 bytes 전체 동일, 다음 형제 x = 110, 부모 140×20(`rotated_child_does_not_change_sibling_flow`) | 통과 |
| `placement_transform`: local 원점 → (48, −4), (60, 0) → (48, 116), identity는 rect origin 이동만(`transform_applies_after_layout_rect`) | 통과 |
| transform canonical round-trip, 기본 identity(`transform_round_trips_in_input_bytes`) | 통과 |

## 차이(작업 지시와 다른 점)

- 계획 순서상 X.3.1보다 앞이지만, "sibling flow" 검증을 Stack flow로 하기 위해 X.3.2 다음에 수행했습니다(X.2 섹션
  내부 순서는 유지).

## 남은 위험 / 한계

- transform pivot은 border box 좌상단으로 고정했습니다. 중심 pivot 등 origin 선택은 후속 정책 소관입니다.
- 변환된 시각 bounds 계산(scene)은 이 crate 범위가 아닙니다.
