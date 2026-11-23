# 작업 보고서: XIV.14.3 component/token dependency staging scenario

- WBS: XIV.14.3
- 작업명: component/token dependency staging scenario
- 상태: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 남은 범위가 있음(아래 본문). 보고 당시 기록: 부분 완료 (`[~]` 후보)
- 선행: XIV.5.4
- 결정: ADR-0080

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, XIV.14.1 fixture: 로컬 bare 저장소, 네트워크 없음).

## 변경 파일 목록

- `crates/ops/src/git.rs`(원본 단위 동반 판정)
- `crates/runtime/src/staging/engine_tests.rs`
- `tools/stellar-cli/tests/git.rs`

## 구현 요약

- 구조 의존: 새 Frame 아래로 기존 node를 옮긴 뒤 위치 변경만 stage하면 `CompanionRequired`(새 Frame 추가 포함)이고 index bytes는 그대로입니다. 필수가 아닌 승인은 `UnexpectedApproval`입니다. 승인하면 성공하고 재검증을 통과합니다.
- 컴포넌트 의존: 새 컴포넌트 원본과 그 인스턴스를 배치한 View에서 `generation`만 stage하면 `CompanionRequired`(단위 `file:design/components/...`, 이유: 정의 id)입니다. 승인하면 두 단위가 staged입니다. 정의 파일만 unstage하면 `generation` 동반이 필요합니다. commit은 경고 없이 성공합니다.
- 정의 삭제: 작업 트리에서 정의와 인스턴스를 함께 지운 뒤 정의 파일만 stage해도 `generation` 동반이 필요합니다(오류가 아니라 승인 목록).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops -p stellar-runtime -p stellar-git-porcelain -p stellar-cli -p stellar-mcp-core` | 모두 통과(실패 0) |
| `cargo clippy -p stellar-ops -p stellar-runtime -p stellar-cli -p xtask --all-targets --features stellar-ops/test-support -- -D warnings` | exit 0 |
| `cargo xtask validate-crate-graph` | exit 0 |
| `cargo check --workspace --all-targets` | exit 0 |

| 검사 | 결과 |
|---|---|
| 구조 의존 차단 → 승인 후 성공 (`moving_into_new_frame_requires_approval_of_the_addition`) | 통과 |
| 컴포넌트 정의 동반 차단 → 승인 후 성공, unstage 동반 (`component_definition_is_a_required_companion_unit_not_auto_included`) | 통과 |
| 작업 트리에서 정의와 인스턴스를 함께 지운 뒤 정의 파일만 stage하면 `generation` 동반 요구 → 승인 후 경고 없이 commit (`staging_definition_removal_requires_generation_companion_not_dangling_error`) | 통과 |
| CLI: `stage generation` 종료 코드 1 `companion-required`, `--with`로 승인 후 commit (`companion_component_is_listed_and_included_only_with_approval`) | 통과 |

## 테스트

- 위 검사 표의 테스트는 `cargo test -p stellar-runtime --lib staging`(19개)과 `cargo test -p stellar-cli --test git`(4개)에 포함되며 모두 통과했습니다.

## 성능

- 성능 예산 대상 경로를 바꾸지 않았고 수치를 측정하지 않았습니다. status·commit 재검증은 HEAD·index를 임시 디렉터리에 materialize합니다(ADR-0080 Performance).

## 차이(작업 지시와 다른 점)

- 셸 FFI와 Swift UI는 사용자 방향(2026-11-23)에 따라 다루지 않았습니다. 엔진은 Rust 테스트와 `stellar git` CLI로 검증했습니다.

## 남은 위험 / 한계

- token은 저장 포맷이 없어(문서 레코드에 token 없음) 저장 프로젝트 scenario를 만들 수 없습니다. token·binding 의존은 semantic-diff dependency graph(XIV.5.2) 단위 테스트로만 검증됩니다. token 저장 포맷이 생기면 같은 scenario를 추가해야 합니다.
