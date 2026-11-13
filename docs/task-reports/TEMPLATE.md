# 작업 보고서: <WBS-ID> <제목>

> 절 제목은 이 template의 8개 제목을 씁니다. 기존 보고서 관례(예: `VI.9.5.md`)의 별칭 제목
> `필수 실행 환경`, `변경 파일 목록`, `실행한 명령 / 검증`, `차이(작업 지시와 다른 점)`,
> `남은 위험 / 한계`도 같은 절로 인정하고, `실행한 명령 / 검증` 절은 테스트·성능 기록을 겸합니다.
> 전체 별칭은 `tools/xtask/src/task_report/validate.rs`의 `COMPLETE_SECTIONS`에 있습니다.

## WBS

- WBS: <WBS-ID>
- 작업명: <작업 내용 한 줄>
- 상태: <완료 (`[x]`) | 부분 완료 (`[~]`) — 미완료 사유>
- 선행: <선행 WBS-ID 목록 또는 없음>

## 변경 파일

- `<경로>` (<신규 | 수정>) — <변경 요지>
- `docs/task-reports/<WBS-ID>.md`(신규), 계획서(<WBS-ID> 상태 표기 변경)

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `<command>` | <code> |

## 환경과 증거

- 필수 실행 환경: <작업 행의 환경 코드: DEV-MAC | CI-MAC | CI-WIN | LAB-WIN | REF-MAC | REF-WIN>
- 환경별 증거: <CI면 workflow run URL·job name·commit SHA·artifact checksum, LAB·REF면 장비 프로필 ID·OS build·artifact checksum, DEV-MAC만이면 없음>

## 테스트

- <테스트 결과(통과·실패 수와 해당 테스트 이름)와 남은 경고 수>

## 성능

- <성능 원시 결과 경로 또는 해당 없음>

## 차이

- <작업 지시와 다른 점과 이유, 자동 생성 파일이 바뀌었으면 생성 원인과 명령, 또는 없음>

## 남은 위험

- <남은 위험·한계·후속 작업 또는 없음>
