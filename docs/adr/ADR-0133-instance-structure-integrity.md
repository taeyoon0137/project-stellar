# ADR-0133: 인스턴스 구조 무결성(Slot content 구조 command 규칙, Detach의 Property override, 가이드 복제, Text 입력 command ID)

## Status

Accepted (2026-11-24). ADR-0129의 후속입니다. ADR-0129 Consequences의 "인스턴스 자식이 있는 문서를 각 경로로 확인하지
않음"·"Detach는 Property override 값을 반영하지 않음", 결정 4.5의 마지막 항목(Property override 미반영), ADR-0126
Consequences의 "Frame 복제·붙여넣기 때 가이드·그리드가 따라가지 않음"을 이 ADR이 대체합니다.

## Context

- ADR-0129 결정 3은 ComponentInstance의 자식을 "그 인스턴스 slot override에 정확히 한 번 투영된 Slot content"로
  정했습니다. 구조를 바꾸는 일반 command는 이 규칙을 몰랐습니다.
  - `delete-subtree`로 Slot content를 지우면 slot override가 지워진 node를 가리킨 채 남았습니다(문서 validator는 자식이
    아닌 node를 가리키는 content를 위반으로 보지 않습니다, ADR-0129 결정 3.2). 그 역 `insert-node`는 삽입 root의 부모가
    인스턴스라 거부되어 undo가 실패했습니다.
  - `move-node`로 Slot content를 인스턴스 밖으로 옮겨도 같았습니다(역 이동이 자유 자식 판정에 막힘).
  - Slot content를 가진 인스턴스를 지운 뒤의 undo, 그런 인스턴스의 복제·붙여넣기는 `insert-node`가 인스턴스 아래
    자식을 거부해 실패했습니다.
- Detach(`plan_detach`)는 Property override 값을 복제 node에 반영하지 않았습니다. 정의 node 속성과 Property의
  연결(binding)은 문서에 없고, 인스턴스 안 값 편집(runtime `canvas::instance::plan_inner_edit`)은 binding 저장소가 넘기는
  연결 목록(`PropertyLink`)을 받아 씁니다.
- 가이드·레이아웃 그리드 기록(ADR-0126)은 소유 Frame StableId로 문서 모델에 있습니다. `insert-node`·`duplicate-subtree`·
  붙여넣기는 이 기록을 옮기지 않았고, 삭제 undo는 같은 session 모델에 남은 기록에 기대어 history journal을 다시 연
  뒤에는 기록을 잃었습니다. layout record(ADR-0086)는 같은 문제를 `insert-node` 끝 절로 풀었습니다.
- 앱 Text 입력(`app::text_input`, ADR-0129 결정 2)의 `begin()`은 남은 편집을 끝낼 때(빈 새 Text 정리, 남은 조합 확정)
  요청 ID로 만든 CommandId를 쓰고, 같은 요청의 이어지는 입력(`replace_canvas_text`)도 같은 CommandId를 썼습니다. 또 끝내기
  뒤에 요청 revision과 문서 revision이 같은지 다시 봐서, 끝내기가 무엇을 기록하면(revision이 오름) 그 요청은 끝내기만
  남기고 실패했습니다(다른 Text를 바로 고르거나 입력하는 첫 요청이 실패).
- 제약: 사용자 방향(2026-11-23) — 엔진·기능만 만들고 셸 FFI·Swift UI는 바꾸지 않습니다. command 적용은 문서 모델만
  봅니다(컴포넌트 library는 계획 단계에만 있음).

## Decision

### 1. 일반 구조 command는 Slot content의 투영을 깨뜨리지 않습니다(거부 규칙 하나)

1. `delete-subtree`·`move-node`·`insert-node`는 slot override를 바꾸지 않습니다. 그래서 투영을 깨뜨리는 변경은 **거부**
   합니다(빼기 대신 거부). 판정 함수는 `commands::node_ops::slot_content_owner`(부모가 `can_own_slot_content` 종류인지)
   하나이고, 거부 이유는 `CommandError::InvalidRequest`입니다.
   - `delete-subtree`: root가 Slot content(인스턴스의 직접 자식)이면 거부합니다. Slot content 아래 자손 삭제, 인스턴스 자신의
     삭제는 받습니다(투영이 그대로이거나 인스턴스와 함께 사라짐).
   - `move-node`: Slot content를 인스턴스 밖으로 옮기면 거부합니다. 같은 인스턴스 안 OrderKey 변경은 받습니다(투영 순서는
     slot override content 순서이고 OrderKey와 무관). 인스턴스 아래로 옮기기는 이전과 같이 자유 자식 판정이 거부합니다.
   - `insert-node`: 삽입 root를 기존 인스턴스 아래에 두는 것은 이전과 같이 거부합니다. 삽입하는 인스턴스 아래 자식은
     받되, 그 인스턴스 slot override에 정확히 한 번 투영돼야 합니다. 판정은 문서 validator와 같은
     `validate::instance_projection_issues`입니다. 그래서 인스턴스 subtree 삭제의 undo, 인스턴스 복제(`duplicate-subtree`,
     slot content id는 이미 remap됨), 붙여넣기가 인스턴스 자식을 그대로 옮깁니다.
