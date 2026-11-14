# 작업 보고서: XIV.5.5 CandidateIndexSnapshot 임시 directory writer

- WBS: XIV.5.5
- 작업명: CandidateIndexSnapshot을 deterministic project files/chunks로 serialize하되 WorkingTree를 수정하지 않는 temp directory writer 구현
- 상태: 부분 완료 (`[~]`) — 임시 directory writer와 검증 기준(write 전후 작업 트리 hash 동일)은 완료. candidate 문서
  레코드를 storage 파일(chunk object·manifest·current pointer)로 직렬화하는 연결은 crate 그래프상 이 lane에서 불가.
- 선행: XIV.5.4(`[x]`, 이 lane), VI.10.2(`[x]`), V.5.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증; 실제 Git fixture는 `cfg(all(test, unix))`).

## 변경 파일 목록

- `crates/git-porcelain/src/candidate_tree.rs`(신규) — `CandidateFile`(KeepIndex·FromWorkingTree·Bytes), `CandidateTree`, `CandidateTreeFile`, `CandidateOrigin`, `CandidateTreeError`, `write_candidate_tree`.
- `crates/git-porcelain/src/lib.rs` — `pub mod candidate_tree;`.
- `docs/task-reports/XIV.5.5.md`(신규), 계획서(XIV.5.5 `[ ]`→`[~]`).

## 구현 요약

- `write_candidate_tree(git, worktree, parent, stellar_files)`는 다음 Index 상태가 될 candidate project tree를
  `parent` 아래 새 `0700` 임시 디렉터리에 씁니다.
  - non-Stellar 파일(XIV.4.4)은 항상 현재 **index**의 blob·mode 그대로(작업 트리 수정분 제외) — 선택 staging이
    non-Stellar 파일을 바꾸지 않게 합니다.
  - Stellar 파일은 serializer가 준 목록이 전부입니다: `KeepIndex`(index 항목 유지), `FromWorkingTree`(디스크에
    저장된 파일, symlink를 따르지 않는 안전 읽기 — 새 asset 등), `Bytes`(새 chunk·manifest·pointer). 목록에 없는
    index Stellar 파일은 candidate에서 빠집니다(삭제).
  - 먼저 목록(non-Stellar 경로·중복·종류)을 검증하고, index에 unmerged 항목이 있으면 거부합니다(conflict 흐름).
    쓰기는 XIV.2.6 materialize 규칙(경로 선검증, `.git` 거부, `create_new`, 상위 symlink 거부, 실패 시 전체
    삭제)을 재사용합니다. submodule은 쓰지 않고 `preserved_gitlinks`로 돌려줍니다. drop 시 tree를 지웁니다.
- 작업 트리는 읽기만, index는 `ls-files --stage`·`cat-file --batch` 읽기만 합니다. 결과 파일 목록은 경로 bytes
  순입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | lib 293 passed + doc 3 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-git-porcelain --all-targets --features test-support -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **write 전후 작업 트리 hash 동일**: commit된 프로젝트 + 미stage 수정(README·pointer·새 chunk·새 asset)에서 candidate(매니페스트·chunk 1 유지, pointer·chunk 2 bytes, photo 작업 트리) 작성 → candidate 파일 bytes 정확(README·.gitignore는 index 내용), 목록에 없는 logo 제외, `.git` 없음, 출처 기록 정확; 작업 트리 전체 bytes·index snapshot이 전후 동일; drop 시 삭제(`candidate_tree_is_written_without_touching_the_working_tree_or_index`) | 통과 |
| non-Stellar 경로·중복·index에 없는 KeepIndex·디스크에 없는 작업 트리 파일·tree 종류·`.git` 경로 거부, 거부 시 임시 디렉터리 0(`invalid_candidate_lists_are_rejected_before_writing`) | 통과 |
| untrusted → `ProjectNotTrusted`, spawn 0(`untrusted_candidate_tree_spawns_nothing`) | 통과 |

## 차이(작업 지시와 다른 점)

- candidate의 **문서 레코드**(XIV.5.4 `CandidateIndexSnapshot::documents`, canonical 정렬)를 storage 파일
  bytes(`serialize_records` chunk model → V.5.1 SavePlan → chunk object·manifest·current pointer)로 바꾸는 부분은
  구현하지 못했습니다. 그 serializer는 project·storage crate에 있고, semantic-diff와 project를 함께 쓰는 crate가
  crate 그래프(`tools/xtask/src/crate_graph.rs`, 이 lane 수정 금지)에 없습니다. 그래서 writer는 serializer 출력을
  `CandidateFile` 목록으로 받는 형태로 두었습니다.

## 남은 위험 / 한계

- `[x]`로 바꾸려면: (1) crate 그래프에 통합 위치(예: project → semantic-diff edge 또는 별도 staging crate)를 ADR로
  정하고, (2) 그 crate에서 candidate 문서 레코드 → `serialize_records` → SavePlan으로 새 generation 파일을 만들어
  `CandidateFile::Bytes`로 넘기는 serializer를 연결하고, (3) 실제 저장 파이프라인으로 만든 프로젝트 fixture에서
  candidate tree를 열어(`SavedProjectLoader`) semantic snapshot이 candidate와 같은지 확인해야 합니다.
- XIV.5.6 이후(file-level diff, blob 기록, index 갱신)는 이 행이 `[~]`라 선행 조건상 진행하지 않았습니다.

## 후속 수정 (리뷰 반영)

- **후보 수 × 작업 트리 파일 수 탐색**: `FromWorkingTree` 항목마다 `listing.files.iter().find`로 찾던 것을 경로 →
  관찰 항목 `BTreeMap` index로 바꿨습니다.
- **index blob 전체를 한 번에 메모리로 읽음**: 모든 non-Stellar·`KeepIndex` blob을 `read_blobs`로 먼저 읽던 것을,
  쓰는 순서대로 `BATCH_CHUNK`(256)개씩 읽어 그 batch만 메모리에 두도록 바꿨습니다(XIV.2.6
  `materialize_snapshot`과 같은 상한). 작업 트리 파일도 미리 모으지 않고 쓸 때 읽으며, `Bytes`는 복사본을 미리
  만들지 않습니다. blob·작업 트리 읽기 실패는 그대로 `CandidateTreeError::Blob`·`WorkingTreeRead`로 돌려주고,
  실패 시 임시 디렉터리는 남지 않습니다.
- 두 항목은 성능·메모리 지적이라 기능 실패 테스트로 재현할 수 없었습니다. 대신 동작 보존 회귀 테스트를 먼저
  추가해 수정 전후 모두 통과함을 확인했습니다: `index_blobs_spanning_several_batches_are_written_exactly`
  (`BATCH_CHUNK`+45개 non-Stellar 파일, 같은 blob ID가 batch 경계 양쪽에 걸침, 작업 트리 asset 40개),
  `unreadable_working_tree_file_keeps_its_error_and_leaves_no_directory`(권한 0 파일 → `WorkingTreeRead`
  `PermissionDenied`, 임시 디렉터리 0). 커밋: `Fix: stage candidate tree writer의 작업 트리 탐색을 경로 index로
  바꾸고 blob을 batch 단위로 읽음`.
- 상태 `[~]`와 그 사유(candidate 문서 레코드 → storage 파일 serializer 연결 없음)는 바뀌지 않습니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | lib 297 passed + doc 3 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-git-porcelain --all-targets --features test-support -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 통과 |
