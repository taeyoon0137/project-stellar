# 작업 보고서: XVI.7.5 임시 디렉터리 권한·정리 강화

- WBS: XVI.7.5
- 작업명: temp directory permissions와 cleanup을 강화하고 다른 사용자 접근을 금지
- 상태: 완료 (`[x]`)
- 선행: XIV.2.6(`[x]`), IV.6.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 권한·Git fixture는 `cfg(unix)`).
  지원 플랫폼은 ADR-0006에 따라 macOS뿐이므로 "platform permission test"는 macOS 증거입니다.

## 변경 파일 목록

- `crates/project/src/private_temp.rs` (신규: `PrivateTempDir`, `PrivateTempError`, `remove_private_tree`,
  `sweep_stale_private_temps`, `SweepReport`, `PRIVATE_TEMP_MARKER`, `PRIVATE_DIR_MODE`, 최소 libc 선언
  `geteuid`·`kill`, 테스트)
- `crates/project/src/clone.rs` (`clone_project`가 비공개 staging 안에서 clone, 잔여 staging 정리,
  `CloneError::Staging`, `CLONE_STAGING_PREFIX`, 테스트 2개 추가)
- `crates/project/src/lib.rs` (`pub mod private_temp;` 추가)
- `crates/project/tests/private_temp_snapshot.rs` (신규: snapshot materialize 권한·정리 통합 테스트)
- `docs/task-reports/XVI.7.5.md`(신규), 계획서(XVI.7.5 `[ ]`→`[x]`)

## 구현 요약

- **비공개 생성**: `PrivateTempDir::create_in(parent, prefix)`는 `<prefix>-<pid>-<n>-<nanos>` 디렉터리를
  `DirBuilder::mode(0700)`로 만들면서 권한을 정하고(생성 후 chmod 사이의 창 없음), umask와 무관하게 정확히
  `0700`으로 맞춘 뒤 symlink 아닌 실제 디렉터리·effective 사용자 소유·`0700`을 다시 확인합니다. 조건을
  못 맞추면 지우고 `NotPrivate`입니다. 안에 `0600` 표식 파일(`.stellar-private-temp`)을 둡니다. 접두사는
  `[A-Za-z0-9._-]`만 허용합니다.
- **정리 강화**: drop(오류·panic 경로 포함)과 `cleanup()`은 `remove_private_tree`를 씁니다. 그대로
  지우다 실패하면 트리를 명시 stack으로 반복 순회하며(재귀 없음, symlink 미추종, 이 사용자 소유
  디렉터리만) 소유자 `rwx`를 되돌린 뒤 다시 지웁니다. 읽기 전용·권한 0 하위 디렉터리가 있어도 지워지고
  링크 대상(프로젝트 밖)은 건드리지 않습니다.
- **잔여 정리**: `sweep_stale_private_temps(parent, prefix)`는 이름 `<prefix>-<pid>-…`, 실제 디렉터리,
  이 사용자 소유, `0700`, 표식 파일이 모두 맞고 `kill(pid, 0)`이 `ESRCH`(process 없음)인 항목만 지웁니다.
  이 process 자신·살아 있는 process(권한 없음 `EPERM` 포함)는 `live`로 남깁니다.
- **clone(IV.6.2) 연결**: `clone_project`는 부모 아래 비공개 staging을 만들고 그 안의 `checkout`으로
  `git clone`을 실행합니다. clone 중 내용은 다른 사용자가 목록·내용 모두 접근할 수 없고, 성공하면
  checkout을 최종 폴더로 rename한 뒤 빈 staging을 지웁니다. 실패·rename 실패는 staging drop으로 정리되어
  최종 폴더가 생기지 않습니다(IV.6.2 계약 유지). 시작할 때 이전 비정상 종료의 잔여 staging을 정리합니다.
- **snapshot(XIV.2.6) 사용 규칙**: materializer는 이미 snapshot root를 `0700`으로 만듭니다. project 쪽은
  snapshot을 `PrivateTempDir` session root 아래에 만들어, snapshot이 `keep()`되었거나 읽기 전용
  디렉터리가 남아도 session 정리가 전부 지우게 합니다(통합 테스트로 확인).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 266 passed; 통합 document_residency 3, document_round_trip 5, document_serialize 5, private_temp_snapshot 1, restricted_yaml_fuzz 4, working_tree_project 2 passed; 0 failed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **권한**: 생성 디렉터리 mode 정확히 `0700`(group·other 비트 0), `geteuid` 소유, symlink 아님, 표식 `0600`, 이름 고유, drop 삭제·`keep` 보존(`private_dir_is_owner_only_with_marker_and_removed_on_drop`) | 통과 |
