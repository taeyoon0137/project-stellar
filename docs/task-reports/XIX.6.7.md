# 작업 보고서: XIX.6.7 에이전트 변경 출처 색인 AgentChangeProvenance를 창 세션 메모리에 정의

- WBS: XIX.6.7
- 작업명: 에이전트 변경 출처 색인 AgentChangeProvenance를 창 세션 메모리에 정의
- 상태: 완료 (`[x]`)
- 선행: XIX.5.5, VII.1.1, VII.1.4, VII.4.1
- 결정: [ADR-0100](../adr/ADR-0100-agent-change-history-and-revert.md)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 셸 UI와 FFI는 범위가 아닙니다(2026-11-23 엔진 우선 방향).

## 변경 파일 목록

- `crates/mcp-core/src/provenance.rs`(신규) — `AgentChangeProvenance`, `BatchProvenance`, `BatchOutcome`, `DisplayState`.
- `crates/mcp-core/src/provenance_tests.rs`(신규), `crates/mcp-core/src/provenance_fixture.rs`(신규, test 전용).
- `crates/mcp-core/src/lib.rs` — module 등록.

## 구현 요약

- 적용 batch마다 AgentSessionId, CLI 세션 표시 이름, batch 순번(세션별 1부터), lease, 그룹 history sequence, `AgentBatchRecord`(change set의 StableId·PropertyPath, 생성·삭제 node, 지문), 결과(진행 중·완료·중단)를 기록합니다.
- batch command 출처는 VII.1.1의 `FutureAgent`이고 VII 타입은 바꾸지 않았습니다.
- 직렬화 경로가 없고 `close_window`로 창과 함께 버립니다. ADR-0093처럼 지금은 mcp-core에 두고 XXIX.3.1 에이전트 편집 구현 crate로 옮깁니다(ADR-0100 결정 3).

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
| batch→StableId·PropertyPath·생성 node 매핑 정확(`batch_maps_to_exact_stable_ids_and_property_paths`) | 통과 |
| 저장 bytes에 세션 표시 이름·세션 id·lease id 0, 창 닫기 후 색인 0(`provenance_is_not_in_saved_bytes_and_is_dropped_with_window`) | 통과 |
| 진단 기록에 출처 필드 없음(`interrupted_diagnostic_has_only_session_duration_and_code`) | 통과 |

## 차이(작업 지시와 다른 점)

- recovery snapshot(V.9.2)·텔레메트리는 이 crate에서 닿지 않아, 출처 타입에 직렬화 경로가 없음과 저장 bytes 검사로 대신했습니다.

## 남은 위험 / 한계

- runtime은 mcp-core에 닿지 못하므로 앱 창이 이 색인을 소유하는 연결은 XXIX.3.1에서 합니다.

## 근거 커밋

- 이 보고서를 담은 `Task(XIX.6.7)` 커밋(브랜치 `lane/agent-history`).
