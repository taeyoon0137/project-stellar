# 작업 보고서: XIV.4.3 untracked Stellar 파일의 WorkingTree 전용 변경 표시

- WBS: XIV.4.3
- 작업명: untracked Stellar files는 WorkingTree snapshot에 포함하되 index snapshot에는 없는 AddObject/File change로 표시
- 상태: 완료 (`[x]`)
- 선행: XIV.4.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/file_status.rs`(신규) — `FileChange`, `StellarFileStatus`, `stellar_file_status(es)`, `read_stellar_file_statuses`.
- `crates/git-porcelain/src/lib.rs` — `pub mod file_status;`.
- `crates/semantic-diff/src/status.rs` — `SemanticStatus::working_tree_additions`, 모듈 문서.
- `docs/task-reports/XIV.4.3.md`(신규), 계획서(XIV.4.3 `[ ]`→`[x]`).

## 구현 요약

- **파일 단위(git-porcelain)**: Stellar 파일(XIV.4.4 분류)의 status 항목을 영역별 `FileChange`
  (Added·Modified·TypeChanged·Deleted·Renamed{from}·Copied{from}·Untracked)로 정리합니다. untracked
  항목은 `unstaged = Untracked`, `staged = None`, `in_index = false`입니다. `D.`(index 삭제)도
  `in_index = false`, `.D`(작업 트리 삭제)는 index에 남습니다. ignored는 WorkingTree snapshot 대상이
  아니라 빼고, unmerged는 conflict 흐름(XIV.11)으로 넘기므로 `conflicted`만 표시합니다. non-Stellar와
  경계 넘는 rename은 raw status(XIV.4.4)에만 남습니다.
- **객체 단위(semantic-diff)**: untracked chunk·asset 파일의 객체는 Index에 없고 WorkingTree에만
  있으므로 unstaged AddObject/AddToken입니다. `working_tree_additions()`가 이를 모읍니다. 같은
  StableId를 다른 종류로 바꾼 교체(삭제+추가)는 Index에도 객체가 있으므로 제외합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff -p stellar-git-porcelain` | git-porcelain 284 passed(+ 통합 3), semantic-diff unit 51 + fuzz 4 passed, 0 failed |
| `cargo clippy -p stellar-semantic-diff -p stellar-git-porcelain --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-semantic-diff -p stellar-git-porcelain -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **untracked chunk/asset fixture(실제 Git)**: commit 후 새 `design/objects/bb/two.starchunk`·`assets/sha256/bb/photo.png`를 쓰고 stage하지 않음 → 두 파일이 `Untracked`(Asset·Design 종류), `in_index=false`; `read_index_snapshot`에는 없고 `list_working_tree_files`에는 있음; 수정된 `design/current.stellarref`는 unstaged Modified·index에 있음; non-Stellar `notes.txt`는 목록에 없음(`untracked_chunk_and_asset_are_working_tree_only_file_changes`) | 통과 |
| 합성 status: `MM`·`A.`·`D.`·`.D`·`.T`·`R.`·`UU`·`??`·`!!`의 영역별 FileChange·in_index·conflicted 정확, README 제외(`status_codes_map_to_file_changes_per_area`) | 통과 |
| **semantic**: 새 View subtree(별도 chunk) 페이지 4·View 5·frame 20·rect 21과 untracked asset 91이 unstaged AddObject로 `working_tree_additions`에 정확히 나오고 Index에는 없음, 문서 페이지 목록·image asset 참조는 속성 변경, 교체의 추가 쪽은 제외(`untracked_chunk_and_asset_objects_are_working_tree_additions`) | 통과 |

## 차이(작업 지시와 다른 점)

- "File change"는 git-porcelain(파일 단위), "AddObject"는 semantic-diff(객체 단위)로 나눴습니다. crate
  그래프상 두 crate가 서로 의존하지 않기 때문입니다.

## 남은 위험 / 한계

- 두 결과를 한 UI 목록으로 합치는 연결(파일 경로 ↔ 객체 매핑)은 semantic-diff와 project를 함께 쓰는
  통합 crate가 필요합니다(XIV.4.1 보고서와 같은 crate 그래프 한계).

## 후속 수정 (리뷰 반영)

- **worktree 쪽 rename·copy 누락**: `file_status::changed`가 rename 항목(`2`)의 원래 경로를 index 쪽(`R.`)에만
  넘겨, intent-to-add 경로의 Index→WorkingTree rename(`.R`)·copy(`.C`)가 unstaged 변경 없음(`None`)으로
  사라졌습니다. 로컬 시스템 Git 2.54에서 `git add -N` 뒤 `2 .R N... ... R100 new\0old`가 실제로 나오는 것을
  확인했고, 아래 두 테스트가 수정 전에 실패하는 것을 본 뒤 `R`·`C` 코드가 있는 영역에 원래 경로를 붙이도록
  고쳤습니다(`Fix: worktree 쪽 rename·copy를 unstaged 변경으로 보존`). Git은 항목마다 원래 경로를 하나만
  내보내고 intent-to-add 경로는 HEAD→Index diff에 나오지 않으므로 두 영역이 동시에 rename/copy인 항목은 없습니다.
  - `worktree_side_rename_and_copy_keep_their_origin_as_unstaged_changes`(합성 `.R`·`.C`·`RM`)
  - `intent_to_add_rename_in_real_git_is_an_unstaged_rename`(실제 Git, `cfg(unix)`)
- **semantic 쪽 fixture는 메모리 구성**(리뷰 minor, 사실 기록): `working_tree_additions` 검사는 XIV.4.1과 같은
  chunk bytes 경유 메모리 fixture입니다. 실제 Git index·작업 트리에서 semantic snapshot을 읽는 연결은 없습니다
  (XIV.4.1 보고서 후속 절). 파일 단위 검사(`untracked_chunk_and_asset_are_working_tree_only_file_changes`)는
  실제 Git입니다. 상태 `[x]`는 유지합니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | lib 297 passed + doc 3 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-git-porcelain --all-targets --features test-support -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 통과 |
