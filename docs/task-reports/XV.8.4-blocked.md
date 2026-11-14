# 차단 보고서: XV.8.4 외부 library 전체 integration fixture

## WBS

- WBS: XV.8.4
- 작업명: 외부 library 전체 integration fixture(A→C v1, B→C v2, local override, offline cache, update)
- 상태: `[!]`
- 선행: XV.1~XV.7(절 단위)

## 실패 선행조건

- 절 단위 선행은 필요한 코드가 있으면 완화할 수 있지만, fixture 구성 요소 중 두 가지의 코드가 없습니다.
  - **update**: lockfile golden에 필요한 적용 단계 XV.7.4가 `[!]`입니다(XV.7.3 → IX.4.6 `[ ]`). 검토 단계
    XV.7.1·XV.7.2만 있습니다.
  - **local override**: override root에서 snapshot을 읽고 진단하는 XV.6.3·XV.6.4·XV.6.5가 `[!]`, 대상 검증
    XV.6.2가 `[~]`입니다.
- 나머지 구성 요소(A→C v1·B→C v2 scope 격리, offline cache 부분 열기)는 XV.5.4·XV.5.5에서 각각 검증했습니다.

## 재현 명령

- `grep -E '^\| XV\.(6\.[2-5]|7\.[3-5]) \|' docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`

## 필요한 결정과 환경

- XV.7.3\~XV.7.5와 XV.6.2\~XV.6.5 완료가 필요합니다.
- 필수 환경: `DEV-MAC`.

## 변경되지 않은 파일

- 기능 코드는 변경하지 않았습니다(계획 §1.2 규칙 4). 이 보고서와 계획서 상태 표기만 바꿨습니다.

## 재개 조건

- 위 행이 `[x]`가 된 뒤 착수합니다. fixture 재료: XV.5.4 그래프 scope 테스트(`ref_resolution`), XV.5.5 offline
  부분 열기 테스트(`library_load`), XV.7.1 실제 Git update 후보 테스트(`library_update`).
