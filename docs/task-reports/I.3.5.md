# 작업 보고서: I.3.5 ADR 보호 경로 정책

- WBS: I.3.5
- 작업명: `tools/xtask/src/adr_policy.rs` — 보호 경로 변경 시 ADR 강제
- 상태: 완료 (`[x]`)
- 선행: I.3.4 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/adr_policy.rs` (신규)
- `tools/xtask/src/lib.rs` (`pub mod adr_policy` 추가)
- `docs/task-reports/I.3.5.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.3.5 `[ ]` → `[x]`)

## 구현 요약

- `check_adr_policy(changed_paths) -> Vec<AdrError>`.
- 보호 경로: `crates/core-types/`(public API·numeric registry), `crates/storage/`(format), `crates/ffi/`(FFI), `schemas/`, `config/performance-budgets.yaml`(performance budget).
- 보호 경로 변경 + 동반 ADR(`docs/adr/ADR-*.md`, TEMPLATE 제외) 없음 → `missing-adr`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 보호 경로만 변경 | `missing-adr` |
| 보호 경로 + ADR 동반 | 통과 |
| docs-only / TEMPLATE-only | 오탐 0 |
| TEMPLATE는 ADR 요건 미충족 | `missing-adr` 유지 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 80 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 보호 경로 패턴은 아키텍처의 계획 레이아웃 기준으로 정의했습니다(crate가 생성되면 동일 prefix가 적용됨).
