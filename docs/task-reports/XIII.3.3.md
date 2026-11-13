# 작업 보고서: XIII.3.3 외부 변경 대기 중 자동 저장 중지

- WBS: XIII.3.3
- 작업명: Semantic Reconciliation 구현 전에는 pending 상태에서 autosave를 중지하고 read-only alert만
  반환, 자동 overwrite 금지
- 상태: 완료 (`[x]`)
- 선행: XIII.3.2(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/external_change.rs` — `AutosaveGate`, `ExternalChangeAlert`, `GuardedSaveError`,
  `WindowExternalState::autosave_gate`·`take_alert`·`run_guarded_save`, 테스트.
- `docs/task-reports/XIII.3.3.md`(신규), 계획서(XIII.3.3 `[ ]`→`[x]`).

## 구현 요약

- **자동 저장 중지**: `ExternalChangePending` 상태에서는 `autosave_gate()`가 `Suspended`입니다.
- **저장 경로 거부**: `run_guarded_save`는 대기 중이면 저장 파이프라인(V.5)을 호출하지 않고
  `GuardedSaveError::ExternalChangePending`으로 거부합니다(어떤 파일도 쓰지 않음). 대기가 아니면 V.5
  `run_save`를 실행하고 성공 시 새 base로 clean 상태가 됩니다. 저장소 자체의 optimistic base check
  (V.5.2)도 오래된 base 계획을 거부하므로 이중으로 막힙니다.
- **읽기 전용 alert**: 대기 상태가 되면 `take_alert()`가 세 snapshot hash와 `read_only: true`만 담은
  alert를 **한 번** 돌려줍니다(해결 선택지 없음 — reconciliation은 XIII.3.4). 같은 대기 상태에서 다시
  꺼내면 없고, 더 새 외부 generation이 오면(새 정보) 다시 냅니다.
- 어느 버전으로도 자동 덮어쓰지 않으므로 대기 중 current pointer는 외부 generation 그대로입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 240 passed, 0 failed; fuzz 통합 4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **pending 중 current pointer 미변경**: 대기 상태에서 자동 저장 `Suspended`, 읽기 전용 alert 1회(반복 없음), 메모리 상태 저장 3회 시도 모두 거부, pointer bytes·프로젝트 파일 트리 bytes 불변, V.5.2도 같은 계획 거부, 새 외부 generation에서만 alert 재발행(`pending_state_suspends_autosave_and_never_moves_current_pointer`) | 통과 |
| 대기가 아닌 창은 저장 가능하고 성공 시 새 base로 clean(`guarded_save_works_outside_pending_state`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 자동 저장 scheduler(XIII.1)는 아직 없으므로 scheduler가 `autosave_gate()`를 확인하고
  `run_guarded_save`를 쓰도록 연결하는 것은 XIII.1.3~XIII.1.5에서 합니다.

## 남은 위험 / 한계

- 대기 상태를 푸는 방법(reconciliation·사용자 선택)은 XIII.3.4·XII.7 범위라 지금은 대기 상태가 창을
  닫을 때까지 유지됩니다.
- 창 닫기 flush(XIII.1.6)는 대기 상태에서 저장하지 않으므로 복구 snapshot(XIII.8)이 메모리 변경을
  보존해야 합니다.
