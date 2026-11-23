# 작업 보고서: XXIX.2.8 AgentSession 상태 기계

- WBS: XXIX.2.8
- 작업명: XIX.6.4 종료 절차와 아키텍처 31.5 수명 규칙을 표현하는 AgentSession 상태 기계
- 상태: `[x]` 완료
- 선행: XXIX.2.7, XIX.6.4
- 설계 기준: ADR-0005 §4·§6·§8, ADR-0023 결정 9, ADR-0092

## 필수 실행 환경

- `DEV-MAC` (macOS 프로세스·파이프·process group·Code Signing Services, 로컬 검증). 네트워크와 실제 공급자 계정은 쓰지 않습니다.

## 변경 파일 목록

- `crates/agent-host/src/session.rs`(신규): `AgentSession`, `SessionState`, `SessionEvent`, `SessionEffect`, `EndReason`, `SessionMode`
- `crates/agent-host/tests/session_machine.rs`

## 구현 요약

- **상태**:
  - `Starting`: 사용자 시작, bridge 결합 전
  - `Running`
  - `Reconnecting { deadline_ms }`: lease·권한 유지, 새 lease·권한·요청은 받지 않음
  - `Ending { reason }`: 적용 중 transaction 대기
  - `Ended { reason }`
- **전이 표**는 `src/session.rs` 모듈 문서에 있고, test `transition_matrix_matches_report_definition`이 그 표의 26개 조합을 검사합니다.
- **소유**: 세션이 lease, 권한(모드 상한 안만), 응답 대기 요청, 적용 전 제안, 적용 batch 기록을 소유합니다. 결합된 bridge가 끊기면 유예(기본 15 s) 동안 같은 세션으로 다시 묶입니다.
  - 15 s의 근거: bridge 재시작과 재결합(handshake·peer 인증)은 수 초 안에 끝나고, lease 점유 시간은 짧게 둡니다.
- **사용자 주도**: `start_by_user`로만 만듭니다. 끝난 세션은 모든 event를 `Ended`로 거부하고, 재개·재시작·재전송 API가 없습니다(inventory).
- **종료 trigger**: 사용자 중지, CLI 프로세스 종료, 유예 초과, 신뢰 철회, lease 충돌, 사용자 lease 해제, CLI 보고 사용 한도·인증 오류. CLI 오류 문장은 채팅 item으로 그대로 보입니다.
- **XIX.6.4 절차(자동 rollback 없음)**:
  1. 적용 중 transaction은 `Ending`에서 끝나기를 기다립니다.
  2. 확정되지 않은 적용 batch는 `FinalizeInterruptedHistoryEntry` 하나로 묶습니다.
  3. `ReleaseLeases`, `RevokeSessionPermissions`
  4. `RejectPendingRequests`, `DiscardProposals`, `CloseApprovalUi`
  5. `EndBridgeBinding`, `CleanupExecutionTraces`, 프로세스가 살아 있으면 `TerminateProcess`
  6. `RecordDiagnostic`(id·duration·code만)
- 적용 batch 기록(출처 표시 근거)은 지우지 않습니다. 정상 완료(`EditCompleted`)는 `ConfirmUndoEntry` 하나와 lease 해제입니다.
- 효과의 실행(history entry 확정, 승인 화면)은 history·document에 도달하지 않는 이 crate 밖, 앱 runtime(XXIX.3.1)이 합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-host` | 통과(아래 표의 대상별 수, 실패 0) |
| `cargo clippy -p stellar-agent-host --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-agent-host -- --check` | 통과 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |
| `cargo xtask validate-adr --base edfea9db` | 종료 코드 0 |
| `STELLAR_CHECK_BASE=edfea9db cargo xtask check-core` | lane 최종 실행 결과는 lane 보고(통합 기록)에 적음 |

| 검사 | 결과 |
|---|---|
| 상태 전이 matrix(26 조합)가 이 보고서·모듈 문서 정의와 일치 | 통과 |
| 중지 후: 적용 batch 2개 유지(출처 근거), 중단됨 entry 1(batches 2), lease 0, 권한 0, 대기 제안 0, 대기 요청 0, 진단 `agent-session=42 duration_ms=10 error=user-stopped` | 통과 |
| 재연결 fixture: 유예 안 재결합 → 같은 세션 `Running`, lease 2개·권한 유지 | 통과 |
| 유예 초과 fixture: 중단됨 history entry 확정 효과, `ReconnectGraceExpired`, lease 0 | 통과 |
| 적용 중 transaction: 신뢰 철회 시 `Ending`, 정산 뒤 batch 3개로 종료 절차 | 통과 |
| 정상 완료: Undo entry 1, 이후 종료에 중단됨 entry 없음 | 통과 |
| 모드 상한: 검토 후 적용 모드에서 Apply·Terminal·FileDelete·GitPush 거부 | 통과 |
| 한도 오류 fixture(테스트 대역이 `rate_limit` 보고): 세션 `UsageLimitReported`로 종료, 프로세스 종료 효과, CLI가 받은 사용자 메시지 1줄(재전송 0), 오류 item 표시, 이후 event 거부 | 통과 |

- 성능 예산 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- 상태와 전이는 작업이 정하도록 되어 있었고, ADR-0092 결정 11과 모듈 문서에 기록했습니다.

## 남은 위험 / 한계

- 효과 목록을 실제 lease 표·승인 저장·history에 적용하는 연결은 XXIX.3.1·XIX.6 runtime 범위입니다. 그 전까지는 상태 기계 수준의 검증입니다.
- 재연결 유예 15 s는 실측 근거가 아니라 설계값입니다. runtime 연결 뒤 bridge 재시작 시간을 측정해 조정할 수 있습니다.
