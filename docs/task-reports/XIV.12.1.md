# 작업 보고서: XIV.12.1 conflict 해결 action 모델

- WBS: XIV.12.1
- 작업명: conflict별 resolution을 정의하고 허용 action을 conflict kind별로 제한
- 상태: 완료 (`[x]`)
- 선행: XIV.10.4~XIV.10.7
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/src/candidate.rs` — `Resolution`, `ResolutionKind`, `allowed_resolutions`, `ResolutionError`.
- `crates/semantic-merge/tests/candidate_resolution.rs` — 허용 action 표 검사.

## 구현 요약

- `Resolution`은 `ChooseBase`·`ChooseOurs`·`ChooseTheirs`·`CustomValue(SemanticValue)`·`Delete` 닫힌 enum이고, `ResolutionKind`는
  안정 code 1~5입니다.
- 허용 action은 conflict 대상과 세 상태로 정합니다. 객체: 세 side 선택(`Missing` = 삭제), 어느 side든 `Missing`이면 `Delete`.
  위치: 위치 상태인 side 선택만. 속성: 속성 상태인 side 선택, `CustomValue`, 어느 side든 속성이 없거나 객체가 없으면 `Delete`.
  conflict 종류마다 대상이 정해져 있으므로(예: token-type-vs-value는 객체, conflicting-reorder는 위치나 목록 속성) 종류별 제한이
  이 표로 결정됩니다.
- 허용하지 않은 action은 `ResolutionError::NotAllowed`로 거부합니다(적용은 XXIII.2.3).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge` | 전체 통과(candidate_resolution 2) |
| `cargo clippy -p stellar-semantic-merge --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 속성 값 conflict = base·ours·theirs·custom, 삭제 대 수정 = side 셋·delete, 위치 = side 셋, 한쪽 속성 삭제 = 다섯 모두(`allowed_actions_depend_on_target_and_states`) | 통과 |

## 차이(작업 지시와 다른 점)

- `KeepModified`는 정의하지 않았습니다. 삭제 대 수정 conflict에서 수정 쪽 side 선택과 결과가 같아 같은 해결을 두 이름으로 두지
  않습니다(ADR-0081 결정 4, lane 결정).

## 남은 위험 / 한계

- `CustomValue`의 값 타입은 이 단계에서 검사하지 않습니다. 쓰기 전 검증(XIV.12.3)과 저장 경로의 문서 역투영이 거부합니다.
