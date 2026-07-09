# 작업 보고서: V.2.5 atomic 파일 쓰기 abstraction

- WBS: V.2.5
- 작업명: temp file write, flush, fsync 정책, atomic rename 플랫폼 abstraction
- 상태: 완료 (`[x]`)
- 선행: V.2.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증. Windows 분기는 cfg 컴파일만 — CI-WIN 증거는 후속).

## 변경 파일 목록

- `crates/storage/src/atomic_file.rs`(신규) — `write_file_atomic`,
  `write_file_atomic_observed`, `WriteStage`, `WriteObserver`.
- `crates/storage/src/chunk/store.rs` — chunk 기록을 atomic write로 교체.
- `crates/storage/src/lib.rs` — `pub mod atomic_file;` 등록.
- `docs/task-reports/V.2.5.md`(신규), 계획서(V.2.5 `[ ]`→`[x]`).

## 구현 요약

- **고정 단계 순서**: 같은 디렉터리 temp 생성 → bytes 기록 → flush → `sync_all`
  (macOS는 std가 `F_FULLFSYNC`, Windows는 FlushFileBuffers) → atomic rename(std가
  Windows에서 `MOVEFILE_REPLACE_EXISTING`) → 부모 디렉터리 sync(POSIX 전용, Windows
  no-op — cfg 분기).
- **불변식**: rename이 대상 경로를 바꾸는 유일한 단계이므로, rename 이전 어느 단계에서
  중단해도 대상은 이전 상태(없음 또는 이전 내용)를 유지합니다.
- **fault injection**: `WriteObserver`가 각 단계 직전에 호출되어 `Err`로 강제 중단을
  모사합니다. 프로덕션 경로는 `NoopObserver`.
- **적용**: `ChunkStore::write_chunk`(V.2.4)의 기록 경로를 atomic write로 교체했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 59 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 각 단계 중단 시 새 대상 미노출(`fault_before_each_stage_never_exposes_partial_file_fresh_target`) | 통과 |
| 각 단계 중단 시 기존 내용 보존(`fault_before_each_stage_keeps_previous_content`) | 통과 |
| rename 이후 중단은 새 내용 유지, 우아한 실패 시 temp 정리 | 통과 |

## 차이(작업 지시와 다른 점)

- 프로세스를 실제로 kill하는 대신 단계 주입 관찰자로 중단을 모사했습니다. 대상 경로
  불변식은 rename의 원자성에서 나오므로 동일한 보증을 검증합니다.

## 남은 위험 / 한계

- Windows 분기(디렉터리 sync no-op, rename 교체 의미)는 컴파일만 검증했습니다. CI-WIN
  증거는 Windows CI(II.7.2) 이후 확보됩니다.
- 강제 종료 시 temp 파일이 남을 수 있습니다. orphan temp 정리는 GC(V.7)에서 다룹니다.