2. 빼기(구조 command가 slot override에서 content를 함께 뺌)를 고르지 않은 이유: `delete-subtree`의 역은 `insert-node`이고
   두 command 모두 인스턴스 payload를 바꾸는 절이 없습니다. 빼려면 두 command 형식에 인스턴스 레코드 교체 절을 더하고
   역에서 원래 투영 위치를 되살려야 합니다. 같은 일을 이미 하는 `place-slot-content`(ADR-0129 결정 4)가 있어 두 번째
   경로가 됩니다.

### 2. Slot content 지우기는 `place-slot-content` 하나입니다

1. `commands::detach::plan_slot_content_delete(model, library, node, restore_default)`가 content를 slot override 투영에서
   빼고 그 subtree를 자식부터 지우는 `InstanceRestructure` 하나를 만듭니다(history 1건, undo 1번에 subtree·투영 복원).
2. 마지막 content를 지우면 `restore_default`일 때 override를 지워 default content로 돌아가고, 아니면 빈 override로
   남깁니다(runtime `plan_slot_take_out`과 같은 규칙). 남는 content는 IX.3 `ComponentSlot::check_content`(종류·개수)를
   지켜야 합니다.
3. 거부: Slot content가 아님, content·인스턴스 잠금(`commands::lock::edit_permission`), 미해결 컴포넌트, 선언되지 않은 Slot,
   Slot 규칙 위반, 삭제 정책 위반(subtree 밖에서 subtree 안을 참조, `delete-subtree`와 같은 판정
   `node_ops::check_external_inbound`).
4. 선택 삭제처럼 여러 node를 지우는 호출자는 `slot_content_owner`로 Slot content를 골라 이 계획으로 보냅니다(런타임 연결은
   UI 재설계 때 합니다).

### 3. Detach는 공개 Property override 값을 연결된 복제 node 속성에 씁니다

1. `plan_detach(model, library, links, instance, generator, clock)`가 연결 목록 `links: &[PropertyLink]`를 받습니다.
   `PropertyLink`(정의 node, 속성 key, Property)는 `commands::detach`가 정의하고 runtime `canvas::instance`가 같은 타입을
   다시 내보냅니다(연결 타입 하나).
2. 쓰는 값은 IX.2.3 규칙(`override_rules::check_override`: 공개·선언·타입)을 통과한 직접 override뿐입니다. override가
   없거나 규칙을 어긴 Property(선언에서 사라진 Property의 남은 override 등)는 정의 값 그대로입니다(IX.1.4 Property 값
   캐시와 같은 규칙).
3. 값 쓰기는 속성 registry 쓰기 경로 `property::write_property` 하나입니다(`SetProperty` 적용도 같은 함수로 옮겼습니다).
   Text 본문(`payload_metadata::text_keys::TEXT_CONTENT`)은 VI.7.5 본문 교체(`text::edit_text`)로 씁니다.
4. Property 값(`TypedValue`)과 registry 값 타입의 대응은 단위를 바꾸지 않는 것만 받습니다(CSS 값 그대로, ADR-0074).
   bool ← Boolean, 문자열 ← String, 비음수 ← `px` 길이·수(0 이상), 수 ← 수·`px`·`deg`, 색·선택 색 ← Color, 0..=1 ← 수·`%`.
   registry 값 타입 `Number`는 `px` 좌표·`deg` 각도·개수를 함께 담으므로 payload registry 값 종류
   (`property::payload_value_kind`)로 단위를 맞춥니다. `px` 길이는 좌표 속성에만, `deg` 각도는 각도 속성에만 쓰고, 개수는
   수만 받습니다.
5. 쓸 수 없는 연결(그 node에 없는 속성, 맞지 않는 값)은 `DetachPlanError::Link`로 거부합니다. 반영하지 못한 채 Detach하면
   화면이 바뀌기 때문입니다(IX.2.6 검증 "detach 후 시각 의미 동일"). 복제되지 않은 정의 node(override된 Slot의 default
   content, nested 인스턴스 안 node)를 가리키는 연결은 건너뜁니다.

