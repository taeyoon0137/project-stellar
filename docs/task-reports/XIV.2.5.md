# 작업 보고서: XIV.2.5 Index provider

- WBS: XIV.2.5
- 작업명: index의 stage 0 파일 목록과 blob ID를 `git ls-files --stage -z`로 읽는 provider, unmerged entry는 별도 conflict provider로 전달
- 상태: 완료 (`[x]`)
- 선행: XIV.2.3(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/snapshot/index.rs` (신규: `read_index_snapshot`, `parse_ls_files_stage`, `split_index_entries`, `IndexSnapshot`, `StagedEntry`, 테스트)
- `crates/git-porcelain/src/snapshot/mod.rs` (`index` 모듈, re-export)
- `docs/task-reports/XIV.2.5.md`(신규), 계획서(XIV.2.5 `[ ]`→`[x]`)

## 구현 요약

- `git ls-files --stage -z --full-name -- :/`로 index 전체를 읽습니다. `--full-name` + root pathspec으로
  cwd가 하위 디렉터리여도 root 기준 전체 목록을 받습니다.
- 레코드 `<mode> <oid> <stage>\t<path>`를 해석해 mode(일반·실행·symlink·gitlink만 허용)·ID·stage(0~3)를
  검증합니다.
- stage 0은 `IndexSnapshot::entries`(`SnapshotEntry`), stage 1·2·3은 `IndexSnapshot::unmerged`
  (`StagedEntry`)로 나눕니다. unmerged 경로는 stage 0 목록에 섞지 않으며 conflict provider(XIV.11)가
  같은 해석기(`parse_ls_files_stage`)로 `ls-files -u -z`를 읽습니다.
- 읽기 전용이며 trust gate를 거칩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 147 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **partial staged fixture**: 같은 파일 HEAD=v1·Index=v2·WT=v3 → index blob = v2, HEAD blob = v1; staged 삭제는 index에 없음, staged 새 파일 있음, `update-index --chmod=+x` mode 반영, untracked 없음, 하위 cwd 결과 동일(`partial_staged_fixture_is_exact`) | 통과 |
| merge conflict → 충돌 경로는 stage 0에 없고 unmerged에 stage 1·2·3 = base/ours/theirs blob(`unmerged_entries_go_to_conflict_list_not_stage0`) | 통과 |
| 합성 stage 레코드 분리, 형식 오류 10종 거부 | 통과 |
| untrusted gate → spawn 0 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- `git add -N`(intent-to-add) 항목은 빈 blob ID로 stage 0에 나타납니다. 이를 별도로 표시하지
  않았습니다(Semantic 단계에서 필요하면 `ls-files --debug` 등 추가 조회가 필요).
- sparse index에서는 Git이 목록을 펼쳐 보여 줍니다(`--sparse` 미사용).
