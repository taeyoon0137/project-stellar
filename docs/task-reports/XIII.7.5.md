# 작업 보고서: XIII.7.5 missing LFS object 진단

- WBS: XIII.7.5
- 작업명: LFS pointer만 있고 object가 없는 asset을 missing LFS diagnostic으로 구분
- 상태: 완료 (`[x]`)
- 선행: XIII.7.1, XIII.5.1 (모두 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git·git-lfs 3.8.1, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/lfs/pointer.rs`(신규) — `LfsPointer`, `parse_lfs_pointer`,
  `MAX_POINTER_SIZE`, `AssetFileState`(진단 코드 3종), `inspect_asset_file`.
- `crates/git-porcelain/src/lfs.rs` — `pointer` 모듈 등록.
- `crates/git-porcelain/src/snapshot/working_tree.rs` — 상위 component 검사(`checked_local_path`)와
  symlink 비추적 읽기(`read_regular_no_follow`, 읽기 상한 인자 추가)를 crate 내부 공유로 분리.
  `read_working_tree_file` 동작은 같습니다.
- `docs/task-reports/XIII.7.5.md`(신규), 계획서(XIII.7.5 `[ ]`→`[x]`).

## 구현 요약

- **pointer 해석**(엄격): 1024 byte 미만, LF로 끝남, 첫 줄 `version https://git-lfs.github.com/spec/v1`
  (구 이름 `hawser` 허용), 이후 key가 엄격한 사전순·중복 없음, 허용 key는 `ext-*`·`oid`·`size`뿐,
  `oid sha256:<소문자 16진수 64자>`, `size`는 선행 0 없는 10진수(u64). 하나라도 다르면 pointer가
  아닙니다(일반 파일로 판정).
- **진단**(`inspect_asset_file(root, path, expected_size)`): 파일 없음(상위 디렉터리 없음 포함) →
  `Missing`(`asset-file-missing`), 작업 트리에 pointer만 있음 → `LfsObjectMissing { pointer }`
  (`asset-lfs-object-missing`), 일반 파일이 아님·상위 symlink·잘못된 경로 → `Unreadable`
  (`asset-file-unreadable`), 그 밖 → `Present`. 1024 byte 이상이거나 파일 크기가 AssetRecord 기대
  크기와 같으면 읽지 않고 실제 내용으로 봅니다(pointer처럼 보이는 실제 텍스트 오판 방지). 읽기는
  1024 byte 상한이며 symlink를 따르지 않습니다.
- 로컬 LFS 저장소에 object가 있는지는 확인하지 않습니다(LFS 저장 경로를 추측하지 않음). 작업 트리에
  pointer만 있으면 어느 경우든 asset 내용을 쓸 수 없기 때문입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | lib 258 passed + doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **일반 missing file과 다른 오류 코드**: 없는 파일 `asset-file-missing`, pointer만 있는 파일 `asset-lfs-object-missing`(서로 다름), 실제 파일 진단 없음 | 통과 |
| 실제 Git+LFS: `lfs install --local` + `lfs track`으로 LFS 커밋 → smudge 없이 clone한 작업 트리의 asset이 `LfsObjectMissing`(커밋된 pointer와 동일), 원본 작업 트리는 `Present`, clone에서 지운 파일은 `asset-file-missing` | 통과 |
| spec pointer 3종(현행·구 version·`ext-*` key) 해석, 비 pointer 19종 + 1024 byte 이상·non-UTF8 거부 | 통과 |
| 기대 크기와 같은 pointer 모양 실제 파일은 `Present`, 잘못된 경로 `asset-file-unreadable` | 통과 |
| 마지막 component symlink·상위 symlink는 따라가지 않고 `Unreadable` | 통과 |
| 실제 Git 테스트가 건너뛰지 않고 실행됨(건너뜀 분기를 일시적으로 panic으로 바꿔 확인 후 원복) | 확인 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 문서 열기·asset 로딩 경로(assets·project·renderer)에서 이 진단을 호출해 누락 asset을 표시하는 연결은
  상위 계층 작업입니다(crate graph상 git-porcelain은 assets crate에 의존하지 않음).
- object가 로컬 LFS 저장소에는 있으나 checkout만 안 된 경우와 원격에서 받지 않은 경우를 구분하지
  않습니다. 해결 안내(`git lfs pull`/`git lfs checkout`)는 UI 문구에서 함께 제시해야 합니다.
