# 작업 보고서: X.1.3 layout property dirty class registry

- WBS: X.1.3
- 작업명: layout property registry에 각 속성의 dirty class를 SelfSize, ParentFlow, DescendantConstraint, PaintOnly로 분류
- 상태: 완료 (`[x]`)
- 선행: VI.3.5(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/property.rs` — `DirtyClass`, `dirty_class`(enum 전체 match), `LayoutPropertyMetadata`, `LAYOUT_PROPERTIES`(14개, tag 0~13), `layout_property`, `node_properties`(`LayoutNode` 무누락 구조 분해), `LayoutProperty::SizeLimits`, 테스트.
- `crates/layout/src/lib.rs` — 모듈 설명.
- `docs/task-reports/X.1.3.md`(신규), 계획서(X.1.3 `[ ]`→`[x]`).

## 구현 요약

- **분류**
  - `SelfSize`: width, height, content, size-limits, aspect-ratio — node 자신의 크기 해석이 바뀜.
  - `ParentFlow`: position, flex, visibility, grid-placement, constraints — 부모 배치 안의 위치·참여가 바뀜.
  - `DescendantConstraint`: children, container-layout, padding — node가 자식에게 주는 제약이 바뀜.
  - `PaintOnly`: transform — 레이아웃 결과 불변(X.2.4 정책), layout queue 대상 아님(`affects_layout() == false`).
- **누락 방지**: `node_properties`는 `LayoutNode`를 `..` 없이 구조 분해해 필드 추가 시 컴파일이 실패하고, `dirty_class`는
  enum 전체를 match해 class 없는 속성을 허용하지 않습니다. registry는 `dirty_class`로 class를 채웁니다.
- **VI.3.5 대응**: layout crate는 core-types만 의존하므로 document registry를 직접 참조하지 않습니다. Frame `size` →
  Width·Height(SelfSize), `transform` → Transform(PaintOnly, Frame registry도 paint만), `layout-mode` → ContainerLayout
  (DescendantConstraint)로 대응을 모듈 문서에 고정했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 114 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 모든 layout property(LayoutNode 필드 14개)에 registry 항목·class 존재, registry ↔ 필드 1:1(`every_layout_property_has_a_class`) | 통과 |
| tag 0~13 오름차순·이름 중복 없음(`tags_and_names_are_unique_and_sorted`) | 통과 |
| class별 속성 목록 고정, PaintOnly만 layout 비영향(`dirty_classes_are_fixed`) | 통과 |
| PaintOnly 근거: transform만 바꾼 입력의 결과 bytes 동일(`paint_only_transform_does_not_change_layout`) | 통과 |

## 차이(작업 지시와 다른 점)

- 계획 순서상 X.1.2 다음이지만 X.1.2는 선행(IX.2.2, IX.7.6) 미완료로 건너뛰었고, 속성 목록이 확정되도록 X.2~X.6 입력
  필드를 정의한 뒤 수행했습니다(X.1 섹션 내부 순서 유지).

## 남은 위험 / 한계

- command invalidation → dirty set 변환(X.7.1)은 선행 VII.1.4 미완료로 이 lane에서 연결하지 않았습니다.
