# 작업 보고서: XXIX.3.6 채팅 item 조립

- WBS: XXIX.3.6
- 작업명: CLI 보고 item과 Stellar 승인·출처 기록을 도착 순서로 조립하고, 문서 효과 표시는 기록에서만 만듦
- 상태: `[~]` 부분 완료(조립과 검증 완료, 출처 색인 XIX.6.7이 없어 앱의 `BatchRecord` 생산이 남음)
- 선행: XXIX.2.11, XXIX.3.2, XIX.9.5(미착수), XIX.6.7(미착수)
- 설계 기준: ADR-0005 §4.4, 아키텍처 31.3\~31.5·31.9, ADR-0101 결정 4

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/agent-host/src/assembly.rs`(신규): `ChatAssembly`, `TranscriptEntry`, `EffectEntry`, `BatchRecord`, `AssemblyError`, `Mismatch`
- `crates/agent-host/tests/chat_assembly.rs`(신규)
- `crates/agent-host/tests/fixtures/assembly/review-session.records`(기록 fixture), `review-session.golden`(기대 목록)
- `crates/agent-host/src/lib.rs`: module 등록

## 구현 요약

- **두 출처**: `TranscriptEntry::Chat(AgentChatItem)`은 CLI 보고 그대로이고, `TranscriptEntry::Effect`는 Stellar 기록에서 만든 문서 효과입니다. 둘은 서로의 근거가 아닙니다.
  - CLI가 tool 호출 성공이나 "적용 완료"를 보고해도 문서 효과 표시는 생기지 않습니다.
- **문서 효과**: `ApprovalRequested`·`ApprovalResolved`(승인 저장소 기록)와 `BatchApplied`(출처 기록 요약 `BatchRecord`, XIX.6.7 출처 색인의 batch 하나)뿐입니다.
- **받는 규칙**(어긋나면 `AssemblyError`로 거부하고 표시하지 않음):
  - 다른 세션의 기록은 받지 않습니다.
  - 요청 확정은 먼저 열린 같은 대상의 요청에만 붙고 한 번만 받습니다.
  - batch 적용은 순번이 늘어날 때만 받습니다.
  - 제안이 있는 batch는 그 제안의 batch 승인이 `approved`여야 하고, 제안 없는 적용은 승인 범위 내 바로 적용 모드에서만 받습니다.
- **검증**: `verify(opened, resolutions, batches)`가 표시된 효과와 기록 목록이 정확히 같은지 다시 확인합니다. 기록에 없는 표시(`Unrecorded`)와 빠진 기록(`Missing`)을 모두 냅니다.
- **상한**: CLI 보고 item은 `MAX_SESSION_ITEMS`(10,000)를 넘으면 오래된 것부터 버리고 수를 셉니다. 문서 효과는 검토 대기 출처 표시의 근거라 버리지 않습니다.
- 세션 메모리 전용이고 직렬화·파일 경로가 없습니다. `Debug`는 개수만 씁니다. `to_golden`은 테스트 기준이며 저장 형식이 아닙니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-host --test chat_assembly` | 4 passed |
| `cargo clippy -p stellar-agent-host --all-targets -- -D warnings` | 경고 0 |

| 검증 기준 | test | 결과 |
|---|---|---|
| 기록 fixture로 만든 item 목록 golden 일치 | `record_fixture_assembles_to_golden` | 통과(23줄: 사용자 문장, CLI 보고 8, 범위·batch·권한 요청과 승인·거부·기한 초과·세션 종료 거부, batch 적용, 반복 실행 동일) |
| 승인·출처 기록과 어긋나는 문서 효과 표시 0 | `record_fixture_assembles_to_golden`(`verify` 0), `effects_that_disagree_with_records_are_refused`, `cli_reports_never_become_document_effects` | 통과 |
| batch 순서·`Debug` 내용 0 | `batch_order_is_enforced_and_debug_has_no_content` | 통과 |

## 차이(작업 지시와 다른 점)

- 출처 색인(XIX.6.7)이 아직 없어 `BatchRecord`를 그 batch 요약 입력 타입으로 정의했습니다. fixture 기록은 손으로 쓴 값입니다.

## 남은 위험 / 한계

- 앱 runtime이 창 메모리 출처 색인에서 `BatchRecord`를 만들고 승인 저장소 결과를 조립에 넘기는 연결이 남았습니다(XIX.6.7, XXIX.3.4).
- 출처 표시 상태 전이(이후 변경됨·실행 취소됨·되돌림, XIX.6.10)는 채팅 조립이 아니라 출처 색인이 소유합니다. 채팅 보기에 그 상태를 보일지는 UI 재설계(XXIX.4.3)에서 정합니다.
- XIX.9.5 tool 결과 범주가 정해지면 tool 결과 표시가 필요한지 다시 봅니다.
