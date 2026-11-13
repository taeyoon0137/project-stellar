# 작업 보고서: XIV.2.7 WorkingTree provider(디스크 저장 파일 기준)

- WBS: XIV.2.7
- 작업명: WorkingTree provider는 disk project loader를 사용하고 저장 대기 memory state와 구분, Git status는 저장된 file 기준임을 명시
- 상태: 부분 완료 (`[~]`)
- 선행: V.4.4(`[x]`), XIV.2.3(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/snapshot/working_tree.rs` (신규: `DiskProjectLoader`, `SnapshotBasis`, `WorkingTreeSnapshot`, `WorkingTreeListing`, `list_working_tree_files`, `read_working_tree_file`, `read_working_tree_snapshot`, 테스트)
- `crates/git-porcelain/src/snapshot/mod.rs` (`working_tree` 모듈, re-export)
- `crates/git-porcelain/src/repo_status.rs` (`RepositoryStatus::basis()` = 디스크 저장 파일)
- `docs/task-reports/XIV.2.7.md`(신규), 계획서(XIV.2.7 `[ ]`→`[~]`)

## 구현 요약

- `SnapshotBasis::SavedFilesOnDisk`로 WorkingTree snapshot과 Git status의 기준을 명시합니다
  (`WorkingTreeSnapshot::basis`, `RepositoryStatus::basis()`, 모듈 문서). 편집기 메모리의 저장 대기
  상태는 provider 입력에 존재하지 않습니다.
- 파일 목록은 `git ls-files -z --cached --others --exclude-standard --full-name -- :/`로 받고 각 경로를
  `symlink_metadata`로 관찰해 일반·실행·symlink 파일, 디스크에 없는 tracked(`missing_tracked`), 파일이
  아닌 항목(`skipped`)으로 나눕니다. `read_working_tree_file`은 경로 검증 후 상위 symlink를 거부하고
  디스크 bytes(symlink는 대상 경로 바이트)를 읽습니다.
- 프로젝트 내용은 호출자가 공급하는 `DiskProjectLoader`(V.4.4 프로젝트 열기 loader를 연결하는 trait)로
  같은 root를 디스크에서 읽습니다. loader 실패는 `project: Err(..)`로 보존하고 파일 목록은 계속
  제공합니다(XIV.4.5의 raw status 유지와 같은 방향).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 157 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **unsaved memory edit fixture**: 편집기 모형 문서가 메모리에서만 `stellar.yaml`을 수정 → `git status` 항목 0, `RepositoryStatus` unstaged 0·basis `SavedFilesOnDisk`, WorkingTree snapshot loader 결과·파일 길이·bytes는 디스크 내용; 저장 후에만 status `.M`·snapshot 반영(`unsaved_memory_edit_does_not_appear_in_git_status_or_snapshot`) | 통과 |
| 파일 종류(일반·실행·symlink·untracked), ignored 제외, 삭제된 tracked는 `missing_tracked`, loader 실패 보존(`file_list_distinguishes_kinds_missing_and_skipped`) | 통과 |

## 차이(작업 지시와 다른 점)

- V.4.4 loader(storage crate)를 직접 호출하지 않고 `DiskProjectLoader` trait로 받습니다.
  crate 그래프(II.2.8)상 git-porcelain은 core-types에만 의존할 수 있기 때문입니다. 테스트는
  `stellar.yaml`을 디스크에서 읽는 fixture loader를 씁니다.

## 남은 위험 / 한계

- **`[~]` 사유**: 실제 V.4.4 loader(`stellar-storage` generation loader)를 `DiskProjectLoader`로
  연결하는 adapter와 그 통합 테스트는 project crate 통합 단계에서 필요합니다(이 lane의 허용 경로 밖).
- WorkingTree 파일 내용은 on-demand로 읽으며 blob ID를 계산하지 않습니다(필요 시 `hash-object` 추가).
- 실행 비트는 디스크 mode로만 판정합니다(`core.fileMode=false` 저장소의 Git 판정과 다를 수 있음).
