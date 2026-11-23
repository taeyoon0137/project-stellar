# 작업 보고서: XIV.14.2 partial same-chunk property stage·unstage·commit scenario

- WBS: XIV.14.2
- 작업명: partial same-chunk property stage·unstage·commit scenario
- 상태: 완료 (`[x]` 후보)
- 선행: XIV.6.6, XIV.7.3, XIV.8.4
- 결정: ADR-0080

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, XIV.14.1 fixture: 로컬 bare 저장소, 네트워크 없음).

## 변경 파일 목록

- `crates/runtime/src/staging/engine_tests.rs`

## 구현 요약

- 같은 root node의 `display_name`·`locked`(같은 chunk)를 함께 바꾸고 `display_name`만 stage → unstage → 다시 stage → commit합니다.
- 기대값: staged=[이름], unstaged=[locked]. unstage 뒤 staged 0이고 `git diff --cached`가 비어 있습니다. commit 뒤 staged 0, unstaged=[locked], 작업 트리 `design/current` 불변입니다.
- 선택 결과가 원천과 의미가 같으면 원천 bytes를 쓰도록 했습니다(ADR-0080 결정 6). 그래서 전부 unstage하면 file-level diff도 사라집니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops -p stellar-runtime -p stellar-git-porcelain -p stellar-cli -p stellar-mcp-core` | 모두 통과(실패 0) |
| `cargo clippy -p stellar-ops -p stellar-runtime -p stellar-cli -p xtask --all-targets --features stellar-ops/test-support -- -D warnings` | exit 0 |
| `cargo xtask validate-crate-graph` | exit 0 |
| `cargo check --workspace --all-targets` | exit 0 |

| 검사 | 결과 |
|---|---|
| HEAD/Index/WT 기대값 일치, `git show --name-only HEAD`가 `design/` 3개 파일(chunk·generation·current)뿐 (`partial_same_chunk_property_stage_unstage_commit`) | 통과 |

## 테스트

- 위 검사 표의 테스트는 `cargo test -p stellar-runtime --lib staging`(19개)과 `cargo test -p stellar-cli --test git`(4개)에 포함되며 모두 통과했습니다.

## 성능

- 성능 예산 대상 경로를 바꾸지 않았고 수치를 측정하지 않았습니다. status·commit 재검증은 HEAD·index를 임시 디렉터리에 materialize합니다(ADR-0080 Performance).

## 차이(작업 지시와 다른 점)

- 셸 FFI와 Swift UI는 사용자 방향(2026-11-23)에 따라 다루지 않았습니다. 엔진은 Rust 테스트와 `stellar git` CLI로 검증했습니다.

## 남은 위험 / 한계

- generation 원본이 바이너리라 `git show`에서 "속성 줄"을 직접 볼 수 없습니다. semantic 기대값과 바뀐 파일 목록으로 확인했습니다. 줄 단위 확인은 View 텍스트 원본 전환 뒤에 가능합니다.
