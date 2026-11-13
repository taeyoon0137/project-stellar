# 작업 보고서: XIV.2.6 blob materialize(SnapshotProvider)

- WBS: XIV.2.6
- 작업명: blob content를 `git cat-file --batch`로 읽고 temp virtual project filesystem에 materialize, `.git` 미생성
- 상태: 완료 (`[x]`)
- 선행: XIV.2.4·XIV.2.5(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/snapshot/blob.rs` (신규: `read_blobs`, `parse_cat_file_batch`, `BlobError`, `BATCH_CHUNK`, 테스트)
- `crates/git-porcelain/src/snapshot/materialize.rs` (신규: `materialize_snapshot`, `MaterializedSnapshot`, `validate_snapshot_path`, `MaterializeError`, `UnsafePathReason`, 테스트)
- `crates/git-porcelain/src/snapshot/mod.rs` (`blob`·`materialize` 모듈, re-export)
- `docs/task-reports/XIV.2.6.md`(신규), 계획서(XIV.2.6 `[ ]`→`[x]`)

## 구현 요약

- **blob 읽기**: ID를 정렬·중복 제거해 256개씩 `git cat-file --batch` stdin으로 넘기고, 응답
  `<oid> <type> <size>\n<content>\n`을 요청 순서대로 해석합니다. ID 불일치·잘린 content·끝 LF 누락·
  남는 출력은 해석 오류, `missing`은 `BlobError::Missing`, blob 아닌 종류는 `NotABlob`입니다.
  content는 filter·textconv 없는 원시 바이트입니다.
- **materialize**: 모든 경로와 mode를 먼저 검증한 뒤(거부 시 Git 실행 0, 파일 생성 0) `parent` 아래
  새 고유 디렉터리(`0700`)를 만들고 파일(`100644`→`0644`, `100755`→`0755`)과 symlink(대상 바이트,
  따라가지 않음)를 씁니다. submodule gitlink는 만들지 않고 `skipped_gitlinks`에 기록합니다.
- **`.git` 미생성·탈출 방지**: `.git` component(대소문자, 끝 공백·점, macOS 무시 Unicode 문자 변형
  포함), 빈 component, `.`/`..`, 절대 경로, NUL을 거부합니다. 파일·symlink는 `create_new`로만 만들어
  같은 경로·대소문자 충돌·파일/디렉터리 충돌을 덮어쓰지 않고 `PathCollision`으로 실패하며, 상위
  component가 symlink면 그 아래에 쓰지 않습니다.
- 실패하면 만든 임시 디렉터리를 지웁니다. 성공 결과(`MaterializedSnapshot`)도 drop 시 지우고,
  `keep()`으로만 보존합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 155 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **binary chunk·symlink fixture bytes 동일**: 1 MiB 의사 난수 `.starchunk`, 0 바이트 8 KiB chunk, 0~255 반복 바이너리, 빈 파일, CRLF 텍스트, 실행 파일, 내부·외부(상위 경로·한글) symlink를 HEAD와 Index에서 각각 materialize → 파일 bytes·symlink 대상·mode(`0755`/`0644`)·root `0700` 일치, `.git` 없음, 외부 대상 미생성(`binary_chunk_and_symlink_bytes_are_identical_from_head_and_index`) | 통과 |
| drop 시 삭제, `keep()` 시 보존 | 통과 |
| gitlink 미생성·기록(`gitlinks_are_skipped_and_recorded`) | 통과 |
| `.git/hooks/...`·`x/../../escape`·절대 경로 → `UnsafePath`, spawn 0, 디렉터리 생성 0(`unsafe_entries_spawn_nothing_and_create_nothing`) | 통과 |
| symlink 상위 아래 쓰기·파일/디렉터리 충돌·중복 경로 → `PathCollision`, 외부 쓰기 없음, 실패 시 임시 디렉터리 잔존 0, 없는 blob → `Missing`(`collisions_and_symlink_parents_do_not_escape`) | 통과 |
| 경로 검증 표(거부 14종, 허용 7종: `.gitignore`·`.github/...`·non-UTF8 등) | 통과 |
| `cat-file --batch` 해석(NUL·LF 포함 binary, missing, tree, 형식 오류 8종) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- blob은 chunk(256개) 단위로 메모리에 읽습니다. 매우 큰 단일 blob(수 GB)에 대한 streaming은 구현하지
  않았습니다.
- 임시 디렉터리 권한·정리 강화는 XVI.7.5에서 별도로 다룹니다. 여기서는 root `0700`과 drop 정리만 합니다.
- APFS는 non-UTF8 이름을 만들 수 없어 그런 경로는 `Io`로 실패합니다(Git 경로 자체는 보존됨).
- Windows는 symlink를 만들지 않고 `SymlinkUnsupported`를 반환합니다(ADR-0006, 미검증).
- `refs/replace`가 있으면 Git checkout과 같게 replace된 content를 읽습니다.
