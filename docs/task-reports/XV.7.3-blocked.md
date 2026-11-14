# 차단 보고서: XV.7.3 candidate 적용 전 external refs·instance override 검증

## WBS

- WBS: XV.7.3
- 작업명: candidate library를 적용하기 전 사용 project의 external refs와 instance overrides 검증(removed property override → orphan diagnostic)
- 상태: `[!]`
- 선행: IX.4.6(`[ ]`), XV.5.2(`[x]`)

## 실패 선행조건

- 엄격 선행 IX.4.6(chain 일부 삭제·타입 변경 시 상위 chain 전체 orphan 표시, 저장에서 삭제하지 않음)이 `[ ]`입니다.
  removed property override를 orphan diagnostic으로 만드는 규칙이 IX.4.6의 orphan 표시 모델에 의존합니다.
- IX.4.6은 document crate 범위라 이 lane(deps2)의 허용 경로에서 구현할 수 없습니다.

## 재현 명령

- `grep -E '^\| IX\.4\.6 \|' docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`

## 필요한 결정과 환경

- IX.4.6 담당 lane의 완료가 필요합니다.
- 필수 환경: `DEV-MAC`.

## 변경되지 않은 파일

- 기능 코드는 변경하지 않았습니다(계획 §1.2 규칙 4). 이 보고서와 계획서 상태 표기만 바꿨습니다.

## 재개 조건

- IX.4.6 `[x]` 후 착수합니다. 재료는 이 lane에 있습니다: XV.7.1 `LibraryUpdateCandidate`, XV.7.2
  `diff_library_snapshots`·`LibraryResourceDiff::removed_properties`, XV.5.4 `LibraryGraphInstance`(scope 해석),
  XV.5.3 `check_library_edit`(공개 property·타입 규칙).
