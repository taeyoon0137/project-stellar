# 작업 보고서: XII.10.35 인스턴스 교체(swap) command

- WBS: XII.10.35
- 작업명: ComponentRef 교체와 override 보존·탈락 preview
- 상태: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 남은 범위가 있음(아래 본문); 선행 XII.6.5(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 부분 완료 (`[~]`) — 코어 command와 preview는 완료. 메뉴 막대 Swap… 항목·`ShortcutPolicy`·현재 크기 유지는 남았습니다.
- 선행: IX.2.4, IX.1.2, VII.1.6, XII.6.5
- 결정: ADR-0079 결정 1·6

## 필수 실행 환경

- `DEV-MAC` (Rust 단위·property·fuzz 테스트).

## 변경 파일 목록

- `crates/commands/src/node_ops.rs` — `SwapInstance` command, `plan_instance_swap`·`plan_instance_swap_with_entries`,
  `SwapSizing`·`SwapProperty`·`SwapOverride`·`SwapDropReason`·`PreservedOverride`·`InstanceSwapPlan`, 보존·탈락
  matrix fixture.
- `crates/commands/src/{kind,command,codec}.rs` — `swap-instance`(0x0090, codec 2) 등록.
- `crates/commands/src/test_support.rs`, `crates/commands/src/property_tests.rs` — 생성기와 VII.6.1 왕복 등록(random
  document에 인스턴스 하나를 넣고 왕복).
- `crates/commands/src/fuzz.rs` — ADR-0079 종류 10개를 종류별로 생성·변조해 decode·실행하는 fuzz test.
- `docs/task-reports/XII.10.35.md`(신규).

## 구현 요약

- `SwapInstance { node, payload }`는 교체 뒤 payload(새 `ComponentRef`와 보존한 override)를 담아 컴포넌트 library
  없이 재실행됩니다. 역은 원래 payload를 담은 같은 command라 undo가 원래 `ComponentRef`와 override 전체를
  복원합니다. 참조 index도 함께 바뀝니다.
- `plan_instance_swap`은 적용 전 preview입니다. 직접 Property override는 옛 Property와 **이름과 선언 타입이 같은
  새 정의의 공개 Property**로 옮깁니다. 탈락 이유는 `NoMatchingProperty`·`TypeMismatch`·`NotPublic`·`NestedPath`·
  `UnknownProperty`·`NoMatchingSlot`이며 결과는 key 순이라 같은 입력은 같은 preview입니다.
- 선언 타입은 commands가 expressions crate에 의존하지 않도록 제네릭 `T: PartialEq`로 같음만 비교합니다.
  `plan_instance_swap_with_entries`가 library 항목(`TypeSpec`)을 그대로 넘깁니다.
- Slot: 새 정의에 같은 id의 Slot이 있고 content가 그 Slot의 종류·개수 규칙을 지킬 때만 옮깁니다(`SlotRuleMismatch`
  탈락 이유 추가). content가 있는 Slot이 탈락하면 인스턴스 소유 node가 투영할 곳 없이 남으므로(IX.3.2) 계획을 거부하고,
  `SwapInstance` 검증도 content 소유·중복·투영 유지를 확인합니다.
- 크기: 인스턴스 payload에는 자기 크기가 없어 새 정의 root의 크기를 씁니다(`SwapSizing::DefinitionDefault`).
  `SwapSizing::KeepCurrent`는 계획 단계에서 거부합니다(ADR-0079 결정 6).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands swap_tests` | 4 passed(검토 보완 후): slot content 보존·거부·command 검증 포함, 보존·탈락 matrix(6개 탈락 이유, slot), 같은 입력 같은 preview, undo 뒤 원래 `ComponentRef`·override 6개 복원, codec 왕복, 크기 유지·비인스턴스·NIL 거부, library fixture 항목 경로 |
| `cargo test -p stellar-commands property_tests` | `swap_instance_round_trip`(120건 이상) 포함 통과 |
| `cargo test -p stellar-commands fuzz` | `adr_0079_kinds_survive_mutation_fuzz`, `smoke_fuzz_command_execution_is_panic_free` 통과 |
| `cargo test -p stellar-commands -p stellar-history` | commands 152, history 50 통과 |
| `cargo clippy -p stellar-commands -p stellar-history -p stellar-document --all-targets -- -D warnings` | 경고 없음 |

| 검사 | 결과 |
|---|---|
| 보존·탈락 matrix fixture | 통과 |
| undo 후 원래 ComponentRef와 override 복원 | 통과 |
| 같은 입력 같은 preview | 통과 |
| `ShortcutPolicy.audit()` 문제 0건 | 미실행 — 메뉴·`ShortcutPolicy`는 셸 작업이라 이번 범위(엔진만, UI 재설계 전) 밖 |

## 차이(작업 지시와 다른 점)

- 메뉴 막대 Object 메뉴 Swap… 항목, `docs/design/ui/tools_menus.md` 표, `ShortcutPolicy` 갱신은 하지 않았습니다.
  사용자 방향(2026-11-23: 엔진·기능만, UI는 처음부터 재설계)에 따른 것입니다.
- "현재 크기 유지" 선택지는 enum으로 두되 거부합니다. 인스턴스 크기를 CSS 값으로 담을 override 모델이 아직
  없습니다(ADR-0074, ADR-0079 결정 6).

## 남은 위험 / 한계

- 셸 메뉴·`ShortcutPolicy`·FFI 연결이 남았습니다.
- 현재 크기 유지는 인스턴스 크기 override 모델 결정 후에 구현해야 합니다.
- Slot은 이름이 없어 같은 Slot id가 새 정의에 있을 때만 보존합니다.
