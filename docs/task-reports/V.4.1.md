# 작업 보고서: V.4.1 generation manifest

- WBS: V.4.1
- 작업명: generation manifest(generation number, parent generation, project id, schema version, chunk pointers, root document IDs) 정의
- 상태: 완료 (`[x]`)
- 선행: V.3.2, IV.1.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/generation/mod.rs`(신규) — generation 모듈 선언.
- `crates/storage/src/generation/manifest.rs`(신규) — `GenerationManifest`,
  `ManifestError`, `MANIFEST_MAGIC`, `MANIFEST_FORMAT_VERSION`.
- `crates/storage/src/lib.rs` — `pub mod generation;` 등록.
- `docs/task-reports/V.4.1.md`(신규), 계획서(V.4.1 `[ ]`→`[x]`).

## 구현 요약

- **binary 형식(format version 1)**: magic `STARMFST` + format version + generation
  `u64`(1부터) + parent flag(`bool`) + parent `u64`(있을 때만) + project id 16바이트 +
  schema version `u32` + chunk pointer 목록(`u32` count, chunk_id 오름차순 강제) +
  root document id 목록(`u32` count, 순서 의미 있음).
- **불변식 검증**: generation ≥ 1, parent < generation, pointer 집합 V.3.2 validator
  통과, root document id 중복 거부. encode·decode 양쪽에서 검증합니다.
- **결정론**: encode는 pointer를 chunk_id 오름차순으로 정렬하고, decode는 정렬 위반
  bytes를 `MisorderedPointers`로 거부합니다. 재인코딩은 byte-identical합니다.
- project id는 `StableId`를 그대로 사용합니다(crate 그래프상 storage는 project에
  의존할 수 없어 `ProjectIdentity` 대신 원시 id를 담습니다).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 102 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| golden manifest round-trip + 재인코딩 byte-identical(`golden_round_trip`) | 통과 |
| 필드 배치 사양 대조(`golden_header_layout_is_fixed`) | 통과 |
| generation 0·parent 역전·중복 pointer·중복 root doc·정렬 위반·절단·magic/version 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- manifest 자체의 content hash 계산(V.4.2)과 current pointer 기록(V.4.3)은 후속
  작업입니다.
