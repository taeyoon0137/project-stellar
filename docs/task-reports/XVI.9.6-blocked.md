# 차단 보고서: XVI.9.6 업데이트 전 open project 상태 저장·복구

## WBS

- WBS: XVI.9.6
- 작업명: 업데이트 전 open project 상태를 저장·복구하고 강제 종료하지 않습니다.
- 상태: `[!]` — 선행 조건 차단(계획 §1.1 정의). 선행이 `[x]`가 아니어서 착수하지 않았습니다.
- 선행: XIII.1.6 (`[ ]`), XVI.9.5 (`[~]`) — 모두 엄격 선행

## 실패 선행조건

- **XIII.1.6 `[ ]`(미착수)**: 창 닫기·앱 비활성화 flush의 timeout과 실패 시 창 닫기 결정 상태를 shell에 돌려주는
  경로가 없습니다. 업데이트 설치 전 "열린 프로젝트를 저장하고, 실패하면 설치를 미룬다"는 판단은 이 결과에 의존합니다.
- **XVI.9.5 `[~]`**: 엄격 선행 XVI.9.2가 `[~]`라서 `[x]`가 아닙니다. 업데이트 경로 자체(Sparkle, ADR-0017)는 구현되어
  있습니다. 다만 공개 키가 아직 사용자 단계로 남아 있어 배포본은 업데이트를 시작하지 않습니다.
- 계획 §1.2 규칙 4에 따라 코드는 수정하지 않았습니다.

## 재현 명령

- `grep -n "| XIII.1.6 \|| XVI.9.5 \|| XVI.9.2 " docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`: 선행 상태
  `[ ]`·`[~]`·`[~]` 확인.
- 선행이 풀린 뒤 검증 후보:
  - `scripts/macos/smoke.sh`에 dirty project update prompt fixture를 추가합니다.
  - Sparkle `-updater:shouldPostponeRelaunchForUpdate:untilInvokingBlock:`·`-updaterShouldRelaunchApplication:` hook과
    XIII.1.6의 flush 결과를 연결해 확인합니다.

## 필요한 결정과 환경

- XIII.1.6 구현(문서 저장 flush timeout·실패 결과)이 먼저 필요합니다.
- XVI.9.2의 깨끗한 macOS 26 장비(또는 VM) 첫 실행 smoke가 필요합니다. 그래야 XVI.9.5가 `[x]`가 될 수 있습니다.
- 사용자 결정이 필요합니다: 저장에 실패한 dirty project가 있을 때 업데이트를 "다음 종료 시 설치"로 미룰지, 사용자에게
  다시 물을지(아키텍처 §40.4 "실행 중 프로젝트를 강제 종료하지 않음"의 UI 정책).
- 필수 실행 환경: `DEV-MAC`.

## 변경되지 않은 파일

- `apps/macos/StellarMac/*.swift`: XVI.9.6 hook을 추가하지 않았습니다.
- `scripts/macos/smoke.sh`: dirty project fixture를 추가하지 않았습니다.
- 계획서 XVI.9.6 행: 상태 표기만 `[ ]`→`[!]`로 바꿨습니다(선행 조건 차단).

## 재개 조건

- XIII.1.6과 XVI.9.5가 모두 `[x]`가 되어야 합니다. XVI.9.5는 XVI.9.2의 깨끗한 장비 실행 증거가 필요합니다.
- 그다음 업데이트 설치·재실행 직전에 열린 프로젝트를 저장(flush)합니다.
  - 실패하거나 timeout이면 설치를 미루고 사용자에게 알립니다.
  - 재실행 후 프로젝트를 복구합니다.
- 위 동작을 dirty project update prompt fixture로 확인하면 `[x]`로 올릴 수 있습니다.
