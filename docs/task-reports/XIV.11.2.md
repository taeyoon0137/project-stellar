# 작업 보고서: XIV.11.2 conflict 세 stage virtual project snapshot

- WBS: XIV.11.2
- 작업명: conflict 관련 Stellar project 파일의 세 stage를 각각 virtual project snapshot으로 materialize, conflict 없는 path는 current index 상태와 결합
- 상태: 완료 (`[x]`) — 1차(git-porcelain lane)에서 `[~]`, 2차(project lane)에서 실제 프로젝트 loader로 여는 연결로 완료
- 선행: XIV.2.6(`[x]`), XIV.11.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

1차(git-porcelain lane):

- `crates/git-porcelain/src/conflict_snapshot.rs` (신규: `materialize_conflict_snapshots`, `side_entries`, `ConflictSnapshots`, `ConflictSide`, `ConflictSnapshotError`, 테스트)
- `crates/git-porcelain/src/lib.rs` (`conflict_snapshot` 모듈)

2차(project lane, 이 보고서 갱신):

- `crates/project/src/conflict_open.rs` (신규: `open_conflict_projects`, `ConflictProjects`, `ConflictOpenError`, `CONFLICT_SESSION_PREFIX`)
- `crates/project/src/lib.rs` (`pub mod conflict_open;` 추가)
- `crates/project/tests/conflict_project_open.rs` (신규: 통합 테스트)
- `docs/task-reports/XIV.11.2.md`(갱신), 계획서(XIV.11.2 `[~]`→`[x]`)

## 구현 요약

- index stage 0(conflict 없는 경로, Git이 이미 병합해 stage한 결과)을 세 side 모두에 넣고, Stellar
  프로젝트 파일(XIV.4.4 분류)인 conflict 경로는 각 side의 stage blob(base=1, ours=2, theirs=3)을 넣습니다.
  그 side에 stage가 없으면 넣지 않고, non-Stellar conflict 경로는 `raw_conflicts`로 돌려줍니다. 각 side는
  XIV.2.6 materializer로 별도 임시 디렉터리(`.git` 없음)에 만듭니다. (1차)
- **실제 project open(2차)**: `open_conflict_projects(git, worktree, temp_parent)`는 소유자 전용 session
  root(`PrivateTempDir`, XVI.7.5)를 만들고 그 아래에 세 side를 materialize한 뒤, 각 디렉터리를 XIV.2.7의
  `SavedProjectLoader`(`stellar.yaml` 검증 → current pointer → manifest → chunk 존재, V.4.4 + DocumentManifest
  읽기)로 엽니다. side별 열기 결과(`Result<SavedProject, SavedProjectError>`)를 따로 보존해, 한 side에서
  프로젝트가 지워졌거나 검증이 실패해도 다른 side와 conflict 목록을 계속 씁니다. 결과를 drop하면 세
  snapshot과 session root가 지워집니다. 작업 트리·index는 읽기만 합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

2차(project lane):

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | unit 266 passed; 통합 conflict_project_open 2, document_residency 3, document_round_trip 5, document_serialize 5, private_temp_snapshot 1 passed; restricted_yaml_fuzz는 이 실행에서 입력당 시간 상한(3s) 초과 1건으로 실패(아래 "남은 위험") → `cargo test -p stellar-project --test restricted_yaml_fuzz --test working_tree_project` 재실행 4 + 2 passed |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

1차(git-porcelain lane): `cargo test -p stellar-git-porcelain` 191 passed, clippy·fmt check exit 0.

| 검사 | 결과 |
|---|---|
| **base/ours/theirs project open(실제 loader)**: 실제 저장 파이프라인(V.5)으로 저장한 프로젝트를 두 branch에서 같은 문서를 다르게 저장해 병합 → `design/current` conflict, `README.md` raw conflict. 세 side 모두 `SavedProjectLoader`로 열리고 generation 1/2/2, 문서 semantic snapshot이 각각 base·ours·theirs 문서와 같음, 매니페스트 공통, `.git`·README 없음(`base_ours_theirs_projects_open_with_the_real_loader`) | 통과 |
| 같은 테스트: session root `0700`, 세 snapshot이 그 아래, 열기 전후 `ls-files --stage`·작업 트리 bytes 동일, drop 후 session·snapshot 삭제(임시 parent 비어 있음) | 통과 |
| ours가 `stellar.yaml`을 지우고 theirs가 바꾼 modify/delete conflict → base·theirs는 열리고(각 이름) ours만 `ManifestRead(NotFound)`로 보존(`side_without_the_project_reports_its_own_open_error`) | 통과 |
| (1차) fixture loader 기반 세 side 열기, stage 0 결합, 작업 트리·index 불변(`stellar-git-porcelain`) | 통과 |

