# 작업 보고서: V.3.2 중복 ChunkId pointer validator

- WBS: V.3.2
- 작업명: 동일 ChunkId가 한 generation manifest에서 두 hash를 가리키면 거부하는 validator
- 상태: 완료 (`[x]`)
- 선행: V.3.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/chunk/pointer.rs` — `validate_pointer_set`, `PointerSetError` 추가.
- `docs/task-reports/V.3.2.md`(신규), 계획서(V.3.2 `[ ]`→`[x]`).

## 구현 요약

- **`validate_pointer_set(&[ChunkPointer])`**: 같은 ChunkId가 두 번 나타나면 거부합니다.
  - 서로 다른 hash를 가리키면 `ConflictingChunkId { chunk_id, first, second }`.
  - hash까지 동일한 완전 중복도 `DuplicateChunkId`로 거부(한 manifest에서 ChunkId는
    정확히 한 번).
- 서로 다른 ChunkId가 같은 hash를 공유하는 것(내용 dedup)은 허용합니다.
- generation manifest(V.4.1) 디코딩·검증 경로에서 이 validator를 사용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 79 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| duplicate pointer fixture 실패(`same_chunk_id_with_two_hashes_is_rejected`, `exact_duplicate_pointer_is_rejected`) | 통과 |
| 고유 id 집합·빈 집합 통과, 두 id가 한 hash 공유 허용 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 없음.
