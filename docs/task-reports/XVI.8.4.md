# 작업 보고서: XVI.8.4 전송 전 payload preview와 수동 진단 번들 내보내기

- WBS: XVI.8.4
- 작업명: 전송 전 payload preview API와 수동 diagnostic bundle export
- 상태: 완료 (`[x]`) — 검증 기준(preview bytes와 실제 upload bytes 동일)을 끝냈습니다. 실제 네트워크 전송자는 범위 밖이라 구현하지 않았습니다(ADR-0109 결정 7).
- 결정: ADR-0109 결정 6, 7, 9
- 선행: XVI.8.2, XVI.8.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/diagnostics/src/bundle.rs`(`PayloadPreview`, `preview_payload`, `PreviewRejected`, `export_bundle` 추가).
- `crates/diagnostics/src/consent_gate.rs`(신규: `TelemetryGate`, `UploadTransport`, 대기열 넣기·보내기).
- `crates/diagnostics/src/store.rs`(`DIAGNOSTICS_DIR` 상수).
- `crates/diagnostics/src/lib.rs`(module 등록 한 줄).
- `tools/stellar-cli/src/diagnostics.rs`(신규: `stellar diagnostics preview|export|scan`).
- `tools/stellar-cli/src/cli.rs`, `tools/stellar-cli/src/lib.rs`, `tools/stellar-cli/tests/fixtures/help.txt`(명령 등록).
- `tools/stellar-cli/Cargo.toml`, `Cargo.lock`(`stellar-diagnostics` 의존 등록, crate graph 허용 목록 안).
- `docs/task-reports/XVI.8.4.md`(신규).

## 구현 요약

- `preview_payload(environment, store, context)`가 번들 bytes를 만들고 scanner(`scan_bundle`)로 검사합니다. 탐지가 있으면 `PreviewRejected`(탐지 목록, 문자열 없음)입니다.
- `PayloadPreview`는 `preview_payload`만 만들 수 있습니다. 보여 줄 bytes(`bytes`, `text`), 수동 내보내기(`export_bundle`), 전송 대기열(`TelemetryGate::enqueue`)이 모두 이 값을 씁니다. 전송자(`UploadTransport::send`)는 그 bytes를 그대로 받습니다.
- `export_bundle`은 새 파일만 만들고(`create_new`, 있으면 거부) 소유자 전용(`0600`)으로 쓴 뒤 fsync합니다. 동의 상태와 무관합니다.
- `TelemetryGate`의 `enqueue`·`flush`는 `Granted`에서만 동작합니다(XVI.8.3). 대기열은 16개 상한이고, 전송이 실패하면 실패한 payload부터 순서대로 남깁니다.
- CLI `stellar diagnostics`:
  - `preview`: store를 읽어 `payload`(보낼 bytes 그대로), record 수, bytes 수, 동의 상태, 자동 전송 허용 여부를 냅니다. 파일을 만들지 않습니다.
  - `export --output <파일>`: 같은 bytes를 새 파일로 씁니다. 이미 있으면 73(`cantcreat`).
  - `scan <파일>`: 파일을 번들로 검사하고 탐지가 있으면 1과 탐지 목록(줄·범주·규칙)을 냅니다.
  - `--data-dir`(기본 `<OS 사용자 데이터>/xyz.taeyoon.stellar/diagnostics`), `--project <경로>`(경로·폴더 이름을 문맥 값으로). 결과 JSON에는 경로를 넣지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics --lib` | 65 passed, 0 failed |
| `cargo test -p stellar-cli --lib diagnostics` | 4 passed, 0 failed |
| `cargo test -p stellar-cli --test headless_help` | 6 passed(help snapshot, 의존 crate 검사 포함) |
| `cargo clippy -p stellar-cli -p stellar-diagnostics --all-targets -- -D warnings` | 경고 0 |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `consent_gate::uploaded_bytes_equal_preview_bytes`: 검증 기준. preview 3개를 넣고 보내면 기록 전송자가 받은 bytes 열이 preview bytes 열과 같습니다.
- `consent_gate::upload_needs_granted_consent`: `NotAsked`·`Declined`에서 넣기 거부, 보내기는 전송자를 부르지 않음(요청 0).
- `consent_gate::queue_is_bounded_and_failures_keep_order`.
- `bundle::preview_is_scanned_and_export_writes_the_same_bytes`: preview bytes = 번들 bytes = 내보낸 파일 bytes, 권한 `0600`, 덮어쓰기 거부, `Debug`에 내용 없음.
- CLI `preview_shows_the_exported_bytes`: CLI `payload` = core 번들 bytes = `export` 파일 bytes, 결과 JSON에 경로 없음, 재내보내기 73, 내보낸 파일 `scan` 통과.
- CLI `empty_store_previews_an_empty_bundle`, `scan_reports_findings_without_echo`(경로·프로젝트 이름·schema 밖 줄 탐지, 결과에 값 없음), `corrupt_store_and_usage_errors`(손상 store 65, 없는 파일 66, 사용법 오류 64 열두 가지).

## 성능

- preview는 store 크기에 선형입니다(상한 8 MiB). 상호작용 경로 밖입니다.

## 차이(작업 지시와 다른 점)

- 실제 네트워크 전송자는 만들지 않았습니다(작업 정의: 네트워크 전송 구현 없음). `UploadTransport` trait과 기록용 전송자로 bytes 동일성을 검증합니다.

## 남은 위험 / 한계

- 앱의 전송 확인 화면은 UI 재설계 때 만듭니다. 그 화면은 `PayloadPreview::text`를 그대로 보여 줘야 합니다.
- `os_version`은 macOS `SystemVersion.plist`만 읽습니다. 다른 운영체제에서는 `0.0.0`입니다(ADR-0006).
