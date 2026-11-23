# 작업 보고서: XXIII.2.3 Candidate 해결·선택 command와 역 command

- WBS: XXIII.2.3
- 작업명: C에서 A/B object·property를 선택하는 resolution command
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록: 구현·검증과 선행 판정은 이미 `[x]` 조건을 충족했고 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 `[~]`에 머물렀습니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:`·`Test:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장(`cargo xtask generate-ledger`)이 근거 커밋과 검증 시각을 채우지 못합니다. 구현·검증은 완료입니다; 선행 XXIII.2.2(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]`)
- 선행: XXIII.2.2
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/src/candidate.rs` — `CandidateCommand`(`Resolve`·`Select`·`Restore`), `Side`, `CandidateSnapshot::apply`,
  `resolution`, `put_unit`, Property 삭제 해결의 override 정리.
- `crates/semantic-merge/tests/candidate_resolution.rs` — apply/inverse 검사.

## 구현 요약

- `Resolve`는 conflict 하나에 허용된 action(XIV.12.1)을 적용하고 해결 기록·출처를 남깁니다. `Select`는 임의 단위를 base·A·B 중 한
  상태로 둡니다(그 단위가 conflict면 그 side 선택 해결과 같음).
- `apply`는 C 사본에서 전부 성공할 때만 C를 바꾸고, 단위 상태·출처·해결 기록의 이전 값을 담은 `Restore` 역 command를 돌려줍니다.
  역 command의 역도 `Restore`입니다. C 밖(파일·index)은 건드리지 않습니다.
- Property 삭제 conflict(종류 7·9)를 삭제 쪽으로 해결하면 그 컴포넌트를 참조하는 인스턴스에서 목록에 없는 Property override를
  지웁니다(삭제 쪽 상태와 같음, 출처 = 그 side).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge` | 전체 통과(candidate_resolution 7) |
| `cargo clippy -p stellar-semantic-merge --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| Resolve → 값·출처·unresolved 0, 입력 A·B 동일, inverse → C 전체 동일, redo 재적용 | 통과 |
| 허용하지 않은 action·없는 conflict는 오류이고 C 동일 | 통과 |
| CustomValue 출처 Custom, 자동 병합된 A 변경을 base로 Select 후 inverse | 통과 |
| Property 삭제 쪽 해결 시 override 정리, inverse로 원복 | 통과 |
| 무작위 Select 6개 순서 64가지를 역 command로 모두 되돌리면 처음 C와 같음 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 객체 단위 선택은 객체 전체를 그 side 객체로 바꿉니다. 같은 객체의 다른 쪽 속성 변경은 함께 사라지며, 필요하면 속성 단위 Select로
  다시 고릅니다.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XXIII.2.3)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `d24d0026` Feat: Candidate 해결·선택 command와 역 command 구현
