# 작업 보고서: XIV.8.1 staged snapshot commit 전 재검증

- WBS: XIV.8.1
- 작업명: staged snapshot commit 전 재검증
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록(ledger-refresh): `[~]`의 이유였던 판정 이중 경로를 integration-followups lane(`2ac9bd6f`)이 없앴습니다. `GitSession::validate_staged`의 문서 판정은 `stellar validate`와 같은 `stellar_ops::validate::validate_records`입니다. 선행이 모두 `[x]`입니다. 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 원장이 근거 커밋을 찾지 못했습니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 판정 이중 경로가 남아 있음: `crates/ops/src/git.rs`의 `validate_staged`가 `crates/ops/src/validate.rs`와 같은 검증(`validate_document` 호출 등)을 따로 조합합니다. 행 지시는 충족하지만 같은 판정을 두 곳에서 하지 않는다는 설계 규칙에 어긋나므로, integration-followups lane이 ops validate 판정을 records·definitions 입력 함수로 노출해 합치면 `[x]`. 보고 당시 기록: 완료 (`[x]` 후보)
- 선행: XIV.6.5, VI.10.1
- 결정: ADR-0080

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, XIV.14.1 fixture: 로컬 bare 저장소, 네트워크 없음).

## 변경 파일 목록

- `crates/ops/src/git.rs`(`GitSession::validate_staged`, `GitSession::commit`)
- `crates/runtime/src/staging/engine_tests.rs`

## 구현 요약

- `validate_staged`는 실제 index를 작업 트리 밖 비공개 임시 디렉터리에 materialize하고 다음을 검사합니다.
  - conflict 없음, `stellar.yaml` 존재
  - 저장소 loader 무결성(`SavedProjectLoader::for_materialized_snapshot`)
  - 문서마다 문서 validator(VI.10.1)
  - 저장 무손실 왕복(`saved_semantic_snapshot`)과 semantic 무결성(`validate_snapshot`)
  - 컴포넌트 원본 읽기. 정의 없는 인스턴스는 `stellar validate`와 같이 경고입니다.
- `commit`은 오류가 하나라도 있으면 `GitError::InvalidStagedSnapshot`을 돌려주고 `git commit`을 실행하지 않습니다. 검사한 index와 commit 직전 index가 다르면 `IndexChangedBeforeCommit`으로 멈춥니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops -p stellar-runtime -p stellar-git-porcelain -p stellar-cli -p stellar-mcp-core` | 모두 통과(실패 0) |
| `cargo clippy -p stellar-ops -p stellar-runtime -p stellar-cli -p xtask --all-targets --features stellar-ops/test-support -- -D warnings` | exit 0 |
| `cargo xtask validate-crate-graph` | exit 0 |
| `cargo check --workspace --all-targets` | exit 0 |

| 검사 | 결과 |
|---|---|
| 손상 chunk object를 stage한 index에서 commit → `InvalidStagedSnapshot`, trace sink로 센 `git commit` spawn 0, HEAD 불변 (`invalid_staged_snapshot_never_runs_git_commit`) | 통과 |

## 테스트

- 위 검사 표의 테스트는 `cargo test -p stellar-runtime --lib staging`(19개)과 `cargo test -p stellar-cli --test git`(4개)에 포함되며 모두 통과했습니다.

## 성능

- 성능 예산 대상 경로를 바꾸지 않았고 수치를 측정하지 않았습니다. status·commit 재검증은 HEAD·index를 임시 디렉터리에 materialize합니다(ADR-0080 Performance).

## 차이(작업 지시와 다른 점)

- 셸 FFI와 Swift UI는 사용자 방향(2026-11-23)에 따라 다루지 않았습니다. 엔진은 Rust 테스트와 `stellar git` CLI로 검증했습니다.

## 남은 위험 / 한계

- 재검증과 `git commit` 사이 짧은 경합 창은 남습니다(ADR-0080 Consequences).

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XIV.8.1)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `b14ea25f` Feat: 의미 단위 Git 엔진을 ops에 두고 commit 직전 재검증을 더함
- `7753af02` Test: Git 엔진 통합 시나리오와 작업 보고서 추가
- `2ac9bd6f` Refactor: staged 재검증이 stellar validate의 문서 판정 함수를 쓰고 텍스트 원본을 원본 단위로 나눔
