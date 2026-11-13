# 작업 보고서: XI.1.5 render thread의 lock 없는 snapshot 읽기

- WBS: XI.1.5
- 작업명: render thread가 `Arc<SceneSnapshot>` handle로 lock 없이 읽게 하고 mutable reference를 노출하지 않음
- 상태: 완료 (`[x]`)
- 선행: XI.1.4 (이 lane에서 완료: `3dfe8d6`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/scene/src/handle.rs`(신규) — `SnapshotHandle`(`Arc<SceneSnapshot>`, `Deref`만 구현),
  `SnapshotLifetime`(약한 참조 회수 관찰), 동시성 테스트, compile_fail doctest.
- `crates/scene/src/lib.rs` — `handle` 모듈 등록.
- `docs/task-reports/XI.1.5.md`(신규), 계획서(XI.1.5 `[ ]`→`[x]`).

## 구현 요약

- **lock 없는 읽기**: snapshot은 생성 후 불변이고 내부 가변성이 없어 `Send + Sync`입니다. handle을 가진
  thread는 `&SceneSnapshot`으로 바로 읽습니다. 읽기 경로에 lock·atomic 쓰기가 없고, 복제·drop만
  참조 카운트를 바꿉니다.
- **가변 참조 미노출**: `SnapshotHandle`은 `Deref`만 구현하고 `DerefMut`·`get_mut`·내부 `Arc` 접근을
  제공하지 않습니다. scene 공개 API에는 `&mut self` 메서드가 없고 갱신은 항상 새 snapshot을
  만듭니다(XI.1.4). `&mut *handle`이 E0596으로 컴파일 실패하는 doctest로 고정했습니다.
- **수명**: 참조 카운트 회수입니다. 오래된 snapshot을 읽는 reader가 있어도 document thread는 기다리지
  않고 다음 세대를 만들며, 마지막 handle drop 시 회수됩니다(`SnapshotLifetime::is_alive`).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | unit 41 passed, doctest 2 passed(compile_fail E0596 포함), 0 failed |
| `cargo test -q -p stellar-scene concurrent_readers` ×5(debug) | 5회 모두 1 passed |
| `cargo test -q --release -p stellar-scene concurrent_readers` | 1 passed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| document thread가 구조 공유 snapshot 200세대를 연속 생성하는 동안 reader 4개가 각 세대를 lock 없이 끝까지 읽음: 모든 읽기에서 semantic hash가 producer 값과 일치, pre-order z-order 불변식 유지, 읽기 800회, 붙잡아 둔 오래된 세대도 producer drop 후 같은 값, 종료 후 200세대 모두 회수(`concurrent_readers_see_consistent_snapshots_without_locks`) | 통과 |
| 붙잡은 handle이 있는 동안 snapshot 유효, 마지막 drop 시 회수(`old_snapshot_stays_valid_while_held`) | 통과 |
| `SnapshotHandle`·`SnapshotLifetime` `Send + Sync + 'static`(`handle_is_shareable_across_threads`) | 통과 |

## 차이(작업 지시와 다른 점)

- ThreadSanitizer는 nightly toolchain이 필요한데 저장소는 Rust 1.96.0 stable로 고정되어 있어 실행하지
  않았습니다. 검증 기준이 허용하는 "동등 동시성 테스트"로 대체했습니다. 동시 참조 카운트 조작과 공유
  node 동시 읽기를 실제 thread로 반복 실행했습니다.
- 테스트의 thread 간 전달은 `std::sync::mpsc`를 썼습니다. 제품 handoff(lock-free 최신값 전달)는
  RenderThread(XII.1.3) 범위입니다.

## 남은 위험 / 한계

- data race 검출 도구(TSan·Miri)로 확인하지 않았습니다. scene crate에 `unsafe`가 없어 safe Rust
  타입 시스템이 data race 부재를 보장하지만, 도구 증거가 필요하면 nightly CI job이 있어야 합니다.
- 매우 깊은 tree의 마지막 handle drop은 재귀 drop입니다(깊이 수만 단계에서 stack 위험).
