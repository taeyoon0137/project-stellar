# 작업 보고서: V.10.6 텍스트 원본의 감시·외부 변경·저장·재열기 연결

## WBS

- WBS: V.10.6
- 작업명: 텍스트 원본의 감시·외부 변경·저장·재열기 연결
- 상태: 부분 완료 (`[~]`) — 엔진 session 경로는 연결했고 앱 기본 열기·저장 전환(V.10.7)과 CLI 표면은 하지 않았습니다. 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: V.10.5, XIII.3.1
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `window-session-stale`

## 변경 파일

- `crates/project/src/external_change/source.rs`(신규): `TextSourceSession`(열 때 journal recover, journal 저장, 외부 변경 반영), `SourceDiagnostic`
- `crates/project/src/watcher/event.rs`: `affects_text_source`·`TEXT_SOURCE_DIR`
- `crates/project/src/external_change.rs`: 모듈 등록
- `crates/project/tests/text_session.rs`(신규)

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project --locked` | 0 — lib 418 passed·1 ignored, 통합 테스트(text_session 8 포함) 전부 통과 |
| `cargo clippy -p stellar-runtime -p stellar-project -p stellar-ops -p stellar-cli -p stellar-ffi --all-targets --locked -- -D warnings` | 0 — 경고 0 |
| `cargo xtask validate-adr --base d2bf97b3` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬 실행(아래 테스트). lane 전체 `check-core` 결과는 lane 결과 보고에 적습니다.

## 테스트

| 테스트 | 결과 |
|---|---|
| `text_session::save_writes_only_changed_files_through_the_journal_and_reopens_identically` | 통과 — 저장→재열기 의미 동일, 바뀐 파일 하나, 시스템 Git diff에 그 파일·이름만 |
| `text_session::clean_session_applies_an_external_edit_as_one_undoable_command_and_ignores_its_own_save` | 통과 — clean 외부 변경, cache 무효화 node, Undo 한 번 |
| `text_session::dirty_session_keeps_its_edits_and_its_next_save_is_stale` | 통과 — dirty 편집 보존, 다음 저장 stale·쓰기 0 |
| `text_session::broken_external_yaml_is_rejected_with_path_and_line_and_applies_nothing` | 통과 |
| `text_session::open_recovers_a_leftover_save_transaction_first` | 통과 |
| `text_session::watcher_events_drive_the_external_check` | 통과(macOS FSEvents) |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- 검증 열의 CLI dry-run 단계는 CLI 명령이 없어 library 시험으로 대신했습니다. CLI는 소유 범위 밖이고 V.10.7 전환과 함께 붙입니다.
- dirty 창의 병합은 XIII.3.4 범위라 대기(편집 보존)까지만 합니다.

## 남은 위험

- 앱 runtime 기본 경로는 아직 chunk generation입니다(V.10.7).
- 문서 밖 의미(컴포넌트·token·catalog)의 외부 변경은 Undo로 되돌리지 않습니다.
