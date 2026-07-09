# 작업 보고서: V.4.5 정상 세대 fallback scanner

- WBS: V.4.5
- 작업명: current 손상 시 보존 manifest 목록에서 최신 정상 세대를 찾는 scanner
- 상태: 완료 (`[x]`)
- 선행: V.4.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/generation/scanner.rs`(신규) — `scan_latest_valid_generation`,
  `ScanError`.
- `crates/storage/src/generation/loader.rs` — 검증 단계를 `verify_generation`으로
  분리(loader·scanner 공유).
- `crates/storage/src/generation/mod.rs` — `scanner` 모듈 등록.
- `docs/task-reports/V.4.5.md`(신규), 계획서(V.4.5 `[ ]`→`[x]`).

## 구현 요약

- **후보 수집**: `design/generations/`의 manifest 파일 이름에서 hash를 복원합니다.
  형식이 아닌 파일은 무시합니다. 후보는 hash 사전식으로 정렬해 결정론적으로
  순회합니다.
- **검증**: 각 후보를 V.4.4와 동일한 기준(manifest hash 일치·구조·project id·chunk
  존재)으로 완전 검증하고, 실패한 세대는 건너뜁니다.
- **선택**: 검증 통과 세대 중 generation number 최대를 반환합니다. 정상 세대가
  없으면 `None`. scanner는 읽기 전용이며 current pointer를 바꾸지 않습니다(V.4.6).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 120 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 최신 2세대 손상 fixture에서 세 번째 정상 세대 선택(`two_newest_corrupted_falls_back_to_third`) | 통과 |
| 전부 정상이면 최신 선택, 누락 chunk 세대 제외, 다른 project 세대 제외, 무관 파일 무시 | 통과 |

## 차이(작업 지시와 다른 점)

- parent chain을 hash로 따라가는 대신 보존 manifest 전수 검증 + generation number
  최대 선택으로 구현했습니다. manifest의 parent 참조는 number뿐이라(hash 아님) 손상
  세대에서 chain을 복원할 수 없고, 전수 검증이 같은 결과를 더 견고하게 보장합니다.

## 남은 위험 / 한계

- fallback 결과를 읽기 전용 recovery로 노출하고 current pointer를 유지하는 정책은
  V.4.6에서 구현합니다.
