# 작업 보고서: XIV.7.2 unstage dependency 검사와 필수 함께-unstage 변경

- WBS: XIV.7.2
- 작업명: unstage dependency graph를 검사하고 남은 index snapshot이 invalid하면 필수 함께-unstage 변경 반환
- 상태: 완료 (`[x]`)
- 선행: XIV.5.2(`[x]`, 이 lane), XIV.7.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/stage.rs` — `plan_unstage`, component Property·override staged fixture와 테스트.
- `docs/task-reports/XIV.7.2.md`(신규), 계획서(XIV.7.2 `[ ]`→`[x]`).

## 구현 요약

- `plan_unstage(head, index, request)`는 staged 선택을 역변경 ID로 바꿔 Index → HEAD 방향 diff의 dependency
  graph(XIV.5.2)로 closure를 구하고, 선택만 되돌린 Index를 semantic validator(XIV.5.3)로 검사합니다. 결과
  selection·`required_additions`는 staged ID 기준이며 반환만 합니다(자동 unstage 없음). 승인 후 실제 candidate는
  `build_unstage_candidate`(XIV.7.1)가 같은 승인 규칙으로 만듭니다.
- 컴포넌트 interface에서 Property를 빼는 역변경은 그 Property를 쓰는 override의 해제에(interface member 제거),
  override를 되살리는 역변경은 그 Property를 interface에 넣는 변경에(interface member 추가) 의존하므로, 남은
  Index에서 override와 interface가 어긋나지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | unit 77 passed, fuzz 통합 4 passed, 0 failed |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **component property/override fixture**(builder로 만든 컴포넌트 300·인스턴스 400): staged = interface [310]→[311](A), override 311 추가(B), override 310 제거(C). A만 unstage → required = {B}, 선택만 적용 시 `UnknownInterfaceMember(311)`; 미승인 candidate 거부, 승인 후 candidate에서 override 311·310 모두 없음·새 위반 0; B만 unstage → 승인 불필요·Valid; C만 unstage → required = {A, B}·Invalid; 계획 전후 snapshot 동일(`unstage_dependencies_keep_component_property_and_override_consistent`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- binding expression payload 안 참조의 unstage 의존은 보수적 scan(XIV.5.2)에 의존합니다.
- 승인된 unstage candidate를 Git index에 기록하는 경로(XIV.7.3)는 XIV.6 선행이 미완이라 이 lane에서 다루지 않았습니다.
