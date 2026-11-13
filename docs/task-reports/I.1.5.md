# 작업 보고서: I.1.5 소스 인벤토리 기록

- WBS: I.1.5
- 작업명: 현재 파일 목록·확장자별 파일 수 기록(자동 생성물·vendored code 별도 집계), 결정론적 생성
- 상태: 완료 (`[x]`)
- 선행: I.1.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`. (보고서 형식 보완: 계획서 작업 행 `[DEV-MAC]`·ledger `required_environments` 기준)

## 변경 파일 목록

- `scripts/source_inventory.sh` (신규, 결정론적 인벤토리 생성 스크립트)
- `docs/status/source-inventory.md` (신규, 생성 결과물)
- `docs/task-reports/I.1.5.md` (신규, 본 보고서)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.1.5 `[ ]` → `[x]`)

## 설계 결정

- 입력은 `git ls-files`(추적 파일 집합)로 한정. 커밋 해시·시각·절대 경로를 출력에 포함하지 않음 → 동일 파일 집합에서 재실행 시 결과 동일.
- `.vscode/` 등 untracked 파일은 집계에서 제외(추적 집합만 대상).
- 자동 생성물은 AGENTS.md의 source-of-truth 정의에 따라 `README.md`, `resources/readme-hero.svg`로 식별.
- vendored code는 `vendor/`, `third_party/`, `third-party/`, `vendored/` 경로 패턴으로 식별(현재 0개).
- 정렬은 `LC_ALL=C sort`로 고정해 로케일 비의존.

## 실행한 명령과 결정론 검증

| 명령 | 결과 |
|---|---|
| `./scripts/source_inventory.sh` (동일 상태에서 2회 연속 실행) | 두 결과가 바이트 단위로 동일(diff 없음) |

- `./scripts/source_inventory.sh` (스테이징 후 재생성으로 신규 파일 반영)
- 결정론 검증: 동일 상태에서 두 번 연속 실행 후 결과 비교 → 차이 없음(아래 "테스트 결과").

## 테스트 결과

- 두 번 연속 실행 결과가 바이트 단위로 동일(diff 없음). 완료 기준("동일 커밋에서 두 번 실행 시 동일 보고서") 충족.

## 남은 경고 수

- 0

## 성능 수치

- 해당 없음

## 작업 지시와 다른 점

- "현재 파일 목록"은 결정론을 위해 추적 파일(`git ls-files`)로 한정했습니다. untracked 파일은 비결정 요인이므로 제외했습니다.
