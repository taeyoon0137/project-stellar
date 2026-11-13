# 차단 보고서: XV.6.5 local override는 개발 세션 전용, release validation에서 경고 또는 정책 오류를 config로 제어

## WBS

- WBS: XV.6.5
- 작업명: local override는 개발 세션 전용, release validation에서 경고 또는 정책 오류를 config로 제어
- 상태: `[!]`
- 선행: XV.6.3(`[!]`)

## 실패 선행조건

- 엄격 선행 XV.6.3이 차단 상태입니다(XV.6.3 차단 보고서 참고: XV.6.2 `[~]`).
- 이 lane(deps_wiring)의 허용 경로는 `crates/dependencies/**`와 project crate의 manifest·lockfile·YAML writer 파일뿐이라 probe·adapter 파일을 만들거나 고칠 수 없습니다.

## 재현 명령

- `cat docs/task-reports/XV.6.3-blocked.md`

## 필요한 결정과 환경

- XV.6.2 → XV.6.3 순서로 완료되어야 합니다.
- 필수 환경: `DEV-MAC`.

## 변경되지 않은 파일

- 기능 코드는 변경하지 않았습니다(계획 §1.2 규칙 4). 이 보고서와 계획서 상태 표기만 바꿨습니다.

## 재개 조건

- XV.6.3 `[x]` 후 착수합니다.
