# 작업 보고서: XIV.2.8 Git UI 전 pending autosave flush 상태 반환

## WBS

- WBS: XIV.2.8
- 작업명: Git UI 전 pending autosave flush 상태 반환
- 상태: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XIII.1.5, XIV.2.7
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `window-session-stale`

## 변경 파일

- `crates/project/src/autosave/flush.rs`(신규): `GitUiPreflight`·`GitUiChoice`
- `crates/runtime/src/app/autosave.rs`: `git_ui_preflight`·`prepare_git_ui`

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
| `app::autosave::pipeline_tests::git_ui_preflight_lets_the_caller_choose_save_or_disk` | 통과 — 확인만으로 쓰기 0, ContinueWithDisk는 쓰기 0·편집 유지, SaveFirst는 저장 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- Git UI 자체는 만들지 않았습니다(사용자 방향).

## 남은 위험

- 없음.
