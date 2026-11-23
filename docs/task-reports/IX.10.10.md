# 작업 보고서: IX.10.10 승인된 profile upgrade 적용(자동 commit 없음)

- WBS: IX.10.10
- 작업명: 승인된 profile upgrade를 CommandTransaction과 manifest 변경으로 적용하고 자동 commit하지 않음
- 상태: `[~]` 유지 — 구현 판정 통과, 선행 미충족(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — 승인·적용·undo 계약과 원자 transaction 모델은 구현·검증했지만, 실제
  `CommandTransaction`과 project 매니페스트 기록 경로를 거치지 않고 테스트 전용 host로만 검증했습니다(아래 상태 정정).
- 선행: IX.10.9(이 lane `[x]`), Phase VII(단계 선행 — 완화 적용, `stellar-commands`의 `CommandTransaction` 존재)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 시스템 Git, 로컬 검증).

## 변경 파일 목록

- `crates/target-profile/src/apply.rs`(신규) — `UsageResolution`, `approve`·`ApprovedUpgrade`·`ApprovalError`,
  `OverrideEdit`(+ `inverse`), `UpgradeTransaction`(+ `inverse`), `UpgradeHost`(trait, commit 수단 없음), `HostError`,
  `apply_upgrade`, `undo_upgrade`, `AppliedUpgrade`, `ApplyError`, hermetic Git·working tree host 테스트 fixture.
- `crates/target-profile/src/lib.rs` — 모듈 등록.
- `docs/task-reports/IX.10.10.md`(신규), 계획서(IX.10.10 `[ ]`→`[x]`).

## 구현 요약

- **승인**: plan의 영향마다 사용자 결정(`Keep`·`ExcludeNode`·`ClearProfileOverride`)을 받습니다. 범위 밖 index, 중복 결정,
  속성이 아닌 사용의 override 지우기를 거부합니다. 결정이 없는 영향은 `Keep`입니다.
- **원자 적용 단위**: `UpgradeTransaction` = 문서 편집(`OverrideEdit` — profile override 값 before/after, node 제외 여부) +
  매니페스트 절 before/after(대상 profile pin만 변경). host(`UpgradeHost`)는 문서 편집을 하나의 `CommandTransaction`(전부 성공
  또는 전부 롤백)으로 실행하고 같은 단위에서 working tree `stellar.yaml`을 기록하며, 실패하면 아무것도 바꾸지 않는 계약입니다.
- **자동 commit 없음**: `UpgradeHost` trait에는 Git commit 수단이 없습니다. 결과는 working tree 변경으로만 남습니다.
- **stale 검사**: 적용은 plan의 기준 revision·매니페스트 절이 host 현재 상태와 같을 때만, undo는 적용 직후 revision·절일 때만
  합니다. 이미 제외된 node·같은 node의 중복 제외는 편집을 만들지 않습니다. 공통값에서 온 속성의 override 지우기는
  `NotAnOverride`로 거부합니다.
