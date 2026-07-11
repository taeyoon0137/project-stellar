# 작업 보고서: V.9.2 canonical recovery payload

- WBS: V.9.2
- 작업명: 문서 미저장 snapshot·열린 탭·viewport·선택·target profile의 canonical recovery payload
- 상태: 완료 (`[x]`)
- 선행: V.9.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/recovery_payload.rs`(신규) — `RecoveryPayload`, `OpenTab`,
  `Viewport`, `UnsavedChunk`, `RecoveryPayloadError`,
  `RECOVERY_PAYLOAD_VERSION = 1`.
- `crates/project/src/lib.rs` — 모듈 등록.
- `docs/task-reports/V.9.2.md`(신규), 계획서(V.9.2 `[ ]`→`[x]`).

## 구현 요약

- **포함 내용**: base manifest hash(마지막 정상 저장 세대, 무저장이면 None) +
  문서 revision(V.9.3 최신성 비교용) + 미저장 logical chunk들(id·kind·payload) +
  열린 탭(view id + viewport) + 활성 탭 index + 선택 node 목록 + target profile.
  **터미널·Git 실행 상태는 포함하지 않습니다.**
- **인코딩**: canonical 규칙(V.1) — payload 버전 프리픽스, 고정 필드 순서, LE
  고정 폭, 길이 프리픽스, 유한 float(V.1.4: 비유한 viewport 거부·`-0.0` 정규화).
  재인코딩은 byte-identical합니다.
- **검증**: 활성 탭 index 범위, 미지원 버전 거부, 절단·trailing bytes 거부.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project` | 126 passed, 0 failed |
| `cargo fmt -p stellar-project -- --check` | 통과 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| payload round-trip + 재인코딩 byte-identical(`payload_round_trips`) | 통과 |
| 최소 payload, 비유한 viewport 거부, 활성 탭 범위, 버전·절단 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- V.9.3 최신성 비교를 위해 `revision` 필드를 payload에 포함했습니다(계획 V.9.3의
  "recovery revision" 근거 필드).

## 남은 위험 / 한계

- viewport·선택의 구체 의미(좌표계, 다중 선택 정렬)는 문서 도메인(Phase VI)과
  뷰포트 구현에서 확정되며, 필요 시 payload 버전 상승으로 확장합니다.
