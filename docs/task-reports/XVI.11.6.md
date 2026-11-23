# 작업 보고서: XVI.11.6 같은 프로젝트 두 창 편집→첫 창 저장→두 번째 창 save alert 두 선택 경로 E2E

## WBS

- WBS: XVI.11.6
- 작업명: 같은 프로젝트 두 창 편집→첫 창 저장→두 번째 창 save alert 두 선택 경로 E2E
- 상태: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XII.7.7
- 결정: [ADR-0111](../adr/ADR-0111-headless-release-e2e-scenarios.md)
- lane: `headless-e2e`

## 변경 파일

- `tests/integration/tests/e2e_two_windows_stale_save.rs`: 시나리오

코드 변경은 테스트뿐입니다(제품 코드 변경 없음).

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-integration-tests --locked` | 0 — 전체 통합 테스트 통과(e2e 6개 파일 포함) |
| `cargo clippy -p stellar-integration-tests --tests --locked` | 0 — 경고 0 |
| `cargo xtask validate-adr --base 682c7b0c` | lane 결과 보고에 적음 |
| `STELLAR_CHECK_BASE=682c7b0c cargo xtask check-core` | lane 결과 보고에 적음 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬 실행, debug build, 시스템 Git. 앱 UI 없이 앱 runtime(`AppRuntime`)과 headless ops만 씁니다.
  테스트 HOME·전역 Git 설정은 테스트 프로세스 임시 폴더로 격리합니다(ADR-0111 결정 2).

## 테스트

| 테스트 | 결과 |
|---|---|
| `second_window_keeps_its_snapshot_with_use_current_window` | 통과 — 새 프로젝트 두 창(같은 base) → A Frame·B Text 편집 → A Save Now → B Save Now `SaveStale`, 디스크 불변, 편집 보존 → 외부 변화 반영 뒤 B에 두 선택지 알림 한 번(재조회 0) → UseCurrentWindow: 디스크 = B snapshot(A Frame 덮임), headless cache 같음, history 유지, 다시 저장할 것 없음 → HEAD·index 불변 |
| `second_window_discards_its_edits_with_use_latest_disk` | 통과 — 같은 준비 → UseLatestDisk: 버린 history 1, undo·redo 0, clean, 디스크 bytes 불변(A 저장 그대로), 알림 없음 → 이어 편집·저장이 일반 저장 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음).

## 차이

- 없음.

## 남은 위험

- 외부 writer(CLI·Git)와 dirty 창이 겹친 경우는 두 선택지가 아니라 reconciliation 대기입니다(ADR-0083 결정 2). 이 시나리오는 같은 앱 두 창만 봅니다.
