# 작업 보고서: XIX.7.1 external change batch를 semantic diff로 계산하고 locked subtree overlap 판정

- WBS: XIX.7.1
- 작업명: external change batch를 semantic diff로 계산하고 locked subtree overlap 판정
- 상태: 완료 (`[x]`)
- 선행: XIII.3.4, XIX.4.2
- 결정: [ADR-0100](../adr/ADR-0100-agent-change-history-and-revert.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 셸 UI와 FFI는 범위가 아닙니다(2026-11-23 엔진 우선 방향).

## 변경 파일 목록

- `crates/commands/src/agent_external.rs`(신규) — `overlapping_leases`, `partition_external`, `RecordTarget`. `crates/commands/src/agent_external_tests.rs`(신규).
- `crates/mcp-core/src/agent_history/external.rs`(신규) — `analyze_external`, `respond_to_external`. `crates/mcp-core/src/agent_history/external_tests.rs`(신규).
- `crates/commands/src/lib.rs`, `crates/mcp-core/src/agent_history/mod.rs` — module 등록.

## 구현 요약

- 창이 맞춘 원본과 외부 원본의 semantic diff(ops `diff_records`)를 계산하고, 바뀐 객체를 node·이동 node·View root 대상으로 바꿔 lease 소속(`LeaseTable::holder`)과 subtree 조상 관계(`SubtreeLock::overlaps`)로 겹침을 판정합니다. lease 뒤 추가된 자손도 subtree 안입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

브랜치 `lane/agent-history` 최종 상태에서 실행했습니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands --locked` | lib 205 passed, 5 ignored; 통합 test 3 passed |
| `cargo test -p stellar-history --locked` | lib 55 passed |
| `cargo test -p stellar-mcp-core --locked` | lib 101 passed |
| `cargo clippy -p stellar-commands -p stellar-history -p stellar-mcp-core --all-targets --locked -- -D warnings` | 경고 0 |
| `cargo xtask validate-adr --base 682c7b0c` | 0 |
| `cargo xtask generate-ffi-header --check` | 0, drift 없음 |
| `cargo xtask validate-crate-graph` | 0 |

| 검사 | 결과 |
|---|---|
| non-overlap(lease 밖 이름 변경) 겹침 0, overlap(lease root 자손) 겹침(`external_batch_semantic_diff_and_lease_overlap`) | 통과 |
| lease root 진조상 속성 변경은 겹침 아님, 위치 변경·View root는 겹침(`overlap_is_lease_subtree_or_moved_ancestor`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 XIII.3.4가 아직 없어 판정과 분할까지 했습니다. 문서 범위 객체만 대상이고 token·component 정의·asset 외부 변경은 프로젝트 library 경로에서 따로 봐야 합니다.

## 남은 위험 / 한계

- 외부 반영 처리 비용(semantic snapshot 세 번)은 재지 않았습니다.

## 근거 커밋

- 이 보고서를 담은 `Task(XIX.7.1)` 커밋(브랜치 `lane/agent-history`).
