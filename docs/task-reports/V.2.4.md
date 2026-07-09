# 작업 보고서: V.2.4 immutable chunk write

- WBS: V.2.4
- 작업명: immutable chunk write — 같은 hash 파일은 내용 검증 후 재사용, 다르면 corruption 오류
- 상태: 완료 (`[x]`)
- 선행: V.2.2, V.2.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/chunk/store.rs`(신규) — `ChunkStore`, `WriteOutcome`,
  `ChunkStoreError`.
- `crates/storage/src/chunk/mod.rs` — `store` 모듈 등록.
- `docs/task-reports/V.2.4.md`(신규), 계획서(V.2.4 `[ ]`→`[x]`).

## 구현 요약

- **`ChunkStore::write_chunk(kind, payload)`**: envelope(V.2.2)+payload를 인코딩하고
  content hash 경로(V.2.3)에 기록합니다.
  - 대상 파일 없음 → 기록, `WriteOutcome::Written`.
  - 대상 파일 있음 + bytes 일치 → 재사용, `WriteOutcome::Reused`(dedup).
  - 대상 파일 있음 + bytes 불일치 → `ChunkStoreError::CorruptExisting`. 덮어쓰기·자동
    수정을 하지 않습니다.
- **파일 형식**: envelope header + payload 원본 bytes. 비교는 header 포함 전체 bytes로
  수행하므로 payload hash가 같아도 header(kind)가 다르면 불변성 위반으로 거부됩니다.
- temp write·fsync·atomic rename은 계획대로 V.2.5에서 플랫폼 abstraction으로 분리합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 53 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| dedup fixture(`same_payload_is_deduplicated`) | 통과 |
| collision/corruption simulation(`corrupted_existing_file_is_not_overwritten`, `different_kind_same_payload_is_collision`) | 통과 |
| 50개 상이 payload 무충돌 기록 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 현 단계 write는 단순 쓰기라 중간 강제 종료 시 부분 파일이 남을 수 있습니다. V.2.5의
  temp write + atomic rename이 이를 제거합니다.
- 존재 검사와 쓰기 사이의 race(동시 writer)는 단일 프로세스 저장 파이프라인(V.5) 하에서
  다룹니다.
