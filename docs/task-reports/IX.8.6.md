# 작업 보고서: IX.8.6 Target adapter용 중립 accessibility snapshot

- WBS: IX.8.6
- 작업명: `AccessibilitySnapshot` — 읽기 순서를 적용한 preorder 평탄화, 플랫폼 중립 canonical bytes. 플랫폼 API 호출은 Phase XII
- 상태: 완료 (`[x]`)
- 선행: IX.8.4 (이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). Windows 비교는 ADR-0006 4번 판정 규칙을 따릅니다(아래 "남은 위험").

## 변경 파일 목록

- `crates/accessibility/src/snapshot.rs`(신규) — `AccessibilitySnapshot`, `SnapshotEntry`, `SnapshotError`, binary 형식.
- `crates/accessibility/src/node.rs` — node 본문 codec을 source·읽기 순서를 뺀 의미 본문(`encode_semantics`·
  `decode_semantics`)으로 나눠 snapshot과 공유(node 형식 bytes는 변경 없음).
- `crates/accessibility/src/lib.rs` — `snapshot` 모듈 선언.
- `crates/accessibility/tests/fixtures/snapshot_v1.golden`(신규) — 양 플랫폼 공통 snapshot bytes golden(hex).
- `docs/task-reports/IX.8.6.md`(신규), 계획서(IX.8.6 `[ ]`→`[x]`).

## 구현 요약

- **읽기 순서 적용**: 형제마다 명시 읽기 순서 node를 값 오름차순(같은 값은 트리 순서)으로 먼저, 나머지를 트리
  순서로 둡니다(tabindex 규칙). entry의 읽기 순서 field는 이미 반영되어 비웁니다.
- **평탄화**: root부터 preorder entry 목록(부모 entry index, 깊이, 의미 node). adapter는 앞에서부터 한 번 읽어
  플랫폼 tree를 만들 수 있고, `children(index)`로 자식 entry를 찾습니다. 구조가 forest가 아닌 tree는
  `SnapshotError::InvalidTree`로 거부합니다.
- **binary 형식**: `"STAX" ++ version:u16 ++ count ++ (source ++ opt_u32(parent) ++ 의미 본문)*`. 정수는 고정 폭
  LE, 문자열은 UTF-8, map은 tag 오름차순이며 `usize`·pointer·hash 순서·부동소수점 연산이 없습니다. 디코더는
  magic·버전·잘림·잉여 bytes·중복 source를 거부하고 부모 index가 현재 preorder 조상인지 검사하며, 재인코딩
  일치를 요구합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-accessibility` | 32 passed, 0 failed |
| `cargo fmt -p stellar-accessibility -- --check` | 통과 |
| `cargo clippy -p stellar-accessibility --all-targets -- -D warnings` | 경고 0 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 읽기 순서 적용 preorder(명시 1·2 먼저, 나머지 트리 순서), 부모 index·깊이·`children`(`snapshot_applies_reading_order_in_preorder`) | 통과 |
| snapshot bytes가 고정 golden(`tests/fixtures/snapshot_v1.golden`: radio group + text input + image, 빈 snapshot)과 일치, tree 구성 순서와 무관하게 같은 bytes(`snapshot_bytes_match_cross_platform_golden`) | 통과 |
| round-trip, 모든 잘린 prefix·잉여 bytes·magic 불일치·preorder 밖 부모·중복 source 거부(`snapshot_round_trips_and_rejects_malformed_bytes`) | 통과 |
| cycle 있는 tree 거부(`invalid_tree_is_rejected`) | 통과 |

## 차이(작업 지시와 다른 점)

- "양 플랫폼 snapshot bytes 동일"은 Windows 지원 보류(ADR-0006)로 macOS에서만 실행했습니다. snapshot bytes는
  정수 연산·UTF-8·고정 순서로만 정해지고 OS API·libm·float 연산에 의존하지 않으며 golden이 고정되어 있어,
  ADR-0006 4번 판정 규칙에 따라 macOS 증거로 `[x]`를 줍니다.

## 남은 위험 / 한계

- Windows를 재개하면 `tests/fixtures/snapshot_v1.golden`을 Windows CI에서도 같은 테스트로 다시 확인해야 합니다.
- 플랫폼 접근성 API 변환(NSAccessibility·UIA·ARIA)은 Phase XII·XXII 작업입니다. 디자인 문서 snapshot을 preview
  inspector가 읽는 bridge(XII.8.3)는 문서 → `SourceTree` adapter(IX.8.3 보고서 참조)가 먼저 필요합니다.
