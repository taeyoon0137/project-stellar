# 작업 보고서: XIII.8.3 앱 시작 시 최신 복구 snapshot probe와 복구·폐기 선택

## WBS

- WBS: XIII.8.3
- 작업명: 앱 시작 시 최신 snapshot probe 결과를 shell에 반환하고 사용자가 복구 또는 폐기를 선택
- 상태: 완료 (`[x]`) 후보 — 엔진 코어 API와 두 선택 fixture를 마쳤습니다. 셸 prompt UI는 사용자 방향(2026-11-23)에 따라
  UI 재설계 때 이 API에 연결합니다. 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: V.9.3, XII.2.2
- 결정: [ADR-0120](../adr/ADR-0120-recovery-probe-git-scenarios-terminal-env.md) 결정 1·2, [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `recovery-git-terminal`

## 변경 파일

- `crates/runtime/src/app/recovery.rs`: `RecoveryProbe`, `AppRuntime::recovery_probe`, runtime 저장소 root를 쓰는
  `recover_checkpoint`(Restore·Discard), 최신 기록 순서와 공용 판정 함수 사용, Discard가 폐기 표식 경로 사용
- `crates/runtime/src/app/recovery/writer.rs`: 정상 저장·닫기·복원 뒤 정리의 삭제가 `discard_snapshot` 사용
- `crates/runtime/src/app/recovery/lifecycle_tests.rs`: 두 선택·Later·삭제 실패 fixture
- `crates/project/src/recovery.rs`: `list_sessions_newest_first`, `snapshot_modified`, `discarded_marker_path`
- `crates/project/src/recovery_probe.rs`: `judge_stored_snapshot`, `StoredSnapshot`, `StaleReason::Discarded`,
  `RecoveryProposal.hash`, 최신 기록 순서 probe
- `crates/project/src/recovery_shutdown.rs`: `discard_snapshot`, `is_discarded`, `end_window_session_clean`이 폐기 경로 사용

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project --lib recovery` | 0 — 23 passed |
| `cargo test -p stellar-runtime --lib app::recovery` | 0 — 8 passed |
| `cargo test -p stellar-integration-tests --test recovery_round_trip --test e2e_autosave_crash_recovery` | 0 — 3 + 1 passed |
| `cargo check -p stellar-ffi` | 0 — FFI 시그니처 변경 없음 |
| `cargo clippy -p stellar-project -p stellar-runtime --all-targets -- -D warnings` | 0 |
| `cargo xtask validate-adr --base 0c226a06` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬 실행. 셸 UI 없이 `AppRuntime`으로 앱 시작 → 프로젝트 열기 → 창 열기 → probe →
  선택을 실행합니다. lane 전체 `check-core` 결과는 lane 결과 보고에 적습니다.

## 테스트

| 테스트 | 결과 |
|---|---|
| `app::recovery::lifecycle_tests::startup_probe_offers_the_newest_checkpoint_and_both_choices_apply` | 통과 — 창 UUID 순서와 반대로 기록한 두 복구본 중 마지막 기록을 제안(revision 1, 탭 1, 기록 시각 일치). 폐기: 그 복구본만 지우고 표식 없음, 창 문서 불변, 다음 probe가 남은 복구본 제안, 폐기한 후보로 Restore하면 `RequestFailed`. 복구: 창 문서 = 복구본, dirty. 두 선택 뒤 프로젝트 폴더 bytes 불변 |
| `app::recovery::lifecycle_tests::later_leaves_the_checkpoint_for_the_next_start` | 통과 — 아무것도 부르지 않으면 두 번의 시작 모두 같은 bytes hash 후보, 파일 그대로 |
| `recovery::tests::sessions_are_listed_by_last_write_not_by_session_name` | 통과 — 수정 시각 내림차순, 표식 파일은 목록 밖 |
| `recovery_probe::tests::newest_snapshot_comes_first_and_discarded_leftovers_are_stale` | 통과 — 최신 기록 순서, hash 포함, 표식과 같은 bytes는 `Discarded`, 해석 불가는 `UnreadablePayload` |

## 성능

- 해당 없음(성능 예산 대상 경로를 바꾸지 않음). probe는 셸 worker에서 복구본마다 metadata를 한 번 더 읽습니다(ADR-0120 Performance).

## 차이

- 계획 문구의 "shell에 반환"은 셸 UI 없이 `AppRuntime::recovery_probe`의 반환값으로 대신했습니다. 기존 셸 FFI
  `stellar_window_recovery_probe`는 시그니처를 바꾸지 않았고 같은 후보(이제 최신 기록 순서)를 돌려줍니다.
- 같은 변경이 XIII.8.7의 "삭제 실패는 다음 시작에서 stale 판정" 결함(기준 세대가 current와 같은 경우)을 고칩니다. 근거와
  시험은 XIII.8.7 보고서에 적었습니다.

## 남은 위험

- 셸 prompt(복구·폐기·나중에) UI 연결은 UI 재설계 몫입니다.
- 지우지 못한 복구본 파일 자체의 GC는 없습니다(제안만 하지 않음).
