# 작업 보고서: I.2.2 WbsId parser

- WBS: I.2.2
- 작업명: `WbsId` parser·Display·Ord (Roman Phase + 두 십진 하위 번호)
- 상태: 완료 (`[x]`)
- 선행: I.2.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/task_status/wbs_id.rs` (신규)
- `tools/xtask/src/task_status/mod.rs` (모듈 추가)
- `docs/task-reports/I.2.2.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.2.2 `[ ]` → `[x]`)

## 구현 요약

- 형식 `<Roman>.<group>.<item>`. Phase는 정규 로마 숫자(재인코딩 일치 검사), group·item은 선행 0 없는 1 이상 십진수.
- `parse`/`Display`/`Ord`(phase→group→item). `Display`는 parse와 round-trip.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 정상: `I.1.1`, `XXVIII.3.3` | 통과 |
| 거부: `1.1.1`, `I.0.1`, `I.1`, `I.1.1.1`, `I.01.1`, `I.1.1a`, `IIII.1.1`, 빈 문자열 | 전부 None |
| 정렬: `I.1.1 < I.1.2 < I.2.1 < II.1.1` | 통과 |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 8 passed |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 비정규 로마 숫자(`IIII` 등)도 거부하도록 재인코딩 일치 검사를 추가했습니다(형식 엄격성 강화).
