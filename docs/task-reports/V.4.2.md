# 작업 보고서: V.4.2 manifest canonical encode + content hash

- WBS: V.4.2
- 작업명: manifest 자체를 canonical encode하고 content hash 계산
- 상태: 완료 (`[x]`)
- 선행: V.4.1, V.1.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/generation/manifest.rs` — `encode_with_hash` 추가, 결정론·hash
  테스트 추가.
- `docs/task-reports/V.4.2.md`(신규), 계획서(V.4.2 `[ ]`→`[x]`).

## 구현 요약

- **`encode_with_hash()`**: V.4.1의 canonical encode 결과 bytes 전체에 대해 ADR-0004
  content hash(SHA-256)를 계산해 `(bytes, hash)`를 반환합니다. 이 hash가 current
  pointer(V.4.3)가 가리키는 값이 됩니다.
- pointer 입력 순서가 달라도 encode가 정렬하므로 동일 모델은 항상 동일 bytes →
  동일 hash입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 104 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 동일 모델(입력 순서 상이 포함) 100회 인코딩 byte-identical + 동일 hash(`identical_models_hash_identically`) | 통과 |
| 다른 모델은 다른 hash(`different_models_hash_differently`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- manifest 파일의 저장 경로 규칙과 current pointer 기록은 V.4.3에서 확정합니다.
