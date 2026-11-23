# 작업 보고서: XIV.8.1 staged snapshot commit 전 재검증

- WBS: XIV.8.1
- 작업명: staged snapshot commit 전 재검증
- 상태: 완료 (`[x]`) — 2026-11-23 cli-mcp-git-cleanup 재판정: staged 재검증과 `stellar validate`가 문서 판정을 같은 함수(`stellar_ops::validate::validate_loaded_document`) 하나로 하고, staged 재검증만 미해결 참조를 오류로 만들던 두 번째 판정을 없앴습니다(ADR-0080 개정 2026-11-23 cli-mcp-git-cleanup). 선행 XIV.6.5·VI.10.1은 `[x]`입니다. 이전 판정: 부분 완료 (`[~]`) — plan-revision 계획서 판정 "판정 이중 경로가 남아 있음"
- 선행: XIV.6.5, VI.10.1
- 결정: ADR-0080

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, XIV.14.1 fixture: 로컬 bare 저장소, 네트워크 없음).

## 변경 파일 목록

- `crates/ops/src/git.rs`(`GitSession::validate_staged`, `GitSession::commit`, 판정 일치 test)
- `crates/ops/src/validate.rs`(`validate_loaded_document`)
- `crates/ops/Cargo.toml`(test 전용 Git fixture dev-dependency 한 줄)
- `crates/runtime/src/staging/engine_tests.rs`(처음 구현)
- `docs/adr/ADR-0080-semantic-git-engine-headless.md`(개정)

## 구현 요약

- `validate_staged`는 실제 index를 작업 트리 밖 비공개 임시 디렉터리에 materialize하고 다음을 검사합니다.
  - conflict 없음, `stellar.yaml` 존재
  - 저장소 loader 무결성(`SavedProjectLoader::for_materialized_snapshot`)
  - 문서마다 문서 판정: `stellar validate`와 같은 `validate_loaded_document`입니다. 읽기 실패(`document-load`), 문서
    validator 위반(`document-invalid`)은 오류, 미해결 참조(`unresolved-reference`)와 정의 없는 인스턴스
    (`missing-component-definition`)는 경고입니다.
  - 저장 무손실 왕복(`saved_semantic_snapshot`)과 semantic 구조 무결성(`validate_snapshot`의 parent·순서·소속·interface 위반)
  - 프로젝트 단위 컴포넌트 원본 읽기(`component-library`)
- `commit`은 오류가 하나라도 있으면 `GitError::InvalidStagedSnapshot`을 돌려주고 `git commit`을 실행하지 않습니다. 검사한
  index와 commit 직전 index가 다르면 `IndexChangedBeforeCommit`으로 멈춥니다.

### 2026-11-23 재판정에서 바꾼 것

- 판정 이중 경로 1: 문서를 읽은 뒤의 판정(읽기 실패 진단, node 수, `validate_records` 호출)을 staged 재검증과
  `stellar validate`가 따로 조합했습니다. `validate_loaded_document`로 합쳐 두 경로는 문서를 어디서 읽는지만 다릅니다.
- 판정 이중 경로 2: staged 재검증은 semantic 무결성의 `DanglingReference`를 인스턴스 정의 참조만 빼고 `snapshot-invalid`
  오류로 냈습니다. 그래서 `stellar validate`가 유효(미해결 참조 경고)로 본 tree의 commit을 막았습니다. 이 동작은 새 test가
  처음 실패로 보여 줬습니다. 참조 대상의 존재는 이제 문서 판정만 보고, semantic 검사에서는 `DanglingReference`를 모두
  뺍니다.
- 같은 이유로 이미지 node가 있는 문서는 asset이 있어도 commit이 막혔습니다. staged semantic snapshot은 문서만 담아 asset이
  없기 때문입니다. 판정 일치 test에 이미지 node를 더해 이 경우를 확인했고, 옛 필터(`IMAGE_ASSET` 참조를 오류로 남김)를
  잠시 되돌리면 이 test가 `snapshot-invalid` 오류로 실패하는 것을 확인했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops` | 83개 통과(실패 0) |
| `cargo test -p stellar-runtime --lib staging` | 19개 통과 |
| `cargo test -p stellar-cli --test git` | 4개 통과 |
| `cargo clippy -p stellar-ops --all-targets --features test-support -- -D warnings` | exit 0 |

| 검사 | 결과 |
|---|---|
| 손상 chunk object를 stage한 index에서 commit → `InvalidStagedSnapshot`, trace sink로 센 `git commit` spawn 0, HEAD 불변 (`invalid_staged_snapshot_never_runs_git_commit`) | 통과 |
| 미해결 clip 참조 2개, 정의 없는 인스턴스 1개, snapshot에 asset이 없는 이미지 node 1개를 가진 문서를 stage → staged 재검증 진단이 `stellar validate`(`validate_project`) 진단과 같고(경고 3개, 오류 0), 재검증 통과 (`git::tests::staged_judgment_matches_project_validation`) | 통과 |

## 테스트

- 위 검사 표의 첫 test는 `cargo test -p stellar-runtime --lib staging`, 둘째 test는 `cargo test -p stellar-ops`에 포함되며
  모두 통과했습니다.

## 성능

- 성능 예산 대상 경로를 바꾸지 않았고 수치를 측정하지 않았습니다. status·commit 재검증은 HEAD·index를 임시 디렉터리에 materialize합니다(ADR-0080 Performance).

## 차이(작업 지시와 다른 점)

- 셸 FFI와 Swift UI는 사용자 방향(2026-11-23)에 따라 다루지 않았습니다. 엔진은 Rust 테스트와 `stellar git` CLI로 검증했습니다.

## 남은 위험 / 한계

- 재검증과 `git commit` 사이 짧은 경합 창은 남습니다(ADR-0080 Consequences).
- 이미지 asset이 실제로 있는지는 `stellar validate`와 staged 재검증 어느 쪽도 판정하지 않습니다(ADR-0080 개정
  cli-mcp-git-cleanup 2).
- staged 재검증은 generation만 읽고 텍스트 원본(`design/**.stellar.yaml`) 정합은 보지 않습니다. 텍스트 원본이 기본 저장이
  되는 V.10.7 전환 때 넣습니다(ADR-0080 개정 integration-followups 결정 2).

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XIV.8.1)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `b14ea25f` Feat: 의미 단위 Git 엔진을 ops에 두고 commit 직전 재검증을 더함
- `7753af02` Test: Git 엔진 통합 시나리오와 작업 보고서 추가
- `2ac9bd6f` Refactor: staged 재검증이 stellar validate의 문서 판정 함수를 쓰고 텍스트 원본을 원본 단위로 나눔