## 차이(작업 지시와 다른 점)

- conflict 없는 경로는 index stage 0만 결합합니다(작업 트리의 미stage 변경은 넣지 않음). merge 중 Git이
  병합 결과를 index에 기록하므로 index가 conflict 해결의 기준이라는 아키텍처 §26.3 원칙을 따랐습니다. (1차)
- 실제 loader 구현은 crate 그래프상 project crate에 있으므로 연결 함수도 project crate에 둡니다(2차).

## 남은 위험 / 한계

- generation pointer와 chunk가 서로 다른 stage에서 와 섞이면 그 side 열기가 실패할 수 있습니다. 실패는
  side별 결과로 보존되며 처리 방식은 semantic merge 단계(XIV.11.3)가 정합니다.
- materializer 자체의 drop 정리는 git-porcelain 소관이며, project 쪽은 비공개 session root 정리로 보강합니다.
- 이번 세션은 기계 부하가 매우 높았습니다(load average 약 170). 전체 `cargo test` 병렬 실행 중 이 작업과
  무관한 timing 테스트(제한 YAML fuzz 입력당 3초 상한, FSEvents live)가 한 번씩 실패했고, 단독·재실행에서는
  통과했습니다(YAML·watcher 코드는 이 작업에서 바꾸지 않음).

## 후속 수정(리뷰 반영): snapshot loader가 저장소 symlink·비정규 파일을 따라가지 않음

- **문제**: `SavedProjectLoader`는 `stellar.yaml` symlink만 거부했습니다. materialize는 symlink를 그대로
  만들므로 theirs branch의 `design/current`(또는 manifest·chunk·shard 디렉터리)가 snapshot 밖 절대 경로
  symlink면 loader가 snapshot 밖 파일을 읽었고, `/dev/zero`·FIFO·거대한 파일이면 상한 없는 읽기로
  `open_conflict_projects`가 멈추거나 메모리를 무한히 쓸 수 있었습니다. `stellar.yaml`도 정규 파일 검사와
  크기 상한 없이 `read_to_string`으로 전부 읽었습니다.
- **수정**(`crates/project/src/saved_project.rs`, `conflict_open.rs`):
  - loader에 `StorageLinkPolicy`(`Follow`: 사용자 작업 트리, 아키텍처 19.3 / `Reject`: materialize된
    snapshot)와 `StorageReadLimits`(pointer 1 KiB·manifest 64 MiB·chunk 256 MiB, 비신뢰 입력 방어 상한)를
    두고, `open_conflict_projects`는 `SavedProjectLoader::for_materialized_snapshot()`(Reject)을 씁니다.
  - 저장소 crate가 읽기 **전에** current pointer, generation manifest, manifest가 가리키는 모든 chunk의
    종류·크기를 검사합니다. Reject는 `design/`부터 파일까지 각 구성 요소를 `symlink_metadata`로 검사해
    symlink를 거부하고, 두 규칙 모두 정규 파일이 아니거나 상한을 넘으면 `StorageEntry` 오류로 거부합니다.
    검사한 manifest와 실제로 연 manifest가 다르면 `StorageChanged`로 거부합니다.
  - `stellar.yaml`은 정규 파일만, loader 상한(1 MiB)까지만 읽고(`Read::take`), 검사한 파일과 연 handle의
    device·inode가 다르면 `ManifestChanged`로 거부합니다.
- **검증**: `snapshot_loader_rejects_storage_symlinks_and_working_tree_follows_them`(pointer·shard
  디렉터리·`design` 절대 경로 symlink: Reject 거부, Follow 열림), `non_regular_storage_entries_are_rejected_before_reading`
  (`/dev/zero` pointer, 디렉터리 pointer, 파일인 `design/generations`), `oversized_storage_files_are_rejected_before_reading`,
  `manifest_must_be_a_bounded_regular_file`(디렉터리·64 MiB sparse), `fifo_manifest_is_rejected_without_blocking`,
  통합 테스트 `theirs_storage_symlink_outside_the_snapshot_is_not_followed`(theirs `design/current`가 snapshot 밖
  절대 경로 symlink인 실제 conflict: theirs `StorageEntry{design/current, Symlink}`).
- **남은 한계**: 저장소 파일은 저장소 crate가 경로로 다시 열기 때문에 검사와 읽기 사이 교체(TOCTOU)는 막지
  못합니다. snapshot은 소유자 전용 session 디렉터리 안이라 같은 사용자 프로세스만 이 틈을 쓸 수 있습니다.
  검사 단계가 manifest를 한 번 더 읽어 열기 비용이 manifest 읽기·디코딩 한 번만큼 늘었습니다.
