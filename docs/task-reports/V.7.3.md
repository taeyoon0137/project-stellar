# 작업 보고서: V.7.3 grace period sweep

- WBS: V.7.3
- 작업명: unreferenced chunk를 grace period 이후 sweep, 삭제 실패는 다음 실행으로 이월
- 상태: 완료 (`[x]`)
- 선행: V.7.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/gc/sweep.rs`(신규) — `sweep_unreferenced`, `SweepOutcome`.
- `crates/storage/src/gc/mod.rs` — `sweep` 모듈 등록.
- `docs/task-reports/V.7.3.md`(신규), 계획서(V.7.3 `[ ]`→`[x]`).

## 구현 요약

- **대상**: `design/objects/**.starchunk` + `design/generations/*.starmanifest`.
  content-addressed 이름이 아닌 파일은 건드리지 않습니다.
- **판정**: mark 집합(V.7.2) 포함 → 보존. unmark라도 mtime 기준 나이가 grace
  period(`config/storage-gc.yaml`, V.7.1) 미만 → 유예 보존(진행 중 저장 산출물
  보호). 그 외 삭제.
- **이월**: 파일 삭제 실패·mtime 읽기 실패는 오류가 아니라 `carry_over`에 기록만
  하고 다음 실행에서 재시도합니다(idempotent).
- 현재 시각은 인자로 주입해 시간 의존 테스트를 결정론적으로 만듭니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 173 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| active chunk 미삭제 + orphan 삭제(`active_chunks_are_kept_and_orphans_are_deleted`) | 통과 |
| grace 이내 orphan 보존, grace 0 즉시 삭제, 무관 파일 미접촉 | 통과 |
| 삭제 실패 이월 후 다음 실행에서 삭제(unix 권한 fixture) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- atomic write가 남긴 `.tmp-*` 파일은 content-addressed 이름이 아니라 sweep 대상이
  아닙니다. temp 파일 정리는 별도 정책이 필요합니다(후속).
- mtime 기반 유예는 시계 조작에 취약하지만, 삭제 대상이 unreferenced 파일뿐이라
  최악의 경우도 orphan이 더 오래 남는 것입니다.
