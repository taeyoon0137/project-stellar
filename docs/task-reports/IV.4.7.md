# 작업 보고서: IV.4.7 생성 rollback 정책

- WBS: IV.4.7
- 작업명: 생성 오케스트레이션의 단계별 rollback 정책(init 이전 롤백 / 이후 유지)
- 상태: 완료 (`[x]`)
- 선행: IV.4.2~IV.4.6

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 git, 로컬 검증; git 테스트는 `cfg(unix)`).

## 변경 파일 목록

- `crates/project/src/create.rs` (`create_project`, `ProjectCreated`, `ProjectCreateError`, `IncompleteReason`, fault injection 테스트)
- `crates/project/Cargo.toml` (`stellar-git-porcelain` 의존 추가)
- `tools/xtask/src/crate_graph.rs` (project 허용 edge에 git-porcelain 추가)
- `docs/architecture/crate-dependency-graph.md` (project 행에 git-porcelain 추가)
- `docs/task-reports/IV.4.7.md`(신규), 계획서(IV.4.7 `[ ]`→`[x]`)

## 구현 요약

- `create_project`가 검증 → 폴더(트랜잭션) → scaffold → git init → add → identity → 커밋을
  오케스트레이션합니다. **rollback 정책**:
  - git init **이전/at** 실패(검증·scaffold·init): `DirTransaction`이 commit 전이라 드롭 시
    폴더를 롤백 제거하고 `Err(ProjectCreateError)`를 반환합니다.
  - git init **이후** 실패(add·identity 누락·commit): git init 직후 `tx.commit()`으로 롤백을
    취소하므로 폴더를 **삭제하지 않고** `Ok(ProjectCreated::InitializationIncomplete)`를
    반환합니다(사용자 복구 가능).
- project가 git-porcelain 연산을 호출하므로 `project → git-porcelain` 의존성을 그래프(doc +
  xtask `crate_graph`)에 반영했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (94 unit) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |
| `cargo run -p xtask -- validate-crate-graph` | 0 |
| `cargo test -p xtask crate_graph` | 0 |

| fault injection | 기대 상태 | 결과 |
|---|---|---|
| **git init 실패(잘못된 git 경로)** | 폴더 롤백 제거, 부모 깨끗 | 통과 |
| **identity 누락(post-init)** | `InitializationIncomplete`, 폴더·`.git` 유지 | 통과 |
| identity 있음 | `Created`, working tree clean | 통과 |

## 차이(작업 지시와 다른 점)

- 오케스트레이션은 git 연산이 필요하므로 `project` crate에 두고 git-porcelain을 직접 의존하게
  했습니다(그래프·validator 갱신). `InitializationIncomplete`는 오류가 아니라 결과 enum의
  한 variant로 표현해, 폴더 유지 의미를 타입으로 분명히 했습니다.
- 빈 `design/`·`assets/`는 git이 빈 디렉터리를 추적하지 않으므로 커밋에는 포함되지 않습니다
  (working tree는 여전히 clean).

## 남은 위험 / 한계

- project_id(UUID)는 호출자가 생성·제공합니다(테스트는 고정 id). UUID 생성기 연결은 앱/상위
  계층 소관입니다. IV.4 섹션(새 프로젝트 생성) 전체가 완료되었습니다.
