# 작업 보고서: XXIX.2.13 디코드 실패 판정의 세션 처리 연결

- WBS: XXIX.2.13
- 작업명: 디코드 실패 판정을 세션 처리에 연결하되 권한을 판단하지 않음
- 상태: `[x]` 완료
- 선행: XXIX.2.11, XXIX.2.8
- 설계 기준: ADR-0005 §4·§6·§8, ADR-0023 결정 9, ADR-0092

## 필수 실행 환경

- `DEV-MAC` (macOS 프로세스·파이프·process group·Code Signing Services, 로컬 검증). 네트워크와 실제 공급자 계정은 쓰지 않습니다.

## 변경 파일 목록

- `crates/agent-host/src/output.rs`(디코드 실패 → `DecodeStopped` item·`OutputSignal::DecodeFailed`)
- `crates/agent-host/src/session.rs`(`SessionEvent::Output(DecodeFailed)` 처리)
- `crates/agent-host/tests/decode_failure.rs`

## 구현 요약

- **처리 정의**:
  1. 디코더가 실패 판정 하나를 내면 채팅 목록에 `DecodeStopped { code }` item 하나를 더하고, 디코더를 닫힌 채 둡니다.
  2. 이후 stdout은 원문 버퍼에만 들어갑니다. 원문 보기는 계속됩니다.
  3. CLI 프로세스는 계속 실행됩니다. 세션 상태 기계는 `DecodeFailed` 신호를 받아 표시 상태(`decode_failed`)만 바꾸고 효과는 0입니다. 상태·권한·lease·대기 요청·적용 변경·출처 기록은 그대로입니다.
  4. 세션 종료는 사용자 중지나 다른 종료 조건(CLI 종료 등)이 정하고, 그때 일반 XIX.6.4 절차를 따릅니다(자동 rollback 없음).
- **세션을 멈추지 않는 이유**: 세션을 끝내면 권한과 lease가 줄어듭니다. 그러면 "디코드 실패 판정은 권한을 넓히거나 좁히는 수단이 아님"(ADR-0005 §4.4)과 "디코드 실패는 문서 상태와 권한을 바꾸지 않음"에 어긋납니다. 문서 접근은 앱의 권한·승인·lease 판정만 정합니다.
- 2026-11-14 fail closed 결정은 채팅 보기 디코드를 닫는 것으로 지킵니다. 해석하지 못한 출력을 추측해 보이지 않습니다.

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
| 해석 불가 출력 fixture(테스트 대역이 실패 fixture를 9 byte 조각으로 재생): 실패 전 item, `DecodeStopped` 1, 그 뒤 item 0 | 통과 |
| 처리 결과가 보고서 정의와 일치: 효과 0, 상태 Running 유지, CLI 정상 종료, 실패 뒤 원문 보존 | 통과 |
| 적용 변경 자동 삭제 0(적용 batch 유지), 이후 CLI 종료 시 일반 절차의 중단됨 entry | 통과 |
| 권한·lease 확대 0(축소도 0): lease·권한·대기 요청 동일 | 통과 |

- 성능 예산 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- 실패 뒤 처리는 작업이 정하도록 되어 있었고 ADR-0092 결정 10에 기록했습니다.

## 남은 위험 / 한계

- 디코드 실패를 사용자에게 알리는 화면 문구와 원문 보기 전환 안내는 XXIX.4 범위입니다.
