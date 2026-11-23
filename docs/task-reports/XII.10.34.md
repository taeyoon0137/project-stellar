# 작업 보고서: XII.10.34 ComponentInstance 캔버스 편집

## WBS

- WBS: XII.10.34
- 작업명: 인스턴스 안 깊은 선택 읽기 전용 윤곽, 공개 Property 연결 값 override, Reset·Detach·정의로 이동 코어
- 상태: 부분 완료 (`[~]`) — 깊은 선택 윤곽, 허용·거부 판정, override Set·속성별·전체 Reset, 문맥 동작 판정은
  끝났습니다. Slot 끌어 놓기는 문서 모델이 인스턴스 자식을 허용하지 않아 만들 수 없었습니다. Detach 실행 command와
  메뉴 막대·`ShortcutPolicy`(셸)도 남았습니다.
- 선행: IX.2.4, IX.2.6, IX.3.3, IX.4.2, XII.10.4, XII.6.5
- 결정: ADR-0118 결정 4·5

## 변경 파일

- `crates/runtime/src/canvas/instance.rs`(신규) — `pick_inside`(커서 아래 가장 위 정의 node와 world 윤곽),
  `PropertyLink`, `plan_inner_edit`(공개 Property 연결 값만 override Set), `plan_reset`, `plan_reset_all`,
  `Refused`(이유와 Detach 가능 여부), `instance_actions`(속성별·전체 Reset, Detach, 정의로 이동).
- `crates/runtime/src/canvas/instance_tests.rs`(신규) — fixture.
- `crates/runtime/src/canvas.rs`(수정) — `instance` mod 등록.
- `docs/task-reports/XII.10.34.md`(신규).

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-runtime --lib canvas::instance` | 0 (3 passed) |
| `cargo clippy -p stellar-text -p stellar-commands -p stellar-runtime --all-targets --locked -- -D warnings` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC만이라 없음.

## 테스트

- 깊은 선택(`deep_pick_returns_a_read_only_outline_of_the_topmost_definition_node`): (500,300)에 놓인 인스턴스 안에서
  자식 정의 node 902의 world 윤곽 (510,310)–(550,350)과 root 901을 고릅니다. 인스턴스 밖이나 인스턴스가 아닌 node는
  없음이고, 문서는 바뀌지 않습니다.
- override·reset fixture(`linked_public_value_becomes_an_override_and_resets_undo_once_each`): Button label Text의
  연결 값 편집은 label override Set 하나(history 1건)입니다. 속성별 Reset 뒤 문서는 처음과 같습니다(희소 저장).
  전체 Reset도 동작마다 1건이고, undo로 정확히 되돌아갑니다.
- 거부 fixture(`unlinked_private_nested_and_locked_edits_are_refused_with_detach_path`): 연결 없음, 값 타입 불일치,
  nested 닫기 Button 직접 편집, 비공개 Property, 잠긴 인스턴스를 거부합니다. 잠금 외에는 Detach 경로를 함께 냅니다.
- 남은 경고 0.

## 성능

- 해당 없음. 깊은 선택은 정의 tree node 수에 선형입니다.

## 차이

- override Set·Reset은 새 command 종류 없이 인스턴스 payload 교체 command(`swap-instance`) 하나로 씁니다. command
  registry를 바꾸지 않기 위해서입니다(ADR-0118 결정 5).
- 정의 node 속성과 Property의 연결은 binding 저장소가 넘기는 `PropertyLink` 목록입니다. binding은 문서 모델에 저장되지
  않습니다.
- Slot 끌어 놓기는 만들지 않았습니다. `validate::can_have_children`이 ComponentInstance를 container로 보지 않아
  `InsertNode`·`MoveNode`와 문서 validator가 인스턴스 자식을 거부합니다. 시도한 계획 함수가 `MoveNode` 검증에서
  `InvalidParent`로 실패하는 것을 확인하고 코드를 지웠습니다. 그 규칙은 이 작업 소유 밖(document crate)이고 문서
  의미 변경입니다.
- 문맥 메뉴·메뉴 막대 Object 메뉴 항목, `docs/design/ui/tools_menus.md` 표, `ShortcutPolicy`는 셸 작업이라 하지
  않았습니다(사용자 방향 2026-11-23). 그래서 `ShortcutPolicy.audit()`은 실행하지 않았습니다.

## 남은 위험

- Slot 끌어 놓기에는 인스턴스 소유 node를 문서에 두는 결정이 필요합니다(document validator와 `can_have_children`).
- Detach 실행은 command registry 등록(IX.2.6 함수 쌍을 command로)이 필요합니다.
- override 편집의 history 이름은 "인스턴스 교체"로 보입니다.
- nested instance 정의 node는 scene 투영이 아직 그리지 않아 깊은 선택으로도 고를 수 없습니다.
