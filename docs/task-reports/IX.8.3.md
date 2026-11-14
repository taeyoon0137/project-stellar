# 작업 보고서: IX.8.3 기본 accessibility tree generator

- WBS: IX.8.3
- 작업명: decorative 제외·Fragment flatten·hidden 제외 규칙을 적용한 기본 접근성 트리 생성기
- 상태: 완료 (`[x]`)
- 선행: IX.8.2 (이 lane에서 완료), VI.4.1 (base에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/accessibility/src/source.rs`(신규) — 생성기 입력 `SourceTree`·`SourceNode`·`SourceKind`·`SourceError`.
- `crates/accessibility/src/generate.rs`(신규) — `generate`, `GenerateError`.
- `crates/accessibility/src/test_support.rs` — 테스트 전용 golden outline 도우미(`#[cfg(test)]`).
- `crates/accessibility/src/lib.rs` — `source`·`generate` 모듈 선언.
- `docs/task-reports/IX.8.3.md`(신규), 계획서(IX.8.3 `[ ]`→`[x]`).

## 구현 요약

- **입력**: crate 그래프상 document에 의존할 수 없으므로, 상위 adapter가 문서 node tree에서 만드는 중립
  `SourceTree`를 읽습니다(종류, 시각 자식 순서, hidden, decorative, 작성자 label, 읽기 순서). 문서 `NodeKind`와의
  대응표는 `source.rs` 모듈 문서에 있습니다.
- **규칙**: hidden은 자신과 subtree 제외, Fragment는 flatten(자식을 접근성 부모로 올림), decorative는 자신만
  제외하고 자식은 올림, Frame·ComponentInstance는 label 없으면 `Generic`·있으면 `Group`, Text는 `StaticText`
  (label = 작성자 label 또는 `NodeText(자신)`), Image는 `Image`, Graphic(Shape·Path)은 label이 있을 때만 `Image`
  (없으면 장식으로 제외). 작성자 읽기 순서는 그대로 옮깁니다.
- **순회**: 반복 preorder(명시 stack)로 시각 자식 순서를 지켜 flatten된 node의 자식이 원래 위치에 들어갑니다.
  없는 root·자식과 두 번 방문(공유 자식·cycle)은 `GenerateError`로 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-accessibility` | 17 passed, 0 failed |
| `cargo fmt -p stellar-accessibility -- --check` | 통과 |
| `cargo clippy -p stellar-accessibility --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| expected tree fixture(Fragment·decorative flatten, hidden subtree·무라벨 Graphic 제외, 라벨 Frame → Group, 읽기 순서 보존) 구조·golden outline 일치, 생성 결과 `validate` 통과, 결정론 bytes(`expected_tree_fixture_passes`) | 통과 |
| root 수준 Fragment·decorative flatten과 hidden root 제외(`hidden_root_and_root_level_flatten`) | 통과 |
| 없는 자식·없는 root·공유 자식·cycle source 거부(`malformed_source_trees_are_rejected`) | 통과 |
| 깊이 20,000 chain을 stack overflow 없이 처리(`deep_source_tree_does_not_overflow_the_stack`) | 통과 |

## 차이(작업 지시와 다른 점)

- 작업 지시에 없는 기본 규칙 두 가지를 정했습니다: decorative는 자신만 제외하고 자식은 올리며(ARIA
  `presentation`과 같은 의미), label 없는 Graphic(Shape·Path)은 장식으로 봅니다(배경 도형이 이름 누락 진단을
  쏟아내지 않도록). 이미지(Image)는 label이 없어도 node를 만들어 IX.8.5 진단 대상으로 남깁니다.

## 남은 위험 / 한계

- 문서 node tree → `SourceTree` adapter는 아직 없습니다. 현재 crate 그래프에는 document와 accessibility를 함께
  의존할 수 있는 crate edge가 없어(`stellar-accessibility → core-types`만 허용), adapter는 crate 그래프 변경과
  함께 후속 단계(예: XII.8.3 preview bridge)에서 추가해야 합니다.
