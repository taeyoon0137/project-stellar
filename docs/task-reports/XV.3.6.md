# 작업 보고서: XV.3.6 cache read lock과 fetch/write lock 분리

- WBS: XV.3.6
- 작업명: cache read lock과 fetch/write lock을 분리해 여러 Stellar project가 같은 entry를 동시에 읽게 함
- 상태: 완료 (`[x]`)
- 선행: XV.3.3(`[x]`, 같은 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + OS advisory file lock·자식 프로세스·시스템 Git, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/cache_lock.rs`(신규) — `CacheLocks`(`read_entry`, `try_read_entry`,
  `try_delete_entry`, `fetch`, `try_fetch`, `index`, `try_index`), `CacheLockGuard`, `CacheLockKind`,
  `CacheLockError`, two-process 테스트.
- `crates/dependencies/src/cache_entry.rs` — `OpenedEntry`, `LibraryEntryStore::{open_entry, locks}`,
  index·source config read-modify-write를 index lock 안으로 옮김, `MaterializeError::Lock`.
- `crates/dependencies/src/mirror.rs` — `MirrorCache::ensure_fetched`가 fetch 동안 repository fetch lock 보유.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.3.6.md`(신규), 계획서(XV.3.6 `[ ]`→`[x]`).

## 구현 요약

- **lock 종류 분리**(`<cache root>/locks/`, OS advisory `flock` — std `File::lock*`, Rust 1.89+):
  - entry read(공유, `sha256-<hex>.entry.lock`): entry를 쓰는 프로젝트가 사용하는 동안 보유합니다.
    여러 프로세스가 동시에 보유할 수 있습니다.
  - entry delete(같은 파일, 배타 try): cache 정리(XV.8)용입니다. reader가 하나라도 있으면 `Busy`이고,
    삭제 중에는 새 reader가 기다립니다.
  - fetch(배타, `fetch-<repository key>.lock`): mirror `init`·`fetch` 동안만 보유합니다(XV.3.2).
  - index(배타, `index.lock`): privacy index·source config read-modify-write 동안만 짧게 보유합니다.
- **read는 fetch/write를 기다리지 않음**: `open_entry`는 index를 lock 없이 읽고(쓰기는 원자적
  rename), entry read lock과 짧은 index lock(최근 사용 시각)만 씁니다. 다른 프로세스가 같은
  repository를 fetch하거나 새 entry를 materialize해도 공개된 entry를 바로 읽습니다. 공개된 entry는
  immutable이고(XV.3.3), materialize의 staging은 프로세스마다 고유하며 공개는 `rename` 한 번입니다.
- **갱신 유실 방지**: materialize·touch는 index lock을 잡은 뒤 index를 다시 읽고 갱신합니다
  (XV.3.3 보고서의 동시 갱신 위험 해소).
- **중첩 없음**: fetch lock을 잡은 채 index lock을 기다리는 경로가 없습니다(같은 프로세스의 다른
  fd끼리도 `flock`은 충돌).
- lock 파일은 지우지 않습니다. 지우면 다른 프로세스가 옛 inode를 잠그는 경쟁이 생깁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 65 passed, 0 failed; doctest 1 passed |
| `cargo test -q -p stellar-dependencies cache_lock` ×3 | 매회 2 passed(반복 안정) |
| `cargo test -p stellar-project` | lib 265 passed, 통합 17 passed, 0 failed |
| `cargo clippy -p stellar-dependencies -p stellar-project --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **two-process concurrency fixture**: 실제 Git library를 materialize한 뒤 테스트 바이너리를 자식 프로세스 2개(read 역할)로 다시 실행하면 두 자식이 같은 entry를 `open_entry`로 동시에 열고 bytes를 읽은 채 대기하며, 부모도 공유 read lock을 함께 얻음. reader가 있는 동안 delete lock `Busy`. 세 번째 자식이 같은 repository의 fetch lock을 잡은 동안 부모의 fetch lock 시도는 `Busy`지만 `open_entry`는 5초 안에(대기 없이) 완료, index lock도 즉시 획득. 모두 해제 후 delete lock 획득, 삭제 lock 보유 중 새 reader `Busy`, 자식 3개 모두 정상 종료(`two_process_readers_share_entries_while_fetch_is_held`) | 통과 |
| lock 종류 충돌 표(공유+공유 허용, 공유 vs 배타 `Busy`, fetch는 index·entry와 독립, lock 파일이 cache root 아래)(`lock_files_live_under_cache_root_and_kinds_conflict_as_documented`) | 통과 |
| 변이 확인: entry read lock을 배타로 바꾸면 두 번째 reader가 막혀 fixture가 시간 초과로 실패함을 확인한 뒤 원복 | 확인 |

## 차이(작업 지시와 다른 점)

- "fetch/write lock"은 fetch lock(repository별, 장시간)과 index lock(전역, 단시간)으로 나눴습니다.
  materialize 자체는 staging·원자 rename으로 충돌이 없어 entry 단위 쓰기 lock이 필요하지 않습니다.
- 자식 프로세스 역할은 부모가 `Command::env`로만 전달합니다. 부모 프로세스의 환경 변수는 바꾸지
  않습니다(git lane 리뷰 G4의 `set_var` 경쟁을 피함).
- 테스트는 구현과 같은 변경에서 작성했습니다. 실패 확인은 위 변이 확인으로 대신했습니다.

## 남은 위험 / 한계

- advisory lock이라 이 cache를 쓰지 않는 외부 도구는 막지 못합니다. 네트워크 파일 시스템의 `flock`
  의미는 보장하지 않습니다(app data는 로컬 디스크 전제).
- Windows의 `LockFileEx` 의미 차이는 검증하지 않았습니다(ADR-0006 보류).
- read lock 보유 시간은 호출자가 `OpenedEntry`를 언제 drop하는지에 달려 있습니다. library snapshot
  로드(XV.5)가 이 guard를 보유하도록 연결해야 합니다.
