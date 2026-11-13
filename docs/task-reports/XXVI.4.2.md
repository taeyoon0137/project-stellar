# 작업 보고서: XXVI.4.2 고급 Git 쓰기 기능 feature flag 기본 비활성

- WBS: XXVI.4.2
- 작업명: 각 기능의 write UI는 별도 task/승인 후 추가하고 시스템 Git에서 가능하다는 이유만으로 자동 노출하지 않음
- 상태: 완료 (`[x]`)
- 선행: XXVI.4.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/git-porcelain/src/advanced/features.rs` (신규: `AdvancedWriteFeature`, `AdvancedFeatureFlags`, `FeatureError`, `require_enabled`, 테스트)
- `crates/git-porcelain/src/advanced/mod.rs` (`features` 모듈)
- `docs/task-reports/XXVI.4.2.md`(신규), 계획서(XXVI.4.2 `[ ]`→`[x]`)

## 구현 요약

- XXVI.4.1 읽기 기능에 대응하는 쓰기 기능(tag 생성·삭제, notes 편집, bisect 제어, sparse-checkout 편집, submodule
  update)을 `AdvancedWriteFeature`로 정의하고 안정 이름(`git.tag.create` 등)을 부여했습니다.
- `AdvancedFeatureFlags::default()`는 전부 비활성입니다. `from_capabilities`는 Git capability(XIV.1.3)가 무엇이든
  전부 비활성으로 만듭니다(Git 지원 = 자동 노출 아님).
- `enable`은 기능의 승인된 구현 작업(`approved_task`)이 있어야만 켭니다. 현재 승인된 쓰기 기능이 없으므로 모두
  `NotApproved`입니다. 쓰기 경로는 `require_enabled`를 먼저 통과해야 하며, 꺼져 있으면 Git을 실행하지 않습니다.
- 이 작업은 쓰기 구현을 추가하지 않습니다(정책과 gate만).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 228 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **feature flags default off**: 6개 기능 모두 비활성, `require_enabled` → `Disabled`(`feature_flags_default_off`) | 통과 |
| 모든 capability를 지원하는 Git(2.54 + LFS)으로 만든 flag도 전부 비활성(`git_support_does_not_auto_enable_writes`) | 통과 |
| 승인 없는 기능 `enable` → `NotApproved`, 상태 불변(`unapproved_features_cannot_be_enabled`) | 통과 |
| 비활성 쓰기 경로 모형 → spawn 0(`disabled_write_path_never_runs_git`) | 통과 |
| 기능 이름 고정·중복 없음 | 통과 |

## 차이(작업 지시와 다른 점)

- 승인 기록을 코드 상수(`approved_task`)로 두었습니다. 기능별 쓰기 작업이 승인되면 그 작업에서 해당 기능의 값을
  WBS ID로 바꾸고 구현·테스트를 함께 추가합니다.

## 남은 위험 / 한계

- flag 상태 저장·UI 노출 연결(사용자 설정, command registry)은 UI·설정 단계(XIV.13.7·XVI.11.8)에서 필요합니다.
