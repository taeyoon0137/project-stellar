# 작업 보고서: IX.10.9 profile upgrade plan

- WBS: IX.10.9
- 작업명: old/new capability registry의 semantic diff와 영향받는 nodes/properties를 반환하는 upgrade plan, 적용 전 project 무변경
- 상태: 완료 (`[x]`)
- 선행: IX.10.4(이 lane `[x]`), IX.10.8(이 lane `[x]`), XIV.3.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/target-profile/src/upgrade.rs`(신규) — `ProjectSnapshot`(읽기 전용 trait), `CapabilityChangeKind`,
  `CapabilityChange`, `AffectedUsage`, `UpgradePlan`(+ `upgraded_section`), `UpgradePlanError`, `diff_registries`,
  `plan_profile_upgrade`, 테스트 공용 release 2·메모리 프로젝트 fixture.
- `crates/target-profile/src/validate.rs` — 사용 수집을 `collect_usages`·`CapabilityUsage`로 분리(검사 결과 동일).
- `crates/target-profile/src/lib.rs` — 모듈 등록.
- `docs/task-reports/IX.10.9.md`(신규), 계획서(IX.10.9 `[ ]`→`[x]`).

## 구현 요약

- **입력**: 프로젝트는 읽기 전용 trait `ProjectSnapshot`(revision·매니페스트 절·View 목록)으로만 받습니다. plan 계산은 공유
  참조만 쓰므로 프로젝트를 바꿀 수 없습니다.
- **semantic diff**: 이 profile에서 의미가 바뀌는 항목만, subject 오름차순. AddCapability(1)·RemoveCapability(2)·
  ChangeStatus(5, 상태 또는 Deprecated 대체 subject 변경). code는 XIV.3.2 `SemanticChangeKind` AddObject·RemoveObject·
  SetProperty와 같은 번호입니다. 다른 profile에만 영향이 있는 규칙 변경(예: Android plan의 Apple subgrid 변경)은 넣지 않습니다.
- **영향 범위**: 모든 View에서 이 profile이 쓰는 capability(IX.10.8 `collect_usages`, profile override·제외 node 반영) 중
  diff에 있는 subject를 View 순서 → 사용 순서로 돌려줍니다.
- **검사**: 프로젝트에 없는 profile, 현재 pin보다 새 버전이 아닌 목표, 앱에 없는 release(현재·목표)를 거부합니다.
- **적용 준비**: plan은 기준 revision과 기준 매니페스트 절을 담아 IX.10.10 적용 시 stale 검사에 씁니다. `upgraded_section`은
  기준 절에서 대상 profile pin만 올린 절을 계산합니다(프로젝트는 바꾸지 않음).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-target-profile` | 37 passed, 0 failed |
| `cargo clippy -p stellar-target-profile --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-target-profile -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| upgrade preview fixture revision unchanged: revision 42 메모리 프로젝트에서 Android 35·iOS plan을 계산한 뒤 revision·매니페스트 canonical bytes·View 전체 동일; Android diff [backdrop U→A 변경, clip rect 삭제, anchor 추가]와 영향(modal backdrop, clipped clip rect), iOS diff [subgrid 변경, clip rect 삭제, anchor 추가]와 영향(grid subgrid, clipped clip rect); `upgraded_section`은 대상 pin만 2·기본 profile 보존 (`upgrade_preview_leaves_project_unchanged`) | 통과 |
| 미등록 profile·같은 버전·앱에 없는 목표 거부, 같은 release diff 비어 있음, 변경 종류 code [1,2,5] (`plan_errors`) | 통과 |
| `collect_usages` 분리 후 IX.10.8 multi-profile 기대 진단 동일 (`validate::tests`) | 통과 |

## 차이(작업 지시와 다른 점)

- crate graph상 semantic-diff·document crate를 쓸 수 없어 변경 종류를 같은 번호의 지역 enum으로, 프로젝트를 읽기 전용 trait로
  정의했습니다. 실제 프로젝트(문서 store + 매니페스트)에 대한 `ProjectSnapshot` 구현은 상위 crate 작업입니다.
- diff를 profile별 유효 변경으로 한정한 결정은 허용 경로 밖인 `docs/adr/` 대신 모듈 문서와 이 보고서에 기록했습니다.

## 남은 위험 / 한계

- 한 번에 profile 하나의 plan만 계산합니다. 여러 profile 동시 upgrade는 호출자가 plan을 profile마다 만들어야 합니다.
- 테스트의 release 2는 fixture입니다. 내장 catalog에는 아직 release 1만 있습니다.
