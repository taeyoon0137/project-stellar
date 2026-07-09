# 작업 보고서: V.3.1 ChunkPointer

- WBS: V.3.1
- 작업명: `ChunkPointer { chunk_id, content_hash, kind, schema_version }` 정의
- 상태: 완료 (`[x]`)
- 선행: V.2.1, V.1.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/chunk/pointer.rs`(신규) — `ChunkPointer`, `PointerCodecError`.
- `crates/storage/src/canonical/encode.rs`, `crates/storage/src/chunk/envelope.rs` —
  `put_raw_magic`를 `put_fixed_raw`로 개명(고정 폭 raw 필드 공용 API).
- `crates/storage/src/chunk/mod.rs` — `pointer` 모듈 등록.
- `docs/task-reports/V.3.1.md`(신규), 계획서(V.3.1 `[ ]`→`[x]`).

## 구현 요약

- **`ChunkPointer`**: 논리 chunk가 가리키는 물리 payload(content hash)와 해석 정보
  (kind, schema version)를 담는 `Copy` 값 타입입니다.
- **binary 형식**: chunk_id 16바이트(big-endian raw) + content hash(ADR-0004 문자열,
  길이 프리픽스 UTF-8) + kind `u32` LE + schema version `u32` LE. `encode_into`로
  상위 인코딩(manifest)에 이어 쓸 수 있습니다.
- **정렬**: `Ord`는 (chunk_id, content_hash, kind, schema_version) 사전식 —
  chunk_id(UUIDv7)가 최상위라 manifest 정렬이 결정론적입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 75 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| binary round-trip(단독·연속 5개) | 통과 |
| 정렬(chunk_id 우선, UUIDv7 시간 순) | 통과 |
| hash 문자열 손상·절단 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 동일 ChunkId 중복(두 hash) 검증은 계획대로 V.3.2 validator에서 다룹니다.
