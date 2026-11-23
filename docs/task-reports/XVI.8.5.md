# 작업 보고서: XVI.8.5 동의 철회 시 대기 전송 폐기와 이후 전송 차단

- WBS: XVI.8.5
- 작업명: consent revoke 시 queued upload를 폐기하고 이후 전송 차단, 로컬 diagnostics는 정책에 따라 유지
- 상태: 완료 (`[x]`) — 검증 기준(revoke fixture network request 0)을 끝냈습니다.
- 결정: ADR-0109 결정 7, 9
- 선행: XVI.8.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/diagnostics/src/consent_gate.rs`(`apply`, `open`, `save`, 대기열 파일 형식 추가).
- `crates/diagnostics/src/bundle.rs`(저장된 payload를 다시 검사하는 `PayloadPreview::revalidate`).
- `tools/stellar-cli/src/diagnostics.rs`(`consent [show|grant|decline|revoke]`, `enqueue`).
- `tools/stellar-cli/src/cli.rs`, `tools/stellar-cli/tests/fixtures/help.txt`(명령 설명).
- `docs/task-reports/XVI.8.5.md`(신규).

## 구현 요약

- `TelemetryGate::apply(event, at_ms)`가 동의 전이(XVI.8.3 `transition`)를 적용하고, `Granted`를 떠나면(철회·거절) 대기열을 모두 버린 뒤 버린 수를 돌려줍니다. 이후 `enqueue`는 `NoConsent`, `flush`는 전송자를 부르지 않습니다. 다시 `Grant`해도 버린 payload는 돌아오지 않습니다.
- 로컬 store는 건드리지 않습니다. store는 XVI.8.1 보존 정책(기본 30일)을 따릅니다.
- 영속: 진단 디렉터리의 `outbox`(`stellar.telemetry-outbox v1`, 항목마다 `payload bytes=<n>`와 번들 bytes). `save`는 동의 파일을 먼저 쓰고, 대기열이 비었거나 동의가 없으면 `outbox`를 지웁니다.
- `open`은 동의가 `Granted`가 아니면(없음·손상 포함) 남은 `outbox`를 지우고 버린 수를 알립니다. `Granted`면 항목마다 scanner로 다시 검사해 통과한 것만 되살립니다. 형식이 깨진 파일은 통째로 버립니다(닫힘).
- 대기열은 16개, 합계 bytes는 진단 파일 상한 안으로 묶습니다.
- CLI: `consent`는 상태(`state`, `previous`, `changed`, `consent_file`, `automatic_upload`, `queued`, `discarded`)를 내고, 철회할 동의가 없으면 1(`nothing-to-revoke`)입니다. `enqueue`는 동의가 없으면 77(`no-consent`), 가득 차면 75(`queue-full`)입니다. 어느 명령도 네트워크로 보내지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics --lib consent_gate` | 7 passed, 0 failed |
| `cargo test -p stellar-cli --lib diagnostics` | 5 passed, 0 failed |
| `cargo test -p stellar-cli --test headless_help` | 6 passed |
| `cargo clippy -p stellar-cli -p stellar-diagnostics --all-targets -- -D warnings` | 경고 0 |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `revoke_discards_queue_and_blocks_later_uploads`: 검증 기준(revoke fixture network request 0). 동의 → 3개 대기 → 철회(버린 수 3) → 보내기 차단, 넣기 거부, 다시 동의 뒤 보내기 0, `Granted`에서 `Decline`도 같이 버림. 기록 전송자 요청 수는 끝까지 0입니다.
- `persisted_revoke_removes_the_outbox_and_keeps_local_store`: 저장·재열기 뒤 대기열 bytes 보존, 철회 저장 뒤 `outbox` 없음, 재열기 후 요청 0, store 파일 bytes 불변.
- `stale_outbox_without_consent_is_discarded_on_open`: 철회 저장이 동의 파일만 쓰고 끊긴 상황, 동의 파일 손상 상황 모두 열 때 `outbox`를 지우고 요청 0.
- `tampered_outbox_entries_are_dropped`: 금지 정보가 들어간 항목만 버림, 형식이 깨진 파일은 통째로 버림.
- `bundle::stored_payloads_are_rescanned`.
- CLI `consent_revoke_discards_queue_and_keeps_store`: 동의 전 `enqueue` 77, 없는 동의 철회 1, 동의 뒤 대기 2개(대기 bytes = `preview` payload), 철회 뒤 `discarded` 2·`queued` 0·`outbox` 없음·`enqueue` 77, store 파일 불변.

## 성능

- 해당 없음(사용자 사건마다 한 번, 대기열 파일은 상한 8 MiB).

## 차이(작업 지시와 다른 점)

- 철회뿐 아니라 `Granted`에서의 거절도 철회와 같게 처리합니다(ADR-0109 결정 5·7).

## 남은 위험 / 한계

- 실제 전송자가 없으므로 "보내는 중 철회"(전송 요청이 이미 나간 뒤)는 다루지 않습니다. 전송자를 만들 때 `flush`가 payload 하나씩 동의를 다시 보는지 확인해야 합니다. 지금 `flush`는 시작할 때 한 번 봅니다.
- 앱의 동의 화면·자동 전송 일정은 UI 재설계 때 연결합니다.
