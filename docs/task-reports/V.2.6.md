# 작업 보고서: V.2.6 chunk read 전체 검증

- WBS: V.2.6
- 작업명: chunk read 시 envelope, length, hash 전부 검증 — 실패 bytes 자동 수정 없음
- 상태: 완료 (`[x]`)
- 선행: V.2.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/chunk/store.rs` — `ChunkStore::read_chunk` 추가, 오류 variant 5종
  추가(`MissingChunk`, `KindMismatch`, `LengthMismatch`, `AddressMismatch`,
  `PayloadHashMismatch`).
- `docs/task-reports/V.2.6.md`(신규), 계획서(V.2.6 `[ ]`→`[x]`).

## 구현 요약

- **`read_chunk(hash, expected_kind)`** 검증 순서: envelope(magic·format version·hash
  형식) → kind가 기대 kind와 일치 → payload 길이가 envelope의 uncompressed length와
  일치 → envelope hash가 요청 주소와 일치 → payload 재해시가 envelope hash와 일치.
  어떤 실패도 파일 bytes를 수정하지 않습니다.
- **`expected_kind`를 요구하는 이유**: payload hash는 header를 커버하지 않으므로 header
  kind 필드의 bit corruption은 hash 검증으로 탐지되지 않습니다. chunk pointer(V.3.1)가
  아는 기대 kind와의 비교가 이 간극을 메워 파일 전 영역의 single-bit corruption을
  탐지합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 64 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 파일 전 byte·전 bit(8 × 127바이트) single-bit corruption 전수 탐지(`every_single_bit_corruption_is_detected`) | 통과 |
| 절단 파일 탐지, missing chunk, 잘못된 기대 kind, round-trip | 통과 |

## 차이(작업 지시와 다른 점)

- read API가 기대 kind를 인자로 요구합니다. header kind 필드는 hash로 보호되지 않아,
  호출자(chunk pointer)가 아는 kind와의 비교 없이는 single-bit corruption 전수 탐지가
  불가능하기 때문입니다.

## 남은 위험 / 한계

- header 필드는 hash가 아니라 구조 검증과 kind 비교로 보호됩니다. header 자체 checksum
  도입은 format version 상승이 필요한 변경으로, 필요성이 확인되면 새 ADR로 다룹니다.
