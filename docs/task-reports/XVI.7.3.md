# 작업 보고서: XVI.7.3 hook·filter는 시스템 Git이 trusted에서만 실행

- WBS: XVI.7.3
- 작업명: hook·filter는 시스템 Git이 trusted에서만 실행
- 상태: 완료 (`[x]` 후보)
- 선행: XIV.8.5, XVI.7.1
- 결정: ADR-0080

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, XIV.14.1 fixture: 로컬 bare 저장소, 네트워크 없음).

## 변경 파일 목록

- `crates/ops/src/git.rs`
- `crates/runtime/src/staging/engine_tests.rs`
- `tools/stellar-cli/tests/git.rs`

## 구현 요약

- 모든 엔진 Git 실행은 trust gate를 거칩니다. untrusted에서는 status·stage·commit 모두 프로세스 생성 0입니다.
- Stellar는 hook 파일을 직접 실행하지 않습니다(코드 검토: `.git/hooks` 실행 코드 없음, hook은 `git commit` 안에서 시스템 Git이 실행).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops -p stellar-runtime -p stellar-git-porcelain -p stellar-cli -p stellar-mcp-core` | 모두 통과(실패 0) |
| `cargo clippy -p stellar-ops -p stellar-runtime -p stellar-cli -p xtask --all-targets --features stellar-ops/test-support -- -D warnings` | exit 0 |
| `cargo xtask validate-crate-graph` | exit 0 |
| `cargo check --workspace --all-targets` | exit 0 |

| 검사 | 결과 |
|---|---|
| untrusted gate로 세션 열기 → `NotTrusted`, spawn 수 불변 (`untrusted_project_spawns_no_git_for_any_operation`, `ops::git::tests::untrusted_session_spawns_nothing`) | 통과 |
| CLI: 신뢰 등록 없는 프로젝트에서 status·stage·commit 모두 종료 코드 77 `not-trusted`, index·HEAD 불변 (`untrusted_project_is_refused_without_changes`) | 통과 |

## 테스트

- 위 검사 표의 테스트는 `cargo test -p stellar-runtime --lib staging`(17개)과 `cargo test -p stellar-cli --test git`(4개)에 포함되며 모두 통과했습니다.

## 성능

- 성능 예산 대상 경로를 바꾸지 않았고 수치를 측정하지 않았습니다. status·commit 재검증은 HEAD·index를 임시 디렉터리에 materialize합니다(ADR-0080 Performance).

## 차이(작업 지시와 다른 점)

- 셸 FFI와 Swift UI는 사용자 방향(2026-11-23)에 따라 다루지 않았습니다. 엔진은 Rust 테스트와 `stellar git` CLI로 검증했습니다.

## 남은 위험 / 한계

- 없음.
