# 작업 보고서: VI.9.5 modifier 참조 cycle detector

- WBS: VI.9.5
- 작업명: modifier 참조 graph cycle detector — direct/self/indirect cycle을 저장 전 거부
- 상태: 완료 (`[x]`)
- 선행: VI.9.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/modifier.rs` — `find_modifier_cycle`, `ModifierCycle`.
- `docs/task-reports/VI.9.5.md`(신규), 계획서(VI.9.5 `[ ]`→`[x]`).

## 구현 요약

- **graph**: 모든 stack의 (소유 node → modifier가 참조하는 node) 간선(VI.9.4
  `referenced_nodes`)으로 구성합니다. 비활성 modifier 참조도 포함합니다 — 다시 켜는
  순간 cycle이 되므로 저장 단계에서 막습니다.
- **검출**: 반복(비재귀) 3색 DFS로 self(`[a, a]`)·direct(`[a, b, a]`)·indirect
  (`[a, b, c, a]`) cycle을 찾아 경로를 반환합니다. node·간선을 StableId 순으로 방문해
  결과가 결정론적입니다. 문서 validator(VI.10.1)가 이 결과로 저장을 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 139 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| self·direct·indirect(Boolean 경유) cycle fixture 전부 검출·경로 정확(`self_direct_and_indirect_cycles_are_detected`) | 통과 |
| 비순환(공유 대상) 통과, 비활성 modifier cycle 거부, 결정론(`acyclic_graphs_and_disabled_modifiers`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- tree 계층(부모·자식)과 modifier 참조가 결합된 cycle(예: 자손을 mask로 쓰는 경우)의
  허용 여부는 렌더 평가 순서 정책(Phase XI)에서 결정합니다. 현재는 modifier 간선만
  검사합니다.
