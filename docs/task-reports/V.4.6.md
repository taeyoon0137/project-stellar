# 작업 보고서: V.4.6 읽기 전용 recovery open

- WBS: V.4.6
- 작업명: 정상 세대 fallback 시 읽기 전용 recovery result 반환, 자동 current pointer 변경 금지
- 상태: 완료 (`[x]`)
- 선행: V.4.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/generation/open.rs`(신규) — `open_project_storage`,
  `OpenOutcome`, `OpenError`.
- `crates/storage/src/generation/mod.rs` — `open` 모듈 등록.
- `docs/task-reports/V.4.6.md`(신규), 계획서(V.4.6 `[ ]`→`[x]`).

## 구현 요약

- **`open_project_storage`**: current 로드(V.4.4) 성공 → `OpenOutcome::Current`.
  실패 → fallback scan(V.4.5) → 정상 세대 발견 시
  `OpenOutcome::ReadOnlyRecovery { fallback, current_error }`. 정상 세대가 없으면
  `OpenError::NoValidGeneration`.
- **불변식**: open은 어떤 경로에서도 디스크에 쓰지 않습니다. current pointer는
  손상된 세대를 계속 가리킵니다 — 복구 확정(새 세대 저장·pointer 전환)은 사용자
  결정과 저장 트랜잭션(V.5) 소관입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 123 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| open 전후 전체 디스크 bytes 스냅샷 동일(정상·recovery·실패 3경로 모두) | 통과 |
| recovery 시 current pointer가 손상 세대를 그대로 가리킴 | 통과 |
| fallback 세대·원인 오류가 결과에 포함 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- ReadOnlyRecovery 상태에서의 UI 노출·복구 확정 흐름은 셸 계층(Phase IX 이후)
  소관입니다.
