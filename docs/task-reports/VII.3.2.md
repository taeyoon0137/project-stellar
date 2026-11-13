# 작업 보고서: VII.3.2 SetSelection history event

- WBS: VII.3.2
- 작업명: `SetSelection` history event — 존재하지 않거나 숨김 정책상 선택 불가 ID 필터
- 상태: 완료 (`[x]`)
- 선행: VII.3.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/selection.rs` — `SelectionPolicy`, `SetSelection`(filtered/apply/codec), `AppliedSelection`,
  `Selection` canonical 직렬화, 테스트 2건. VII.3.1의 임시 `allow(dead_code)` 제거.
- `docs/task-reports/VII.3.2.md`(신규), 계획서(VII.3.2 `[ ]`→`[x]`).

## 구현 요약

- **event**: 선택 변경은 문서 command가 아니라 view 상태에 적용하는 history event입니다. `apply`는 문서를 바꾸지 않고
  `WindowViewState`의 선택을 교체하며, 실제 적용한 선택(정방향)과 이전 선택(역방향)을 `AppliedSelection`으로 돌려줍니다.
  history는 이 둘을 기록합니다(VII.4.6).
- **필터**: 문서에 없는 id와, `SelectionPolicy`(기본: 숨김 거부)상 자신 또는 조상이 숨김인 node를 거릅니다. 걸러진 결과에
  primary 규칙(VII.3.1: 요청 primary가 남아 있으면 유지, 아니면 마지막 원소)을 다시 적용합니다. 정방향은 걸러진 결과라 다시
  적용해도 같은 선택입니다(redo 결정론).
- **직렬화**: 수 `u32` + id* + primary(flag + id). 디코딩은 정규형(중복·NIL 없음, primary 규칙 만족)만 받습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 59 passed, doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 첫 선택의 역은 빈 선택, 없는 id·숨김·숨김 조상 자손·숨김 leaf 필터, primary가 걸러지면 마지막 원소, 정방향 재적용 결정론, 역방향이 primary까지 복원, 숨김 허용 정책(`set_selection_filters_and_inverts`) | 통과 |
| codec round-trip(빈 선택 포함), 비정규형·trailing 거부(`selection_codec_round_trips_and_rejects_non_canonical`) | 통과 |

## 차이(작업 지시와 다른 점)

- 숨김 정책은 "자신 또는 조상이 숨김이면 선택 불가"를 기본으로 하고 정책 값으로 허용할 수 있게 했습니다. 잠금 node는 layer
  panel 선택을 위해 선택 가능으로 둡니다(캔버스 hit test 제외는 XI 단계 정책).

## 남은 위험 / 한계

- 숨김 판정은 node마다 부모 chain을 따라갑니다(O(depth)).
