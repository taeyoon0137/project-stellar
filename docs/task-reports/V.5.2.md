# 작업 보고서: V.5.2 optimistic base check

- WBS: V.5.2
- 작업명: 저장 전 current pointer와 base generation 비교 — 다르면 write 미시작
- 상태: 완료 (`[x]`)
- 선행: V.5.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/save/optimistic.rs`(신규) — `check_base_generation`,
  `BaseCheckError`.
- `crates/storage/src/save/mod.rs` — `optimistic` 모듈 등록.
- `docs/task-reports/V.5.2.md`(신규), 계획서(V.5.2 `[ ]`→`[x]`).

## 구현 요약

- **`check_base_generation(root, plan)`**: current pointer를 읽어 계획의
  `base_manifest_hash`와 비교합니다.
  - base 있음: current가 정확히 그 hash여야 통과.
  - 첫 저장(base 없음): current pointer 파일이 없어야 통과.
  - 불일치는 `StaleBase { expected, found }` — 호출자는 write를 시작하면 안 됩니다.
  - current pointer 형식 위반·I/O는 별도 오류.
- 실행기(V.5.3)가 이 검사를 첫 단계로 수행합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 131 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| stale base fixture에서 검사 실패 + chunk 파일 0개 유지(`stale_base_fails_and_no_chunk_files_are_created`) | 통과 |
| 일치 base 통과, 첫 저장 규칙, 손상 current pointer 보고 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 검사와 write 시작 사이의 race는 남습니다(TOCTOU). 단일 프로세스 저장 파이프라인
  가정 하에서는 실행기(V.5.3~V.5.4)가 순차 실행으로 보장하고, 다중 프로세스 동시
  저장은 Git 계층·잠금 정책의 후속 주제입니다.