### 4. 가이드·레이아웃 그리드 기록은 layout record와 같은 규칙으로 따라갑니다

1. `insert-node`에 `node_guides`(삽입하는 Frame의 비어 있지 않은 기록, 소유 node 오름차순)를 더합니다. 검증은 문서
   validator와 같은 `guides::persist::check_records`입니다.
2. `delete-subtree`의 역 `insert-node`는 subtree의 가이드 기록을 담습니다(history journal을 다시 연 뒤의 삭제 undo도
   기록을 되살림). `insert-node` 적용은 기록을 담지 않은 삽입 node에 같은 session에서 남은 기록을 지웁니다.
3. `duplicate-subtree`와 붙여넣기(`ClipboardSubtree`, 캡처는 `delete-subtree` 역과 같은 snapshot)는 기록을 새 소유 Frame
   으로 옮깁니다. 가이드 id는 소유 Frame 안에서만 유일하므로(ADR-0126 결정 1.4) 그대로 둡니다.
4. 인스턴스 구조 command(`InstanceRestructure`)도 만들 Frame의 기록(`insert_guides`)을 담고, 지운 node의 기록을 역에
   담습니다.
5. codec: 두 command 끝의 선택 절을 함께 씁니다(`node_ops::encode_insert_tail`·`decode_insert_tail`, 두 command가 같은
   함수). 둘 다 없으면 절이 없고, 가이드 기록이 없으면 layout record 절만 있어 이전 bytes와 같습니다. 가이드 기록이
   있으면 layout record 절(수 0 허용)에 이어 가이드 절(수 1 이상)을 씁니다. 형식은 하나뿐입니다(수 0 layout 절은 가이드
   절이 따를 때만, 빈 가이드 절 거부, 오름차순). command codec 버전을 9로 올립니다.

### 5. 한 요청의 history 항목은 서로 다른 CommandId를 씁니다(앱 Text 입력)

1. `app::text_input`의 `begin()`이 다른 대상의 남은 편집을 끝낼 때(빈 새 Text 정리 또는 조합 확정 기록) 쓰는 CommandId는
   요청 CommandId와 용도 표지 `text-end`에서 만듭니다(`end_command_id`). 같은 요청의 입력 기록은 이전과 같이
   `frame_move::command_id(창, 요청)`입니다.
2. 요청 revision은 끝내기 전에 한 번 확인합니다. 끝내기가 올린 revision은 이 요청이 만든 변화이고 끝내는 편집의 대상(다른
   node)만 바꾸므로, 끝내기 뒤에는 revision 비교 대신 새 대상 확인(편집 가능, 범위, 부모 View)을 끝내기 뒤 문서로 합니다.
   그래서 `replace_canvas_text`는 끝내기와 입력을 한 요청에서 서로 다른 CommandId로 기록합니다.
3. `end_canvas_text`의 끝내기는 그 요청의 유일한 기록이라 요청 CommandId를 씁니다.

### 6. 인스턴스 자식이 있는 문서의 경로 확인

Slot content가 있는 문서를 각 경로로 보내는 통합 시험(`tests/integration/tests/instance_slot_content_paths.rs`)을 둡니다.
확인 결과(2026-11-24):

1. scene 그리기: 전체 변환에서 content는 slot host 가상 node 아래에 한 번 들어갑니다(ADR-0084 결정 3의 투영 그대로).
   content·안쪽 node 편집, Slot content 지우기, 끌어 놓기, Detach의 증분 편집(`scene_edits`)이 새 전체 변환과 같습니다.
   고칠 것이 없었습니다.
2. semantic diff: snapshot → 문서 레코드 역투영이 원래 레코드와 같고, Slot content 지우기의 diff(content subtree 삭제 +
   인스턴스 slot override 변경)를 적용하면 결과 snapshot과 같습니다. 고칠 것이 없었습니다.
3. 텍스트 원본 codec(View v4)과 저장 chunk: 쓰고 다시 읽은 레코드가 같고 다시 쓴 bytes가 같습니다. 고칠 것이 없었습니다.
4. codegen(Web 내보내기): Slot content 투영 출력은 지원하지 않습니다. 인스턴스에 소유 자식이 있으면 blocking 진단
   `unsupported-component-instance`로 거부하고 HTML을 내지 않습니다(content를 조용히 빼지 않음). 이 거부를 시험으로
   고정합니다. 투영 출력(정의 slot host 아래 content 배치)은 codegen 작업이 Property override 출력과 함께 정합니다.

## Alternatives

