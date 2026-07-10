# 작업 보고서: V.7.2 reachable mark scanner

- WBS: V.7.2
- 작업명: 보존 manifest에서 reachable content hash를 mark하는 scanner
- 상태: 완료 (`[x]`)
- 선행: V.7.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/gc/mark.rs`(신규) — `mark_reachable`.
- `crates/storage/src/gc/mod.rs` — `mark` 모듈 등록.
- `docs/task-reports/V.7.2.md`(신규), 계획서(V.7.2 `[ ]`→`[x]`).

## 구현 요약

- **`mark_reachable(root, retained_manifests)`**: 보존 대상 manifest들을 읽고
  (hash 일치·구조 검증 포함) mark 집합을 만듭니다 — 보존 manifest 자신의 hash +
  각 manifest가 가리키는 모든 chunk hash의 합집합.
- **과잉 삭제 방지**: 보존 대상 manifest가 읽히지 않으면 오류를 반환합니다 —
  sweep(V.7.3)은 mark가 성공해야만 진행할 수 있습니다.
- chunk는 content-addressed로 세대 간 공유되므로, 어떤 보존 세대라도 참조하면
  mark됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 168 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 공유 chunk가 한 generation 삭제(미보존)로 unmark되지 않음(`shared_chunk_survives_generation_removal`) | 통과 |
| 다중 보존 manifest 합집합, 읽기 실패 시 오류, 빈 집합 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- sweep(V.7.3)이 이 mark 집합의 소비자입니다.
