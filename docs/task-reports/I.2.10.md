# 작업 보고서: I.2.10 계획서 WBS 추출

- WBS: I.2.10
- 작업명: `plan/markdown_wbs.rs` — 계획서 표에서 WBS ID·title·status·선행 추출
- 상태: 완료 (`[x]`)
- 선행: I.2.2 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/plan/markdown_wbs.rs` (신규)
- `tools/xtask/src/plan/mod.rs` (신규)
- `tools/xtask/src/lib.rs` (`pub mod plan` 추가)
- `docs/task-reports/I.2.10.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.2.10 `[ ]` → `[x]`)

## 구현 요약

- `extract_wbs(&str) -> Vec<PlanWbsEntry>`. col1이 valid WbsId이고 col2가 정확한 상태 marker(`[x]`/`[~]`/`[ ]`/`[!]`)인 표 행만 추출.
- fenced code block 무시, prose 예시(파이프로 시작 안 함) 제외, 요약/Gate/환경 표(WbsId 아님) 제외.
- title은 구현 지시 열(col3). depends_on은 선행 열에서 exact WbsId만(없음·range·Gate 무시).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 표 추출 + fence/prose 무시 | 통과 |
| 요약/Gate/환경 행 무시 | 추출 0 |
| 다중 선행 추출 + range 제외 | 통과 |
| 실제 계획서: 중복 없는 ID, I.1.1·I.2.10 포함, count>30 | 통과 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 46 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 표에 별도 title 열이 없어, "title"은 `구현 지시`(col3)로 매핑했습니다(표에서 가장 가까운 작업 설명). I.4.3 sync-plan 비교 시 동일 기준을 사용합니다.
