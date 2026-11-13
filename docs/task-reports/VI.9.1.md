# 작업 보고서: VI.9.1 ModifierId와 분리 저장 modifier stack

- WBS: VI.9.1
- 작업명: `ModifierId`를 StableId newtype으로 정의하고 ordered modifier list를 node payload와 분리 저장
- 상태: 완료 (`[x]`)
- 선행: III.1.1, III.5.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/modifier.rs`(신규) — `ModifierId`, `ModifierEntry`,
  `ModifierStack`(insert/push_back/remove/encode/decode), `ModifierError`.
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.9.1.md`(신규), 계획서(VI.9.1 `[ ]`→`[x]`).

## 구현 요약

- **`ModifierId`**: StableId newtype으로 node id와 타입 수준에서 섞이지 않습니다.
- **분리 저장**: `ModifierStack`은 소유 node StableId만 참조하고 node payload bytes를
  포함하지 않는 별도 인코딩입니다(node id + 항목 수 + (id, OrderKey)*).
- **순서**: III.5 `OrderKey`로 정렬을 유지합니다. 삽입·재정렬이 다른 modifier의
  id·key를 바꾸지 않아 semantic merge에 유리합니다. id·OrderKey 중복과 디코딩 시 정렬
  위반을 거부합니다. modifier 종류(VI.9.2)와 enabled·payload(VI.9.3)는 후속입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 131 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| ID와 순서 round-trip + 맨 앞 삽입 시 기존 key 불변 + 재인코딩 byte-identical(`id_and_order_round_trip`) | 통과 |
| node payload와 분리(빈 stack = node id + count), 중복·malformed 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 항목에 kind·enabled·payload가 아직 없습니다(VI.9.2·VI.9.3에서 추가하며 인코딩
  확장).
