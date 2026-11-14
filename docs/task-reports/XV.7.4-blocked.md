# 차단 보고서: XV.7.4 승인 후 lockfile·in-memory library snapshot 원자 갱신

## WBS

- WBS: XV.7.4
- 작업명: 사용자 승인 후 lockfile과 in-memory library snapshot을 원자적으로 갱신, 사용 project override는 StableId로 재적용
- 상태: `[!]`
- 선행: XV.7.3(`[!]`), XV.4.5(`[x]`)

## 실패 선행조건

- 엄격 선행 XV.7.3이 차단 상태입니다(IX.4.6 `[ ]`, `docs/task-reports/XV.7.3-blocked.md`).

## 재현 명령

- `cat docs/task-reports/XV.7.3-blocked.md`

## 필요한 결정과 환경

- XV.7.3 완료가 필요합니다.
- 필수 환경: `DEV-MAC`.

## 변경되지 않은 파일

- 기능 코드는 변경하지 않았습니다(계획 §1.2 규칙 4). 이 보고서와 계획서 상태 표기만 바꿨습니다.

## 재개 조건

- XV.7.3 `[x]` 후 착수합니다. lock 기록은 XV.4.5 `lockfile_write` transaction, 새 snapshot 집합은 XV.5.5
  `load_locked_libraries`, 후보는 XV.7.1 `prepare_library_update`를 씁니다.
