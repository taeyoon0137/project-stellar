# 작업 보고서: XV.1.2 branch 선택자와 lock exact commit

- WBS: XV.1.2
- 작업명: branch 선택자를 개발 편의 선언으로 허용하되 lockfile에는 항상 exact commit 기록
- 상태: 완료 (`[x]`)
- 선행: XV.1.1(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/selection.rs`(신규) — `SelectorResolver` trait, `SelectorError`,
  `VersionSelector::is_floating`, `LockPin`, `pin_declaration`, 테스트용 `TableResolver`.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.1.2.md`(신규), 계획서(XV.1.2 `[ ]`→`[x]`).

## 구현 요약

- `SelectorResolver::resolve(repository, selector) -> CommitId`가 해석 계약입니다. 시스템 Git 구현은
  XV.2.4에서 추가합니다.
- `LockPin`은 lockfile에 기록할 pin으로 `commit: CommitId`만 해석 결과로 가집니다. `CommitId`는
  40·64자 전체 hex만 만들 수 있으므로 branch·tag 이름이나 축약 hash가 lock 값이 될 수 없습니다(타입
  강제). 선언 선택자(`requested`)는 이후 "선택자가 바뀌었는지" 비교(XV.4.3)용으로만 보존합니다.
- branch 선택자는 `is_floating() == true`인 개발 편의 선언으로 허용합니다(거부하지 않음).
- `SelectorError`는 Git stderr 원문을 담지 않고 작업 이름·종료 코드만 담습니다(URL 노출 방지).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 9 passed, 0 failed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| `branch: main` 해석 → pin commit이 해석된 40자리 hex와 같고 branch 이름이 아님, floating 표시, branch 이동 후 재해석은 새 exact commit(`branch_resolves_to_exact_commit_in_lock_pin`) | 통과 |
| tag·commit 선택자도 exact commit pin, floating 아님(`tag_and_commit_selectors_also_pin_exact_commits`) | 통과 |
| 해석 실패 시 pin 없음, 축약 hash는 `CommitId` 생성 불가(`resolution_failure_produces_no_pin`) | 통과 |

## 차이(작업 지시와 다른 점)

- "branch resolve"는 in-crate `TableResolver`로 검증했습니다. 실제 시스템 Git 해석은 XV.2.4
  (`ls-remote`·fetch·commit 검증) 범위입니다. lockfile 텍스트 기록은 XV.4.x와 project writer가
  담당하며, 이 작업은 lock에 들어갈 값의 타입 계약을 고정합니다.

## 남은 위험 / 한계

- branch 선언이 있는 프로젝트를 release 검증에서 경고할지(개발 편의 한정)는 별도 정책이 필요합니다.
  현재는 `is_floating`으로 식별만 합니다.

## 상태 정정(통합 리뷰, 2026-11-14)

- `[x]` → `[~]`: 엄격 선행 사슬에 `[~]` 행(XV.1.1 등)이 있어 함께 `[~]`로 둡니다. 이 행 자체의 검증은 dependencies crate 안에서 통과했습니다.

## 상태 복원(deps_wiring lane, 2026-11-14)

- `[~]` → `[x]`: 엄격 선행 XV.1.1 `[x]`.
- 실제 파서 경로 검증: 실제 Git fixture에서 b가 `{branch: main}`으로 선언한 c가 lock 텍스트에 40자 exact commit(v2)으로, `requested`에는 branch로 기록하고, 다시 읽은 `Lockfile`에서도 같음(`manifest_source::tests::real_git_resolution_through_real_parsers_pins_branch_to_exact_commit`).
- 공통 근거: `Fix: stellar.yaml 의존성 선언을 실제 매니페스트 파서에 연결`(리뷰 P2), `Fix: stellar.lock.yaml entry 형식을 IV.1.6 모델 하나로 통합`(리뷰 P3, ADR-0021), `Fix: resolver library 매니페스트 읽기를 실제 stellar.yaml 파서로 연결`.
- 실행(`DEV-MAC`): `cargo test -p stellar-project` lib 261 passed·통합 17 passed, `cargo test -p stellar-dependencies` 57 passed·doctest 1 passed, `cargo clippy -p stellar-project -p stellar-dependencies --all-targets -- -D warnings` 경고 0, `cargo fmt -p stellar-project -p stellar-dependencies -- --check` 통과.
