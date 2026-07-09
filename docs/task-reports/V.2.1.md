# 작업 보고서: V.2.1 ChunkId newtype

- WBS: V.2.1
- 작업명: `ChunkId`를 StableId 기반 불변 논리 ID로 정의(타입 혼용 방지 newtype)
- 상태: 완료 (`[x]`)
- 선행: III.1.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/chunk/mod.rs`(신규) — chunk 모듈 선언, `ChunkId` re-export.
- `crates/storage/src/chunk/id.rs`(신규) — `ChunkId` newtype.
- `crates/storage/src/lib.rs` — `pub mod chunk;` 등록.
- `docs/task-reports/V.2.1.md`(신규), 계획서(V.2.1 `[ ]`→`[x]`).

## 구현 요약

- **`ChunkId(StableId)` newtype**: 내용 변경(content hash 변경)에도 유지되는 불변 논리
  식별자입니다. `from_stable_id`/`stable_id`, 16바이트 big-endian `to_bytes`/`from_bytes`,
  표준 UUID 문자열 `Display`/`FromStr`(파싱 오류는 core의 `ParseStableIdError` 재사용)을
  제공합니다. `Ord`는 StableId 바이트 순서(UUIDv7 → 시간 순)를 따릅니다.
- **compile-fail 혼용 테스트**: doctest 2건 — `ChunkId` 자리에는 원시 `StableId`를,
  원시 `StableId` 자리에는 `ChunkId`를 넘길 수 없음을 컴파일 실패로 고정했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 38 passed + doctest 2 passed(compile-fail), 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| compile-fail 혼용 doctest 2건 | 통과 |
| binary·string round-trip, byte 순서 정렬 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- Node 쪽 StableId newtype은 Phase VI에서 정의됩니다. 현재 compile-fail 테스트는 원시
  `StableId`와의 혼용을 고정하며, Node newtype 등장 시 상호 혼용 테스트를 추가할 수
  있습니다.
