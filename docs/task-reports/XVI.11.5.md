# 작업 보고서: XVI.11.5 자동 저장 중 비정상 종료→다시 실행→복구 선택→Git 작업 트리 확인 E2E

## WBS

- WBS: XVI.11.5
- 작업명: 자동 저장 중 비정상 종료→다시 실행→복구 선택→Git 작업 트리 확인 E2E
- 상태: 부분 완료 (`[~]`) — 2026-11-23 통합 판정: 이 행의 구현은 끝났지만 선행 XIII.8.7이 `[x]`가 아닙니다. 그 조건이 풀리면 `[x]`. 구현 보고 당시 기록: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: XIII.8.7
- 결정: [ADR-0111](../adr/ADR-0111-headless-release-e2e-scenarios.md)
- lane: `headless-e2e`

## 변경 파일

- `tests/integration/tests/e2e_autosave_crash_recovery.rs`: 시나리오(자식 프로세스 비정상 종료)
- `tests/integration/tests/e2e_common/mod.rs`: 부모 HOME 상속(`STELLAR_E2E_INHERITED_HOME`)

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
| `crash_after_checkpoint_relaunch_restore_and_working_tree` | 통과 — 자식 프로세스가 자동 저장·체크포인트를 켠 앱으로 첫 편집을 자동 저장 writer로 쓰고 둘째 편집의 체크포인트를 남긴 뒤 `abort`(SIGABRT 확인) → 디스크는 첫 편집까지·검증 통과 → 새 앱에서 복구 후보 = 중단한 창 → Restore: 미저장 변경 복구, undo·redo 0, 디스크 bytes 불변, 레이어에 두 Frame → HEAD·index 불변 → Save Now가 일반 저장(작업 트리 변경), 복구 후보 정리 |
| `crash_while_autosave_is_writing_loses_nothing` | 통과(4회 반복) — 체크포인트 뒤 자동 저장 flush 직후 중단 → 다시 열기·검증 통과, 쓰기가 끝났으면 복구 제안 없음, 아니면 복구로 둘째 편집을 되살림 |
| `crash_child_process` | 자식 전용(환경 변수 없으면 바로 끝남) |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음).

## 차이

- 비정상 종료는 테스트 binary를 자식으로 다시 실행해 `std::process::abort()`로 냅니다(ADR-0111 결정 6). 살아 있는 창의 체크포인트는 그 창이 잠가 후보 조회가 건너뛰므로 자식은 저장소 파일을 직접 확인합니다.

## 남은 위험

- 쓰기 중 중단 시험은 어느 시점에 끊겼는지가 실행마다 다릅니다. 두 결과 모두 판정하지만 특정 쓰기 단계(journal marker 앞·뒤)를 골라 끊지는 않습니다.
- 선행 XIII.8.7은 원장에 `[~]`입니다.
