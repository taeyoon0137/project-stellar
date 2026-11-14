# 작업 보고서: IX.10.9 profile upgrade plan

- WBS: IX.10.9
- 작업명: old/new capability registry의 semantic diff와 영향받는 nodes/properties를 반환하는 upgrade plan, 적용 전 project 무변경
- 상태: 부분 완료 (`[~]`) — 구현과 검증 열(upgrade preview fixture revision unchanged)은 충족했지만 엄격 선행
  IX.10.4가 `[~]`입니다(아래 "상태 정정").
- 선행: IX.10.4(작성 당시 이 lane `[x]`, 리뷰 정정 후 `[~]`), IX.10.8(이 lane `[x]`), XIV.3.2(`[x]`)

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

## 후속 수정(리뷰 반영): 영향 사용 조회 비용

리뷰(TP-5)에서 사용마다 `registry_changes`를 선형 탐색(`find`)하는 비용이 지적되어 `Fix` commit으로 subject → 첫 변경 map
조회로 바꿨습니다(결과 같음, 첫 일치 규칙 유지). 동작 보존 재구성이라 비용을 직접 재는 실패 테스트는 만들지 않았고, 기존
upgrade preview fixture가 그대로 통과합니다. 사용 수집(`collect_usages`)의 제외 판정 비용 수정은 IX.10.8 보고서의 후속 수정
절에 있습니다.

## 상태 확인(리뷰 반영, 2026-11-14)

- 리뷰(TP-2)는 이 행의 입력도 지역 모델이라는 점을 지적했습니다. 이 행의 작업명과 검증 열은 upgrade plan API와 미리보기 fixture(revision 불변)이고, 실제 문서·프로젝트
  저장이나 command를 요구하지 않습니다. 검증 열은 fixture로 충족되므로 `[x]`를 유지합니다.
- 사실: 프로젝트 입력은 읽기 전용 trait `ProjectSnapshot`이고, 실제 문서 store·매니페스트에 대한 구현이 아직 없습니다. "적용 전 project를 수정하지 않음"은 `&self` 읽기 전용 trait과 테스트 snapshot으로 보장·검증했습니다. 이 연결은 IX.10.3(`stellar.yaml` 연결)과 IX.10.10(override 저장 형식·command·실제 host)의 남은 범위로 두었고,
  두 행은 `[~]`입니다.
- 선행 IX.10.4도 이번 정정으로 `[~]`입니다. 이 행이 쓰는 pin 기준 release 해석(`RegistryCatalog`)은 검증되었고, IX.10.4의 남은
  범위는 실제 `stellar.yaml`에 pin을 저장하는 연결뿐입니다.

## 상태 정정(통합 검증 반영, 2026-11-14)

- `[x]` → `[~]`: 계획 §1.2 규칙 4와 상태 정의(`[x]`는 선행까지 완료)에 따라, 엄격 선행 IX.10.4가 `[~]`인 동안 이 행을
  `[x]`로 둘 수 없습니다. IX.10.4는 이 행의 Task 커밋 뒤 리뷰 정정에서 `[~]`로 내려갔고, 남은 범위는 Windows 증거가 아니라
  실제 `stellar.yaml`에 `registry_version` pin을 저장하는 연결(IX.10.3과 같은 project 연결)입니다. 그래서 Windows 보류
  선례(ADR-0006에 따라 남은 미완료가 Windows 증거뿐인 선행)와 다르게 다뤘습니다.
- `verify-task IX.10.9`는 이 정정 전에도 후에도 `missing-dependency`(선행 IX.10.4가 verified 아님)로 종료 코드 65입니다.
  정정으로 계획서 상태가 선행 규칙과 맞게 되었고, ledger는 `cargo run -q -p xtask -- generate-ledger`로 다시 만들었습니다
  (IX.10.9 `reported_done`).
- 위 "상태 확인(리뷰 반영)" 절의 `[x]` 유지 판단은 검증 열 기준으로는 그대로이지만, 선행 규칙 때문에 이 정정으로
  대체합니다. 이 행의 코드·테스트와 위 검증 결과는 바꾸지 않았습니다.
- **`[x]` 복원 조건**: IX.10.4가 `[x]`가 된 뒤(IX.10.3의 project 연결로 실제 `stellar.yaml`에 pin을 저장하고 bytes·해석
  결과 불변을 확인한 뒤) 이 행의 upgrade preview fixture(`upgrade_preview_leaves_project_unchanged`, `plan_errors`)를 그
  기준에서 다시 통과시키고 `verify-task IX.10.9`가 종료 코드 0이면 `[x]`로 되돌리고 ledger를 다시 만듭니다.
