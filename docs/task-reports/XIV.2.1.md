# 작업 보고서: XIV.2.1 `git status --porcelain=v2 -z --branch` parser

- WBS: XIV.2.1
- 작업명: porcelain v2 + `-z` status parser — ordinary, rename/copy, unmerged, untracked, ignored 구분
- 상태: 완료 (`[x]`)
- 선행: XIV.1.5(`[x]`, `-z` 레코드 분해·원시 경로)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/status.rs` (신규: `parse_status_v2`, `read_status`, `status_args`, `StatusReport`·`StatusEntry`·`BranchHeaders` 등, 테스트)
- `crates/git-porcelain/src/object.rs` (신규: 검증된 `ObjectId`(40/64자 소문자 hex), `FileMode`(Git mode만 허용), `ObjectKind`)
- `crates/git-porcelain/src/ops.rs` (신규: gate 경유 실행 `run_ok`, 공통 오류 `GitOpError`·`ParseFailure`)
- `crates/git-porcelain/src/test_support.rs` (`write_blob`, `add_index_path` 추가)
- `crates/git-porcelain/src/lib.rs` (`object`, `ops`, `status` 모듈)
- `docs/task-reports/XIV.2.1.md`(신규), 계획서(XIV.2.1 `[ ]`→`[x]`)

## 구현 요약

- `-z` 출력을 NUL 레코드로 나눈 뒤 첫 글자로 종류를 판정합니다: `#` header, `1` ordinary,
  `2` rename/copy(다음 레코드가 원래 경로), `u` unmerged, `?` untracked, `!` ignored. 앞쪽 고정
  필드만 공백으로 나누고 나머지 전체를 경로로 보존하므로 공백·개행·non-UTF8 경로가 그대로
  `GitPath`(원시 바이트)로 남습니다.
- `XY`는 `StatusCode`(`.MTADRCU`), unmerged `XY`는 `UnmergedKind`(DD/AU/UD/UA/DU/AA/UU),
  `sub`는 `SubmoduleState`, mode·object ID는 검증 타입으로 해석합니다. rename/copy는
  `RenameKind`와 점수(0~100)를 가집니다.
- `--branch` header는 `BranchOid`(`(initial)`=unborn), `BranchHead`(`(detached)`), upstream,
  `AheadBehind`로 해석합니다. Git 문서대로 모르는 header(`# stash` 등)는 무시하되 원시 바이트를
  `unknown_headers`에 보존합니다.
- 모르는 레코드 종류, 필드 부족, 잘못된 XY·mode·oid·score, 원래 경로 누락, NUL 미종료, 중복
  header, `branch.oid`/`branch.head` 누락은 `ParseFailure`로 실패합니다(부분 결과를 성공으로
  돌려주지 않음).
- `read_status`는 trust gate(`GatedGitRunner`)를 거쳐 `git --no-optional-locks status
  --porcelain=v2 -z --branch --untracked-files=all [--ignored=matching]`을 실행합니다.
  `--no-optional-locks`로 사용자의 다른 Git 명령과 index lock이 경합하지 않게 합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 125 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| 합성 fixture corpus 12종(빈 출력, branch only, unborn, detached+`# stash`, upstream+ab, 공백 경로, rename, copy, unmerged, submodule `SCMU`, non-UTF8·개행 경로 untracked/ignored, SHA-256 ID) 전부 parse(`whole_fixture_corpus_parses`) | 통과 |
| 항목 종류·필드 값 구분(`entry_kinds_are_distinguished`, `branch_headers_are_typed`, `submodule_flags_parse`) | 통과 |
| 형식 오류 17종 모두 실패 반환(`malformed_records_fail_without_partial_success`) | 통과 |
| 실제 Git 출력: `.M`·`M.`·`MM`·`.D`·`A.`·`.T`(file→symlink)·mode 변경·한글 공백 경로·rename·untracked·ignored·non-UTF8 경로(`.D`, `AD`) (`real_mixed_repository_status_parses_every_kind`) | 통과 |
| 실제 Git copy 감지 출력(`status.renames=copies`) → `Copied` + 원래 경로(`real_copy_detection_output_parses`) | 통과 |
| 실제 merge conflict → `UU`·`DU`·`AA` unmerged(`real_merge_conflict_output_parses_unmerged`) | 통과 |
| untrusted gate → `ProjectNotTrusted`, spawn 0(`untrusted_status_spawns_nothing`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. (rename 감지 여부는 사용자 Git config를 따르며 parser는 rename·copy 모두 처리합니다.)

## 남은 위험 / 한계

- macOS APFS는 non-UTF8 파일 이름을 만들 수 없어, 실제 Git fixture의 non-UTF8 경로는
  `update-index --cacheinfo`로 index에만 등록해 검증했습니다(untracked non-UTF8 파일은 합성
  fixture로만 검증).
- Windows Git 출력은 이 환경에서 실행하지 않았습니다(ADR-0006, macOS 우선). parser는 바이트 형식만
  다루므로 플랫폼 의존 값은 없습니다.
