# 차단 보고서: XV.7.3 candidate 적용 전 external refs·instance override 검증

## WBS

- WBS: XV.7.3
- 작업명: candidate library를 적용하기 전 사용 project의 external refs와 instance overrides 검증(removed property override → orphan diagnostic)
- 상태: `[!]`(작성 당시) → 통합 후 `[ ]`(차단 해소, 미착수 — 아래 "통합 후 확인")
- 선행: IX.4.6(작성 당시 `[ ]`, 통합 후 `[x]`), XV.5.2(`[x]`)

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

## 통합 후 확인 (2026-11-14: 선행 충족, `[ ]`로 되돌림)

- **선행 IX.4.6 해소**: 통합 브랜치 계획서에서 IX.4.6이 `[x]`입니다(`75a23eb` Task(IX.4.6)). removed property override를
  orphan으로 표시할 모델은 document crate `component::drilling`의 `OrphanCause::PropertyDeleted`·`PropertyRetyped`·
  `UpstreamOfBreak`와 `DrillingSet::orphan_chain`입니다. 위 "실패 선행조건"은 작성 당시 상태입니다.
- **다른 차단 원인 없음**: 엄격 선행 두 개(IX.4.6, XV.5.2)가 모두 `[x]`이고 `verify-task XV.7.3`이 종료 코드 0입니다.
  "이 lane의 허용 경로에서 구현할 수 없음"은 그 lane의 범위 제한이었고 차단 사유가 아닙니다. crate graph 결정도 필요하지
  않습니다. document(orphan 모델)와 dependencies(XV.7.1·XV.7.2·XV.5.3·XV.5.4 재료)를 함께 쓸 수 있는 project crate가 이미
  정책에 있습니다(`tools/xtask/src/crate_graph.rs`). 연결 위치는 착수할 때 정합니다.
- 그래서 `[!]`의 근거가 없어 계획서 상태를 `[ ]`(미착수)로 되돌렸습니다. 기능 코드는 바꾸지 않았고, ledger는
  `cargo run -q -p xtask -- generate-ledger`로 다시 만들었습니다(XV.7.3 `not_started`).
- 후속 행 XV.7.4·XV.7.5는 엄격 선행(XV.7.3·XV.7.4)이 아직 `[x]`가 아니라 `[!]`를 유지합니다. 각 차단 보고서에 같은 확인을
  적었습니다.

| 명령 (기준 `ae10970` + 이 수정) | 결과 |
|---|---|
| `grep -E '^\| IX\.4\.6 \|' docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` | 상태 `[x]` |
| `target/debug/xtask verify-task XV.7.3` | 종료 코드 0 |
