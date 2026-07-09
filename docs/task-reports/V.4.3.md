# 작업 보고서: V.4.3 current pointer 파일

- WBS: V.4.3
- 작업명: current pointer 파일에 manifest hash 하나만 기록, temp write 후 atomic replace 전환
- 상태: 완료 (`[x]`)
- 선행: V.4.2, V.2.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/generation/current.rs`(신규) — `write_current_pointer`,
  `read_current_pointer`, `current_pointer_path`, `CurrentPointerError`.
- `crates/storage/src/generation/mod.rs` — `current` 모듈 등록.
- `docs/task-reports/V.4.3.md`(신규), 계획서(V.4.3 `[ ]`→`[x]`).

## 구현 요약

- **파일 형식**: `design/current`에 manifest content hash 문자열(ADR-0004) + LF —
  총 72바이트, hash 하나만. 여러 줄·LF 누락·hash 형식 위반은 읽기에서 거부합니다.
- **전환**: V.2.5 atomic write abstraction(temp write → flush → fsync → atomic
  replace)만 사용합니다. rename이 유일한 대상 변경 단계이므로 어떤 시점에 읽어도
  이전 또는 새 hash 중 하나만 보입니다.
- fault injection용 `write_current_pointer_observed`를 제공합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 108 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 각 write 단계 중단에서 이전 pointer 보존, partial 미노출(`replace_is_atomic_under_fault_injection`) | 통과 |
| round-trip(72바이트 고정), missing 보고, malformed 내용 4종 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- current pointer 파일 자체는 Git이 추적하는 텍스트이므로 merge 충돌이 가능합니다.
  저장 충돌 처리는 optimistic check(V.5.2)와 Git 계층에서 다룹니다.
