# 작업 보고서: IX.1.1 ComponentDefinition resource

- WBS: IX.1.1
- 작업명: `ComponentDefinition` resource — StableId, root FrameRef, 공개 Property 목록, Slot 목록, local token scope 정의
- 상태: 완료 (`[x]`)
- 선행: Phase VIII, Phase VI (phase 단위 선행 — 완화 적용, 아래 "차이" 참조)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/mod.rs`(신규) — component 모듈 진입점.
- `crates/document/src/component/codec.rs`(신규) — component 모듈 공용 canonical LE reader/writer.
- `crates/document/src/component/definition.rs`(신규) — `ComponentPropertyId`, `ComponentSlotId`,
  `LocalTokenScope`, `DefinitionNode`, `DefinitionTree`, `ComponentDefinition`, `ComponentEntry`,
  `ComponentLibrary`, `ComponentDefinitionError`.
- `crates/document/src/lib.rs` — `pub mod component;` 한 줄 추가.
- `docs/task-reports/IX.1.1.md`(신규), 계획서(IX.1.1 `[ ]`→`[x]`).

## 구현 요약

- **정의 resource**: `ComponentDefinition`은 StableId, 표시 이름(식별 금지), root(`NodeRef`,
  Frame 검증), 공개 Property 목록(`ComponentPropertyId`, 선언 순서·중복 금지), Slot 목록
  (`ComponentSlotId`), local token scope(`TokenRef` 오름차순 집합)를 갖습니다. Property·Slot의
  상세 record는 IX.1.2·IX.3.1의 몫이며 여기서는 interface id 목록만 둡니다.
- **정의 tree**: 컴포넌트가 소유한 node는 `DefinitionTree`(StableId 부모, OrderKey, 표시·잠금,
  종류별 payload bytes)에 둡니다. 부모 선삽입 규칙으로 cycle이 불가능하고 parent 없는 node는
  하나뿐이며 형제 OrderKey 중복을 거부합니다. 자식은 OrderKey(동일 시 StableId) 순입니다.
- **root 검증**: `ComponentDefinition::validate(tree)`가 tree 소유 컴포넌트 일치, root의 tree
  소속(밖이면 `RootOutsideComponent`), Frame 종류, 최상위 여부를 검사합니다.
- **library**: `ComponentLibrary::insert`가 정의 검증을 재사용하고, 한 node가 두 컴포넌트에 속하지
  않게(`NodeOwnedByOtherComponent`), node id와 컴포넌트 id가 겹치지 않게 합니다. 실패 시 상태
  불변입니다.
- **직렬화**: 정의·tree·library 모두 canonical LE(V.1 R1~R3) encode/decode를 제공합니다. tree는
  전위 순서로 기록하고, 비canonical 순서·중복·trailing bytes를 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 147 passed, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | 종료 0 |

| 검사 | 결과 |
|---|---|
| 다른 컴포넌트 node·어떤 tree에도 없는(scene) node를 root로 지정하면 `RootOutsideComponent`, library 등록도 거부·상태 불변(`root_outside_component_is_rejected`) | 통과 |
| Frame이 아닌 root, 최상위가 아닌 root, 다른 컴포넌트 tree 연결 거부(`root_must_be_top_level_frame_of_own_tree`) | 통과 |
| 한 node의 이중 소속·중복 컴포넌트 id 거부, 제거 시 소유 해제(`node_belongs_to_exactly_one_component`) | 통과 |
| tree 삽입 규칙(두 번째 root, 없는 부모, 중복 id, 형제 OrderKey 중복, 자식 순서)(`tree_insert_rules`) | 통과 |
| Property·Slot 목록 중복 거부·선언 순서 유지, token scope 정렬(`interface_lists_reject_duplicates`) | 통과 |
| 정의·tree·library round-trip과 재인코딩 byte-identical(`definition_and_library_round_trip`) | 통과 |
| 절단·trailing·중복 id·비전위 순서 bytes 거부(`malformed_bytes_are_rejected`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 `Phase VIII`, `Phase VI`는 phase 단위 선행이라 사용자 지시(2026-11-14)에 따라 완화했습니다.
  Phase VI 코드(NodeKind 등)는 3002e6d에 있고, Phase VIII(token 정의)는 필요하지 않도록 local
  token scope를 `TokenRef` 참조 집합으로만 정의했습니다.
- root FrameRef는 core-types에 별도 타입이 없어 `NodeRef` + Frame 종류 검증으로 표현했습니다.

## 남은 위험 / 한계

- local token scope의 token 존재·타입 검증은 token 계층(Phase VIII)이 담당합니다.
- 정의 tree와 scene node id의 전역 유일성(정의 node가 scene에도 있는지)은 문서 validator
  (VI.10.1)가 library와 scene을 함께 검사해야 합니다.
- library chunk 저장 경로 연결(VI.10.2)은 후속입니다.