- **undo**: 적용 transaction의 역(편집 역순·각 편집의 역, 매니페스트 이전 절, 기준 = 적용 후 revision)을 같은 host 경로로
  실행합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-target-profile` | 39 passed, 0 failed |
| `cargo clippy -p stellar-target-profile --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-target-profile -- --check` | 통과 |
| `cargo test -p stellar-target-profile apply:: -- --nocapture` | 2 passed, "시스템 git 없음" 건너뜀 메시지 없음(`/usr/bin/git` 사용) |

| 검사 | 결과 |
|---|---|
| apply/undo + Git working tree change fixture: 임시 Git 저장소(hermetic 설정, 초기 commit 1개)의 Android 35 upgrade(v1→v2, clipped node 제외 결정) 적용 → revision 43, 문서 제외 override와 매니페스트 pin [1,1,1,2] 함께 반영, `git status --porcelain` = ` M stellar.yaml`, HEAD·commit 수(1) 불변; 적용 후 검사는 release 2 기준(backdrop 근사, clipped 제외); undo → revision 44, View·매니페스트·`stellar.yaml` bytes 원래와 동일, `git status` 깨끗, HEAD 불변; 같은 undo·옛 plan 재적용은 stale (`apply_and_undo_leave_only_working_tree_changes`) | 통과 |
| release 3(Android 35 status bar Unsupported)에서 override 지우기 적용·undo 값 복원, 승인 오류(범위 밖·중복), 공통값 속성 override 지우기 거부, host 매니페스트 기록 실패 시 문서 무변경, plan 이후 매니페스트 변경 시 `StaleManifest` (`clear_profile_override_round_trips_and_failures_change_nothing`) | 통과 |

## 차이(작업 지시와 다른 점)

- crate graph상 target-profile은 core-types에만 의존하므로 `stellar-commands::CommandTransaction`을 직접 만들 수 없습니다.
  문서 편집은 `OverrideEdit` 목록으로 정의하고, host 계약(`UpgradeHost::apply_transaction`)이 이를 하나의 `CommandTransaction`과
  `stellar.yaml` 기록으로 실행하게 했습니다. 실제 host 구현(문서 command 계층의 profile override command 추가, project의
  매니페스트 기록 연결)은 상위 crate 작업입니다. 테스트는 같은 계약(사본에 전부 적용 후 교체, 실제 파일 기록)을 따르는 테스트
  host와 실제 시스템 Git working tree로 검증했습니다.
- 테스트의 `stellar.yaml` 텍스트는 테스트 전용 결정론 기록기로 만들었습니다(제한 YAML canonical writer는 project crate 소유).
- 단계 선행 Phase VII은 지시에 따라 완화했습니다(필요한 `CommandTransaction`은 이미 존재).

## 남은 위험 / 한계

- profile override를 문서 저장 형식에 기록하는 command(문서 crate)와 history 연결은 아직 없습니다. 연결 전까지 undo는 이 crate의
  `undo_upgrade` 경로로만 검증되었습니다.
- 여러 profile을 한 번에 upgrade하는 transaction은 지원하지 않습니다(profile마다 plan·적용).

## 후속 수정(리뷰 반영): 시스템 Git이 없으면 테스트 실패

리뷰(TP-4)에서 PATH에 `git`이 없으면 Git working tree 단언을 건너뛴 채 테스트가 통과한다는 점이 지적되어 `Fix` commit으로
고쳤습니다. 이제 테스트 fixture(`test_git::FixtureGit::locate`)는 시스템 Git이 없으면 panic(테스트 실패)하고,
`STELLAR_ALLOW_MISSING_GIT=1`일 때만 경고를 남기고 건너뜁니다(dependencies crate와 같은 규칙과 환경 변수 이름).

| 명령 | 결과 |
|---|---|
| 수정 전 `cargo test -p stellar-target-profile apply::` | 1 failed(`missing_git_fails_unless_explicitly_allowed`: git 없는 PATH에서 실패하지 않음) |
| 수정 후 `cargo test -p stellar-target-profile` | 41 passed, 0 failed |
| 수정 후 `cargo test -p stellar-target-profile apply:: -- --nocapture` | 3 passed. 건너뜀 메시지는 정책 테스트의 명시적 허용(`1`) 경우에서만 나오고, 실제 Git fixture 테스트 2개는 시스템 Git으로 실행 |

| 검사 | 결과 |
|---|---|
| 빈 PATH 디렉터리·PATH 없음에서 기본값·`0`·`yes`는 panic, `1`만 `None`(건너뜀) (`missing_git_fails_unless_explicitly_allowed`) | 통과 |

## 상태 정정(리뷰 반영, 2026-11-14)

- `[x]` → `[~]`: 리뷰(TP-2) 지적을 확인했습니다. 작업명의 "CommandTransaction과 manifest 변경으로 적용"은 계약 수준에서만
  충족됩니다.
  - `UpgradeHost`는 trait뿐이고, 이 crate·상위 crate 어디에도 실제 구현이 없습니다. 테스트의 `WorkingTreeHost`는 `Vec<ViewModel>`
    사본에 편집을 적용한 뒤 교체하는 메모리 host입니다. `stellar-commands::CommandTransaction`을 거치지 않습니다.
  - 테스트의 `stellar.yaml`은 테스트 전용 기록기(`emit`)로 씁니다. 형식은 `key:` 다음 줄에 `"value"`를 쓰는 방식이라 project의
    canonical YAML writer(IV.1.5) 출력이 아닙니다. Git working tree fixture(` M stellar.yaml`, HEAD 불변, undo 후 깨끗)는
    실제 시스템 Git으로 실행했지만, 실제 매니페스트 writer와 문서 command가 아닌 테스트 host 위에서의 결과입니다.
  - profile override(값과 node 제외, IX.10.7 `ProfileOverrides`)를 문서에 저장하는 형식과 command가 아직 없습니다.
  - 이 연결을 맡은 다른 WBS 행은 계획서에서 찾지 못했습니다.
- `[x]`로 되돌리려면 다음이 필요합니다(이 행의 남은 범위).
  1. 문서 계층에 profile override 저장 형식과 그 편집 command 추가(document·commands crate).
  2. 상위 crate에서 `UpgradeHost`·`ProjectSnapshot` 실제 구현. 문서 편집은 하나의 `CommandTransaction`으로 실행하고,
     `stellar.yaml`은 project canonical writer로 기록합니다(IX.10.3 project 연결 필요).
  3. 실제 host로 apply/undo와 Git working tree change fixture 재실행.
- IX.10.7·IX.10.8·IX.10.9도 지역 입력 모델(`ViewModel`·`VariantLayer`·`ProjectSnapshot`)을 씁니다. 다만 그 행들의 작업명·검증
  열은 resolver·validator·plan API와 fixture이고 실제 저장·command를 요구하지 않습니다. 그래서 `[x]`를 유지하고, 실제 입력
  연결은 이 행과 IX.10.3의 남은 범위로 둡니다(각 보고서에 같은 사실을 적음).

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 구현 판정 통과, 선행 미충족.
- 근거: 승인·적용·undo 계약과 원자 transaction 모델은 있지만 실제 manifest 변경과 Git working tree fixture는 IX.10.3 연결에 달려 있습니다. 선행 IX.10.9가 `[~]`라 `[~]`를 유지합니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/target-profile/src/` 승인 upgrade 적용 모델. 자동 commit 경로 없음.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-target-profile` | 42 passed |

### 남은 조건과 finding

- IX.10.3 연결 뒤 실제 `stellar.yaml` 변경과 working tree change fixture로 다시 판정합니다.
