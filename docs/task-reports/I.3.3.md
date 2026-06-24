# 작업 보고서: I.3.3 라이선스 정책

- WBS: I.3.3
- 작업명: dependency validator에 license policy 추가
- 상태: 완료 (`[x]`)
- 선행: I.3.2 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/dependencies.rs` (license policy + `separate_approval` 필드 추가)
- `docs/task-reports/I.3.3.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.3.3 `[ ]` → `[x]`)

## 구현 요약

- 자동 허용: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC.
- 자동 거부 prefix: GPL, AGPL, SSPL → `rejected-license`.
- 그 밖: `separate_approval: true` 없으면 `license-needs-approval`.
- SPDX 복합식(`OR`/`AND`/`WITH`, `/`, 괄호) 토큰 분해 후 평가.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| MIT / MIT OR Apache-2.0 | 허용 |
| GPL-3.0 / AGPL-3.0-only | `rejected-license` |
| Custom-1.0 (flag 없음/있음) | `license-needs-approval` / 허용 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 76 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 없음.
