# 작업 보고서: XII.7.6 UseLatestDisk: 로컬 편집·history 폐기와 최신 generation 재로드

## WBS

- WBS: XII.7.6
- 작업명: UseLatestDisk: 로컬 편집·history 폐기와 최신 generation 재로드
- 상태: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XII.7.2, VII.5.4
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `window-session-stale`

## 변경 파일

- `crates/runtime/src/window_session.rs`: `reload_latest_disk`·`reopen_tabs`

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --locked` | 0 — lib 239 passed·2 ignored, 통합 테스트 전부 통과 |
| `cargo clippy -p stellar-runtime -p stellar-project -p stellar-ops -p stellar-cli -p stellar-ffi --all-targets --locked -- -D warnings` | 0 — 경고 0 |
| `cargo xtask validate-adr --base d2bf97b3` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬 실행(아래 테스트). lane 전체 `check-core` 결과는 lane 결과 보고에 적습니다.

## 테스트

| 테스트 | 결과 |
|---|---|
| `window_session::stale_tests::use_latest_disk_discards_local_edits_and_history` | 통과 — reload 후 local changes·history·선택 없음, 문서 = 디스크, revision 증가 |
| `window_session::stale_tests::use_latest_disk_keeps_memory_edits_when_the_latest_generation_does_not_load` | 통과 — 로드 실패 시 메모리 편집 유지 |
| `app::autosave::pipeline_tests::use_latest_disk_from_the_runtime_discards_local_edits` | 통과 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- 없음.

## 남은 위험

- 열린 탭은 새 문서에 남은 View만 유지합니다.
