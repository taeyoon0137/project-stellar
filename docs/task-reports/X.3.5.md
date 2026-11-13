# 작업 보고서: X.3.5 hidden·zero-size child 처리 규칙

- WBS: X.3.5
- 작업명: hidden child와 zero-size child 처리 규칙 구현 — hidden은 flow 제외, zero-size는 gap 참여 정책 명시
- 상태: 완료 (`[x]`)
- 선행: X.3.2(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/input.rs` — `LayoutNode::visible`(기본 `true`), `with_visible`, canonical bytes.
- `crates/layout/src/engine.rs` — `Engine::children`이 표시되는 자식만 반환(모든 배치 방식 공통).
- `crates/layout/src/property.rs` — `LayoutProperty::Visibility`.
- `crates/layout/src/stack.rs` — 정책 문서와 fixture 테스트.
- `docs/task-reports/X.3.5.md`(신규), 계획서(X.3.5 `[ ]`→`[x]`).

## 구현 요약

- **hidden**(`visible = false`): 부모 배치에서 완전히 제외합니다 — 크기·gap·grow 분배·줄 바꿈·Hug 측정·overflow에
  참여하지 않고 subtree는 `LayoutResult`에 없습니다. 자식 목록 단계에서 거르므로 Free·Stack 및 이후 배치 방식에
  같은 규칙이 적용됩니다. root의 `visible`은 적용하지 않습니다(root는 항상 배치).
- **zero-size**(main 크기 0) 정책: flow에 참여하는 일반 자식입니다. 양옆 gap을 그대로 받고 main 정렬 분배 칸
  하나를 차지합니다. gap은 표시되는 이웃 자식 쌍마다 하나이며 크기 0은 gap을 접지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 76 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| hidden 가운데 자식: 다음 자식 x 40, box 없음, Hug 너비 60 / hidden Fill은 grow 분배 제외 / wrap 줄 바꿈에서 제외(`hidden_children_are_excluded_from_flow`) | 통과 |
| zero-size 자식: 양옆 gap 유지(40, 50, Hug 70), SpaceBetween 칸 참여(60, 90)(`zero_size_children_keep_gaps`) | 통과 |
| Free Hug에서 hidden 자식 extent·overflow 비기여, hidden subtree 생략, visible round-trip, root visible=false 무시(`hidden_subtree_is_omitted_in_all_layouts_and_round_trips`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- hidden node는 결과에 box가 없으므로 편집 UI가 hidden node 위치를 보여 주려면 별도 정책이 필요합니다.