| **정리 강화**: `0555` 디렉터리 안 `0444` 파일, `0000` 디렉터리, 외부 `0555` 디렉터리 링크가 있어 `remove_dir_all`이 실패하는 트리를 `cleanup()`이 삭제; 링크 대상 권한·내용 미변경(`cleanup_restores_owner_access_without_following_links`) | 통과 |
| 명시 정리 없이 drop(오류 경로)해도 쓰기 불가 하위 디렉터리까지 삭제, parent 비어 있음(`drop_after_failure_paths_cleans_up`) | 통과 |
| 잘못된 접두사 7종 거부·생성 0, 없는 parent는 `Io(NotFound)`(`invalid_prefixes_are_rejected`) | 통과 |
| **잔여 정리**: 죽은 pid의 표식 있는 `0700` 디렉터리(읽기 전용 하위 포함)만 삭제; 이 process·pid 1은 `live`; 표식 없음·`0755`·symlink·다른 접두사·pid 아닌 이름은 미변경(`sweep_removes_only_stale_marked_private_dirs`) | 통과 |
| **clone**: 대리 실행 파일로 git 실행 시점의 clone 대상 부모를 관찰 → `drwx------` staging, 표식 존재; 완료 후 parent에는 최종 폴더만(`clone_runs_inside_an_owner_only_staging_directory`) | 통과 |
| clone 시작 시 죽은 process의 잔여 staging 정리, 사용자 파일 보존(`clone_sweeps_stale_staging_left_by_a_dead_process`) | 통과 |
| 기존 IV.6.2 검사(성공 rename 후 잔여 0, 실패 시 최종 폴더 부재·parent 비어 있음) | 통과 |
| **snapshot**: 실제 Git index snapshot을 `PrivateTempDir` 아래에 materialize → session·snapshot root `0700`, 같은 소유자; `keep()`한 snapshot에 읽기 전용 디렉터리를 남겨도 session `cleanup()`이 전부 삭제, 작업 트리 보존(`snapshots_live_under_an_owner_only_root_that_cleans_up_everything`) | 통과 |

## 차이(작업 지시와 다른 점)

- materializer(`crates/git-porcelain`) 자체의 drop 정리는 이 lane의 허용 경로 밖이라 바꾸지 않았습니다.
  대신 project 쪽 사용 규칙(비공개 session root 아래에 만들기)과 그 정리로 강화했습니다.
- libc crate 없이 `geteuid`·`kill` 두 함수만 `extern "C"`로 선언했습니다(`tools/stellar-mcp`의
  `ipc::os`와 같은 방식, 새 외부 의존성 없음).
- "다른 사용자 접근 금지"는 실제 다른 계정으로 접근해 보는 대신 권한 비트·소유자 검사로 확인했습니다
  (테스트 환경에 두 번째 사용자 없음).

## 남은 위험 / 한계

- 비-unix(Windows, ADR-0006 보류) 빌드는 권한 검사를 하지 않고 잔여 정리도 하지 않습니다. Windows 재개 시
  ACL 기반 비공개 디렉터리와 그 테스트가 필요합니다.
- 잔여 정리는 pid 재사용이 있으면 남길 수 있습니다(살아 있다고 보수적으로 판정, 삭제 오판은 없음).
- root 권한으로 실행하면 권한 비트로 막히지 않으므로 권한 의존 테스트 일부는 의미가 줄어듭니다.
- 이번 세션의 전체 `cargo test` 병렬 실행 중 이 작업과 무관한 timing 테스트(FSEvents live, 제한 YAML
  fuzz 입력당 시간 상한)가 한 번씩 실패했고 단독·재실행에서는 통과했습니다.

## 후속 수정(리뷰 반영): 잔여 정리가 다른 기계의 staging을 지우지 않음

- **문제**: `clone_project`는 사용자가 고른 `parent_dir`를 clone마다 정리하고, 잔여 여부를 이 기계의
  `kill(pid, 0) == ESRCH`로만 판정했습니다. `parent_dir`가 공유·네트워크 volume이면 다른 기계(같은 uid)의
  살아 있는 Stellar process가 쓰는 staging을 지울 수 있었습니다(이름에 host 식별자가 없음).
- **수정**(`crates/project/src/private_temp.rs`): 표식 파일에 만든 host 이름(`host=<gethostname>\n`)을
  기록하고, `sweep_stale_private_temps`는 표식 내용이 이 host의 것과 정확히 같은 디렉터리만 pid 검사로
  지웁니다. 다른 host·이전 형식(빈 표식)·1 KiB를 넘는 표식은 `SweepReport::other_host`로 보고하고
  남깁니다(남겨도 디렉터리가 남을 뿐 데이터를 잃지 않음). `gethostname`은 기존 `kill`·`geteuid`와 같은
  최소 extern 선언으로 부르고, NUL 종료가 없거나 UTF-8이 아닌 이름은 쓰지 않습니다.
- **검증**: `marker_records_this_host`, `sweep_keeps_dirs_marked_by_another_host_or_legacy_markers`(다른
  host·빈 표식·큰 표식 남김, 이 host 표식만 지움), 기존 `sweep_removes_only_stale_marked_private_dirs`,
  `clone_sweeps_stale_staging_left_by_a_dead_process`(이 host 표식으로 갱신).
- **남은 한계**: macOS host 이름이 네트워크에 따라 바뀌면 이전 이름으로 남은 잔여는 정리하지 않습니다(누수만,
  삭제 오판 없음). 리뷰가 함께 지적한 상태 표시(비-unix에서 이 모듈이 검사·정리를 하지 않으므로 ADR-0006
  규칙 4에 따라 `[~]`이 맞는지)는 이 행의 상태 표식 결정이라 이 수정 범위 밖으로 남깁니다.
