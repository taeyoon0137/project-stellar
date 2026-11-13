# 작업 보고서: XIV.2.7 WorkingTree provider(디스크 저장 파일 기준)

- WBS: XIV.2.7
- 작업명: WorkingTree provider는 disk project loader를 사용하고 저장 대기 memory state와 구분, Git status는 저장된 file 기준임을 명시
- 상태: 완료 (`[x]`) — 1차(git-porcelain lane)에서 `[~]`, 2차(project lane)에서 실제 loader 연결로 완료
- 선행: V.4.4(`[x]`), XIV.2.3(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

1차(git-porcelain lane):

- `crates/git-porcelain/src/snapshot/working_tree.rs` (신규: `DiskProjectLoader`, `SnapshotBasis`, `WorkingTreeSnapshot`, `WorkingTreeListing`, `list_working_tree_files`, `read_working_tree_file`, `read_working_tree_snapshot`, 테스트)
- `crates/git-porcelain/src/snapshot/mod.rs` (`working_tree` 모듈, re-export)
- `crates/git-porcelain/src/repo_status.rs` (`RepositoryStatus::basis()` = 디스크 저장 파일)

2차(project lane, 이 보고서 갱신):

- `crates/project/src/saved_project.rs` (신규: `SavedProjectLoader`, `SavedProject`, `SavedProjectError`, `read_working_tree_project`, 단위 테스트)
- `crates/project/src/lib.rs` (`pub mod saved_project;` 추가)
- `crates/project/tests/common/git.rs` (신규: 시스템 Git·저장 파이프라인 fixture helper), `crates/project/tests/common/mod.rs` (`pub mod git;`)
- `crates/project/tests/working_tree_project.rs` (신규: 통합 테스트)
- `docs/task-reports/XIV.2.7.md`(갱신), 계획서(XIV.2.7 `[~]`→`[x]`)

## 구현 요약

- `SnapshotBasis::SavedFilesOnDisk`로 WorkingTree snapshot과 Git status의 기준을 명시합니다
  (`WorkingTreeSnapshot::basis`, `RepositoryStatus::basis()`, 모듈 문서). 편집기 메모리의 저장 대기
  상태는 provider 입력에 존재하지 않습니다. (1차)
- 파일 목록은 `git ls-files -z --cached --others --exclude-standard --full-name -- :/`와
  `symlink_metadata` 관찰로 만듭니다. (1차)
- **실제 disk project loader(2차)**: `SavedProjectLoader`가 `DiskProjectLoader`를 구현합니다.
  `stellar.yaml`을 제한 YAML(IV.1.2)·스키마(IV.1.1)로 검증해 project id(IV.2.3)를 얻고,
  `LazyGeneration::open`(V.4.4 current pointer → manifest → chunk 존재 검증 + V.6.6 DocumentManifest
  읽기)으로 current generation을 엽니다. 문서 내용은 `SavedProject::load_document`(VI.10.3)로 요청
  시 읽습니다. `.git`은 요구하지 않고(materialize된 snapshot에도 쓰기 위함), symlink인
  `stellar.yaml`은 따르지 않고 `ManifestSymlink`로 거부합니다. 실패는 typed error
  (`ManifestRead`·`ManifestSyntax`·`ManifestSchema`·`InvalidProjectId`·`Storage`)입니다.
- `read_working_tree_project(git, root)`는 `read_working_tree_snapshot`에 이 loader를 연결한
  WorkingTree provider입니다. loader 실패는 `project: Err(..)`로 보존하고 파일 목록은 계속 제공합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

2차(project lane):

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 259 passed; 통합 document_residency 3, document_round_trip 5, document_serialize 5, restricted_yaml_fuzz 4, working_tree_project 2 passed; 0 failed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

1차(git-porcelain lane): `cargo test -p stellar-git-porcelain` 157 passed, clippy·fmt check exit 0.

| 검사 | 결과 |
|---|---|
| **unsaved memory edit fixture(실제 loader)**: 실제 저장 파이프라인(V.5)으로 저장·커밋한 Stellar 프로젝트에서 메모리 문서만 `set_display_name`으로 수정 → `git status` 항목 0, `RepositoryStatus` unstaged·untracked 0·basis `SavedFilesOnDisk`, `read_working_tree_project`의 프로젝트는 generation 1이고 문서 semantic snapshot이 디스크(커밋) 내용과 같고 메모리 편집과 다름; 메모리 문서를 저장한 뒤에만 `design/current`가 unstaged로, 새 manifest·chunk가 untracked로 나타나고 snapshot은 generation 2·메모리 내용(`unsaved_memory_edit_does_not_appear_in_git_status_or_working_tree_project`) | 통과 |
| current manifest가 지워진 손상 상태에서 loader 실패는 `project: Err`로 보존, 파일 목록·`missing_tracked` 유지(`working_tree_project_failure_keeps_the_file_listing`) | 통과 |
| loader 단위: 매니페스트·current generation 열기와 문서 두 개 로드(원본과 semantic snapshot 동일), `.git` 없이 열림(`opens_manifest_and_current_generation_and_loads_documents`) | 통과 |
| loader 단위: 매니페스트 없음·문법 오류·스키마 오류·UUID 아님·current pointer 없음·project id 불일치를 typed error로 거부(`invalid_projects_are_rejected_with_typed_errors`), symlink 매니페스트 거부(`symlinked_manifest_is_not_followed`) | 통과 |
| (1차) fixture loader 기반 unsaved edit·파일 종류 검사(`stellar-git-porcelain`) | 통과 |

## 차이(작업 지시와 다른 점)

- git-porcelain은 crate 그래프(II.2.8)상 core-types에만 의존하므로 loader는 `DiskProjectLoader`
  trait로 받고, 실제 구현(`SavedProjectLoader`)은 project crate에 둡니다.
- loader는 프로젝트 열기 probe(IV.5.1)의 root 검증(`.git` 필수)을 쓰지 않습니다. 같은 loader를
  `.git`이 없는 materialize snapshot(XIV.11.2)에도 쓰기 때문입니다. 매니페스트 parse·스키마 검증은 같은
  규칙입니다.

## 남은 위험 / 한계

- storage 계층(`design/` 아래 pointer·manifest·chunk 읽기)은 symlink를 따라갑니다. snapshot 안의
  `design/` symlink 처리 정책은 storage 쪽 결정이 필요합니다(이 작업은 `stellar.yaml`만 거부).
- WorkingTree 파일 내용은 on-demand로 읽으며 blob ID를 계산하지 않습니다(필요 시 `hash-object` 추가).
- 실행 비트는 디스크 mode로만 판정합니다(`core.fileMode=false` 저장소의 Git 판정과 다를 수 있음).
- 전체 `cargo test` 병렬 실행 중 이 작업과 무관한 timing 테스트(FSEvents live, 제한 YAML fuzz의 입력당
  시간 상한)가 한 번씩 실패했고 단독·재실행에서는 통과했습니다(부하 의존 flake로 판단, 원인 수정은 이
  작업 범위 밖).
