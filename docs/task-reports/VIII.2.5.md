# 작업 보고서: VIII.2.5 canonical AST serializer

- WBS: VIII.2.5
- 작업명: canonical AST serializer는 map 순서가 아니라 child slot 순서를 사용
- 상태: 완료 (`[x]`)
- 선행: VIII.2.2(`[x]`), V.1.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/expressions/src/codec.rs` — `AST_FORMAT_VERSION`, `ExpressionAst::canonical_order`, `to_canonical_bytes`,
  `from_canonical_bytes`, `CodecError::{UnsupportedVersion, EmptyAst, Ast}`, 모듈 문서(형식 정의), `ast_tests` 테스트 모듈.
- `crates/expressions/src/lib.rs` — 모듈 설명 갱신.
- `docs/task-reports/VIII.2.5.md`(신규), 계획서(VIII.2.5 `[ ]`→`[x]`).

## 구현 요약

- **형식**: `version:u16 LE(=1) ++ node_count:u32 LE ++ node*`. node 형식은 VIII.2.2 그대로입니다. 첫 node가 root입니다.
- **순서**: node를 삽입 순서나 ID(map) 순서가 아니라 root에서 시작하는 **child slot 순서 전위 순회**로 기록합니다
  (`canonical_order`). 공유(DAG) node는 처음 방문할 때 한 번만 기록합니다. node 안의 child도 slot 순서로 정렬되어
  있어(VIII.2.1), GUI의 node 생성 순서·child 연결 순서와 무관하게 같은 AST는 같은 bytes가 됩니다.
- **디코드**: node별 닫힌 연산·payload 디코드(VIII.1.5/2.2) → `ExpressionAst::new` 구조 검증(ID 유일성, slot 구성,
  도달성, cycle 금지) → 재직렬화 bytes가 입력과 같아야 함(비-canonical node 순서 거부). 따라서 한 AST에는 한 bytes
  표현만 존재합니다. 순회·직렬화는 명시적 stack이라 깊은 AST에서도 재귀하지 않습니다.
- canonical 규칙은 V.1.1/V.1.2의 primitive 규칙(LE, 고정 폭, `u32` 길이 프리픽스)을 따릅니다. crate 의존 정책상
  `stellar-expressions`는 `stellar-storage`에 의존할 수 없어(core-types만 허용), 같은 규칙을 이 crate에서 직접 구현했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-expressions` | 종료 코드 0 |
| `cargo test -p stellar-expressions` | 종료 코드 0 (69 unit passed + 3 doctest passed, 0 failed) |
| `cargo clippy -p stellar-expressions --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-expressions -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **GUI 생성 순서가 다른 동등 AST bytes 동일**(`gui_creation_order_does_not_change_bytes`): if·and·less·match·call·공유 node 3개를 포함한 14 node AST를 node 삽입 순서와 각 node child 연결 순서를 seed 고정 셔플로 1,000회(서로 다른 순서 900개 초과) 바꿔도 bytes 동일 | 통과 |
| canonical 순서 = child slot 전위 순서, 공유 node 1회(`canonical_order_follows_child_slots`) | 통과 |
| 형식 golden(`canonical_bytes_golden`): `1 + 2`의 기대 bytes를 encoder 없이 형식 정의로 조립해 일치 | 통과 |
| round-trip·재직렬화 동일(`canonical_bytes_round_trip`), 다른 AST(연산자 변경, Lhs/Rhs 교환)는 다른 bytes(`different_asts_have_different_bytes`) | 통과 |
| 비-canonical node 순서, AST 안의 미지 연산·미등록 함수 ID, cycle, 중복 ID, 미지원 버전, 빈 AST, 잉여·모든 절단, 과장된 node 수 거부(`non_canonical_and_malicious_bytes_are_rejected`) | 통과 |
| 깊이 100,000 AST 직렬화·역직렬화(`deep_ast_serializes_without_recursion`) | 통과 |

## 차이(작업 지시와 다른 점)

- `stellar-storage`의 encoder를 직접 쓰지 않고 같은 primitive 규칙을 구현했습니다(위 crate 의존 정책 사유).
- 저장 chunk에 이 bytes를 담는 연결(파일 포맷 통합)은 이 작업 범위가 아닙니다.

## 남은 위험 / 한계

- 형식 버전 1만 지원합니다. 연산·함수·node 종류를 추가하면 디코더 수용 범위가 바뀌므로 schema 정책(V.8 마이그레이션)과
  함께 다뤄야 합니다.
- node 수 상한은 두지 않았습니다. 디코더는 입력 길이를 넘는 node 수를 할당 없이 거부하지만, 정상 형식의 매우 큰 AST는
  호출자가 크기 제한을 적용해야 합니다.