- 구조 command가 slot override에서 content를 함께 뺌: 결정 1.2. `place-slot-content`와 같은 일을 하는 두 번째 경로가 되고
  `delete-subtree`·`insert-node` 형식을 바꿔야 합니다.
- 문서 validator가 자식이 아닌 node를 가리키는 slot content를 위반으로 봄: 이 규칙 전에 저장된 문서의 slot override가
  모두 그런 참조라 저장 문서가 열리지 않습니다(ADR-0129 결정 3.2). 채택하지 않았습니다.
- Detach가 쓸 수 없는 연결을 건너뜀: Detach 뒤 화면이 바뀌어 IX.2.6 검증을 어깁니다.
- Detach가 binding 저장소를 직접 읽음: binding은 문서·command 계층 밖입니다(VIII.6). 인스턴스 안 값 편집과 같이 연결
  목록을 인자로 받습니다.
- 가이드 기록을 별도 command(`set-node-guides`)로 복제 뒤에 붙임: 복제·붙여넣기가 history 항목 둘이 되거나 transaction
  하나에 command 둘이 되고, 삭제 undo는 여전히 남은 기록에 기댑니다. layout record와 같은 끝 절로 묶었습니다.
- Text 입력의 끝내기와 입력을 한 history 항목으로 합침: 빈 Text 정리는 별도 항목이어야 합니다(ADR-0129 결정 1.1).

## Consequences

- 구조 command만 쓰는 호출자(CLI `stellar apply`의 `delete-subtree`·`move-node` 입력, agent batch 등)는 Slot content를
  지우거나 꺼낼 수 없습니다. 거부 이유 문구가 `place-slot-content` 경로를 알립니다. 다만 지금 command 입력 규칙
  (`stellar_ops::mcp_command_input`)에는 `place-slot-content`·Slot content 지우기 입력이 없어, CLI·MCP로는 Slot content를
  지우거나 꺼낼 길이 없습니다. 그 입력(`plan_slot_content_delete`·`plan_slot_take_out`을 부르는 입력)은 command 입력 규칙
  작업으로 남깁니다. 이전에는 같은 요청이 투영을 깨뜨린 채 성공했으므로, 막힌 동작은 문서를 망가뜨리던 동작입니다.
- runtime 선택 삭제·끌어 내기 제스처가 Slot content를 만나면 `slot_content_owner`로 골라 `plan_slot_content_delete`·
  `plan_slot_take_out`으로 보내야 합니다. 그 연결은 UI 재설계 때 합니다(지금 runtime 선택 삭제는 Slot content가 섞이면
  거부됩니다).
- Detach 호출자는 연결 목록을 넘겨야 합니다. 빈 목록이면 이전 동작과 같습니다(값 반영 없음).
- command codec 버전 8 → 9. 옛 reader는 가이드 절을 담은 history journal 복원만 포기합니다(ADR-0079 규칙). 같은 시기에
  다른 작업이 버전 9를 쓰면 통합 때 번호를 맞춰야 합니다.

## Compatibility

- 파일 포맷 변경은 없습니다(문서 레코드·텍스트 원본·chunk 그대로).
- command codec 버전 9(결정 4.5). 가이드 기록이 없는 command bytes는 버전 8과 같습니다.
- 클립보드 bytes(`STELLAR-CLIP` v1)는 `insert-node` codec을 그대로 담으므로 가이드 기록이 있을 때만 끝 절이 더해집니다.
  절을 모르는 옛 앱은 trailing bytes로 거부합니다(조용히 잃지 않음).
- FFI·`stellar_ffi.h` 변경은 없습니다.
- 공개 Rust API 변경:
  - `InsertNode.node_guides`, `InstanceRestructure.insert_guides` 필드가 생겼습니다.
  - `plan_detach`가 `links` 인자를 받고, `DetachPlanError::Link`가 생겼습니다. `commands::detach::PropertyLink`가 생겼고
    runtime `canvas::instance::PropertyLink`는 그 다시 내보내기입니다. `plan_detach_instance`가 `links`를 받습니다.
  - `node_ops::slot_content_owner`, `detach::plan_slot_content_delete`, `detach::SlotContentDeleteError`가 생겼습니다.

## Performance

- `insert-node` 검증에 삽입 node 수에 선형인 투영 검사와 가이드 기록 검사가 더해집니다.
- `delete-subtree`·`move-node` 검증에 부모 종류 조회 하나가 더해집니다.
- Detach 계획은 연결 수 × 복제 node 수(선형 탐색)만큼 더합니다(드문 사용자 동작).

## Security

영향 없음.

## Migration

없음. 저장 문서 형식은 그대로입니다.
