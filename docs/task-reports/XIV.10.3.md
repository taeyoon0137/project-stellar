# 작업 보고서: XIV.10.3 같은 값 변경 병합

- WBS: XIV.10.3
- 작업명: 같은 PropertyPath를 양쪽이 같은 값으로 바꾼 경우 conflict 없이 한 변경으로 합침
- 상태: 완료 (`[x]`)
- 선행: XIV.10.2 `[x]`(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/src/merge.rs` — `same_new_value`, `same_position`, 동일 구조 변경 처리.
- `docs/task-reports/XIV.10.3.md`(신규), 계획서(XIV.10.3 `[ ]`→`[x]`).

## 구현 요약

- **속성**: 양쪽이 같은 경로를 바꿨고 이후 값이 canonical 비교로 같으면(둘 다 속성 삭제 포함)
  ours의 변경 하나만 받아들입니다. 이전 값은 양쪽 모두 base이므로 같습니다.
- **같은 결과로 확장**: 같은 이후 위치의 이동·재정렬, 같은 객체 삭제·추가·교체(변경 내용 동일)도
  conflict 없이 한 번만 받아들입니다(아키텍처 §27.1 "같은 Property를 같은 값으로 변경"의 위치·
  구조 대응).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge` | 13 passed, 0 failed |
| `cargo fmt -p stellar-semantic-merge -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-merge --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 같은 이름 변경 + 한쪽 추가 변경 → conflict 0, 결과의 해당 변경 1개; 같은 속성 삭제·같은 이동·같은 삭제·같은 추가 → conflict 0(`identical_edit_merges_as_one_change`) | 통과 |
| 100 seed: `merge(base, X, X) == X`, conflict 0(`same_edits_on_both_sides_property`) | 통과 |

## 차이(작업 지시와 다른 점)

- 지시는 PropertyPath 대상이며, 같은 결과의 위치·구조 변경 병합을 함께 넣었습니다(같은 의도의
  양쪽 편집이 conflict가 되지 않도록).

## 남은 위험 / 한계

- 같은 값으로 바꿨지만 출처(provenance)가 다른 경우(예: 인스턴스 정의 참조를 동시에 교체)에도
  ours 쪽 변경의 출처를 씁니다. 적용 결과는 같습니다.
