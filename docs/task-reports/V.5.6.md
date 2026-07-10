# 작업 보고서: V.5.6 save transaction fault matrix

- WBS: V.5.6
- 작업명: chunk write, fsync, manifest write, pointer replace 각 지점 실패 자동 주입 matrix
- 상태: 완료 (`[x]`)
- 선행: V.5.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/save/fault_matrix.rs`(신규, 테스트 전용) — 전 지점 자동 주입
  matrix 테스트.
- `crates/storage/src/save/executor.rs` — `SaveObserver::before_file_stage`(파일 내부
  단계 주입) + `FileStageAdapter` 배관.
- `crates/storage/src/chunk/store.rs` — `write_chunk_observed` 추가.
- `crates/storage/src/generation/loader.rs` — `write_manifest_file_observed` 추가.
- `crates/storage/src/save/cancel.rs` — `CancelCheck`가 파일 단계 주입을 내부
  관찰자로 전달.
- `crates/storage/src/save/mod.rs` — `#[cfg(test)] mod fault_matrix;` 등록.
- `docs/task-reports/V.5.6.md`(신규), 계획서(V.5.6 `[ ]`→`[x]`).

## 구현 요약

- **matrix 지점(총 22)**: prepare 단계 직전 4곳(base check·chunk write·chunk verify·
  manifest write) + chunk write 내부 atomic write 단계 6곳(temp 생성·기록·flush·
  **fsync**·rename·dir sync) + manifest write 내부 6곳 + **pointer replace** 내부 6곳.
- **검증 불변식**(각 fixture): ① 실패한 저장은 in-memory saved generation을 바꾸지
  않음 ② 프로젝트는 recovery 없이 열리고 열린 세대는 base(1) 또는 새 세대(2) 중
  정확히 하나 ③ pointer rename **이후** 실패(dir sync)만 새 세대로 열림 — 나머지
  21개 지점은 전부 base 유지.
- 주입 없는 대조군: 새 세대(2)가 열리고 메모리도 갱신됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 144 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 22개 주입 지점 전부에서 불변식 3종 유지(`every_fault_point_leaves_exactly_one_openable_generation`) | 통과 |
| 대조군(무주입) 저장 성공(`fault_free_save_opens_new_generation`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- pointer rename 성공 후 dir sync 실패 경로에서는 디스크는 새 세대, 메모리는 base로
  갈립니다(저장은 실패로 보고). 다음 저장 시 optimistic check(V.5.2)가 stale을
  탐지해 재동기화를 강제하므로 데이터 불일치는 없지만, 사용자에게는 "저장 실패 후
  재열기 시 저장돼 있음"으로 보일 수 있습니다.
