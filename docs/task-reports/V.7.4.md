# 작업 보고서: V.7.4 GC scheduler interface

- WBS: V.7.4
- 작업명: GC를 idle+저장 없음 조건에서만 시작하는 scheduler interface
- 상태: 완료 (`[x]`)
- 선행: V.7.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/gc/scheduler.rs`(신규) — `GcScheduler`, `SaveGuard`,
  `GcGuard`.
- `crates/storage/src/gc/mod.rs` — `scheduler` 모듈 등록.
- `docs/task-reports/V.7.4.md`(신규), 계획서(V.7.4 `[ ]`→`[x]`).

## 구현 요약

- **`GcScheduler`**: Mutex 상태(활성 저장 수, GC 실행 중, 마지막 활동 시각)로 GC
  시작 조건을 판정합니다. 시각은 단조 `Instant` 인자로 주입해 테스트가
  결정론적입니다.
- **조건**: `try_begin_gc`는 ① 활성 저장 guard 0 ② GC 미실행 ③ 마지막 활동 이후
  idle threshold 경과 — 셋 다 만족할 때만 `GcGuard`를 반환합니다. 생성 시각이
  곧 최초 활동이므로 **open 직후에는 시작할 수 없습니다**.
- **RAII**: 저장 파이프라인은 `begin_save` guard를 잡고, drop이 종료 표시입니다.
  guard가 살아 있는 한 GC는 시작되지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 178 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| save 중 GC 미시작 동시성 테스트(저장 스레드 4개 × 200회 vs GC 시도 2,000회, 위반 0)(`concurrent_saves_block_gc`) | 통과 |
| open 직후 시작 금지, 활동 후 idle 리셋, GC 단일 실행 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- scheduler는 판정 인터페이스이며, 실제 idle 감지(입력 이벤트)와 GC 실행 루프
  연결은 셸/런타임 계층에서 수행합니다.
