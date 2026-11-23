# ADR-0126: 가이드·레이아웃 그리드와 View 변수·Variant의 문서 저장 연결

## Status

Accepted (2026-11-24).

- 계획 행 XII.10.19·XII.10.20의 남은 범위(문서 모델·command·저장·Semantic Diff·Merge 연결)와 XVI.11.1이 남긴 View
  변수·Variant 영속 형식을 다룹니다.
- ADR-0117 결정 2.4·2.5가 후속으로 미룬 연결을 이 ADR이 정합니다. ADR-0111 결정 3의 "View 변수·Variant는 영속 형식이
  없다"는 이 ADR로 바뀝니다.
- 보호 경로 `crates/core-types`(`reserved::guides`)와 `schemas/command-transaction.schema.json`(새 command 종류 이름)을 바꿉니다.

## Context

- 가이드와 레이아웃 그리드는 문서 데이터이고 소유자는 Frame node입니다(ADR-0117 결정 2). 값 모델·canonical bytes·텍스트 줄
  문법은 `stellar_document::guides`에 이미 있습니다. 문서 모델 필드, command, chunk·텍스트 원본, Semantic Diff·Merge 연결이
  없어 저장되지 않았습니다.
- View 변수·Variant(IX.7, `ViewVariantModel`)는 메모리 command와 canonical bytes만 있고 문서·텍스트 원본·chunk에 들어가지
  않아, 새 프로젝트 E2E(XVI.11.1)가 Variant를 만들고 다시 열 수 없었습니다.
- 사용자 방향(2026-11-23): 엔진·기능만 만들고 Rust test·CLI·MCP로 검증합니다. 셸 UI·FFI는 바꾸지 않습니다.
- Context(ADR-0099)는 문서 전체 단위라 별도 `context-set` chunk와 manifest 절을 두었습니다. 가이드는 Frame 하나, View 변수는
  View 하나에 붙는 국소 데이터입니다.

## Decision

### 1. 문서 레코드와 모델

1. `DocumentRecords`에 `node_guides: Vec<NodeGuides>`(Frame마다 하나, 빈 기록 없음, 소유 node 순)와
   `view_variants: Vec<ViewVariantModel>`(View마다 하나, 빈 model 없음, View 순)을 둡니다.
2. `DocumentModel`은 두 map을 StableId로 들고 있습니다. layout record(ADR-0086)와 같이 node·View를 지워도 map에는 남고,
   같은 id가 되돌아오면(삭제 undo) 다시 보입니다. 조회(`node_guides`, `view_variants`)와 영속 레코드(`to_records`)에는
   살아 있는 Frame·문서 정의에 있는 View의 것만 나옵니다. `set_node_guides`는 Frame이 아닌 소유자를, `set_view_variants`는
   없는 View를 거부합니다. 빈 값은 기록을 지웁니다.
3. validator(`guides::persist::check_records`)는 소유 node 없음·Frame이 아님·중복·빈 기록, View 없음·중복·빈 model을
   위반으로 냅니다. Variant override가 가리키는 scene node는 검사하지 않습니다(IX.7.8과 같이 orphan은 resolver가 적용하지
   않고 진단합니다).
4. 가이드 id는 소유 Frame 안에서 유일합니다. 문서 전체 영속 id 등록에는 넣지 않습니다(경로가 항상 소유 Frame을 root로 가지므로
   Diff·Merge 식별에 충분). View 변수·Variant id는 새로 만들 때 문서의 다른 영속 id와 겹치지 않아야 합니다.

### 2. chunk: View subtree 확장 절

1. 별도 chunk kind를 만들지 않고 **subtree chunk payload 끝의 확장 절**에 둡니다. 가이드는 소유 Frame이 있는 subtree,
   View model은 그 View root subtree입니다. Frame 하나를 바꾸면 그 subtree chunk만 새 hash가 되어(V.3.3) 국소 데이터와
   chunk 단위가 맞고, manifest·storage kind 표를 바꾸지 않습니다.
2. 형식: layout 절 자리 뒤에 tag 오름차순 절. 절 하나 = tag `u8` + record 수 `u32`(1 이상) + (`u32` 길이 + record bytes)*.
   tag는 core-types `reserved::guides`의 번호입니다.

| tag | 이름 | record bytes |
|---|---|---|
| `0x01` | `SUBTREE_SECTION_NODE_GUIDES` | `NodeGuides::encode`(record v1), 소유 node 순 |
| `0x02` | `SUBTREE_SECTION_VIEW_VARIANTS` | `ViewVariantModel::encode`, View 순 |

3. 확장 절이 있고 layout record가 없으면 layout 절 자리에 수 `0`을 씁니다. 확장 절이 없는 subtree는 이전 형식과 같은 bytes입니다.
   이 번호를 모르는 옛 reader는 수 `0`이나 남는 bytes를 형식 오류로 거부합니다(조용히 버리지 않음, ADR-0117 결정 2.5).
4. API: `SubtreeRecords` 값은 바꾸지 않고 확장 절을 `SubtreeSections`로 주고받습니다(`encode_with`, `decode_with_sections`,
   `DocumentRecords::view_subtrees_with_sections`, `from_subtrees_with_sections`). 확장 절을 모르는 `decode`·`view_subtrees`는
   확장 절이 있으면 거부합니다. 제품 저장·열기(`serialize`, `load`)는 확장 절 경로를 씁니다.
5. 읽을 때 View 절의 View는 subtree만으로 확인할 수 없으므로 받아 두고, 문서 조립 뒤 validator가 View 존재를 확인합니다.
   쓰기는 언제나 그 View의 root subtree입니다.

### 3. command (영속 command 대역)

| 번호 | 이름 | payload | 역 |
|---|---|---|---|
| `0x00B0` | `set-node-guides` | owner id + `GuideEdit::encode`(Add·Move·Remove·SetGrids) | `NodeGuides::apply`가 돌려준 역 편집 |
| `0x00B1` | `edit-view-variants` | View id + `view_variables::encode_command`(IX.7.8 `ViewVariantCommand` 7종) | 문서 crate가 돌려준 inverse command |

1. command codec version은 7입니다. `apply-external-snapshot`에 변경 tag 9(가이드 기록)·10(View model)을 더해 외부 원본의
   가이드·View 변수 변경도 외부 반영 command 하나로 들어오고 undo 한 번으로 되돌아갑니다. 그 종류·tag가 없는 command의 bytes는
   version 6과 같습니다.
2. 두 값은 layout 입력이 아니므로(ADR-0074, ADR-0117 결정 4.4) 무효화는 비어 있습니다. 변경 경로는 결정 5의 snapshot 경로와
   같습니다.
3. 잠금(D8) 판정은 다른 command와 같이 진입 쪽(`commands::lock`)이 합니다.
4. `restore-node-records`는 종류를 바꿀 때 남는 기록이 새 종류에 붙을 수 있어야 합니다: 가이드가 있는 Frame을 다른 종류로
   바꾸거나, 붙을 수 없는 layout longhand가 남는 복원은 거부합니다(새 종류 command가 더해지며 바뀐 command 순서 fuzz가 드러낸
   layout record 쪽 기존 결함도 같은 검사로 막습니다).

### 4. 텍스트 원본 v4

1. Frame node 항목의 `guides` 목록. 한 줄 = 가이드 하나 또는 그리드 하나이고 문법은 `NodeGuides::lines` 그대로입니다
   (가이드 id 순, 그 뒤 그리드 그리는 순). 읽을 때 같은 함수로 다시 쓴 줄과 같아야 합니다(정규형). 기록이 없으면 키가 없습니다.

```yaml
guides:
  - guide x 120px 0195c530-3acd-7bc1-8414-c213f61cb965
  - grid columns 12 auto gap 20px margin 32px stretch
```

2. View 파일 `view` mapping의 선택 키 `variables`·`variants`(단일 View 원본과 다중 파일 원본이 같은 함수
   `source_v4::view_data`를 씁니다).
   - 변수: `id`, `name`, `type`(Context 원본 타입 이름, 예: `enum(3329)`, `boolean?`), `default`(값 문장), enum이면 `allowed`.
     값 문장은 `null`, `enum(<타입>) <variant>`, token 원본과 같은 리터럴 `<타입> <값>`입니다.
   - Variant: `id`, `when`(`<변수 id> any` 또는 `<변수 id> = <값 문장>`), override가 있으면 `overrides`(`VariantOverrideSet`
     canonical bytes hex).
   - 값 문장으로 쓸 수 없는 값(gradient 등)이면 그 변수·Variant 전체를 `bytes`(canonical bytes hex)로 씁니다. 손실 저장은 없습니다.
3. 새 키는 선택 키라 schema_version은 `4` 그대로입니다. 키가 없으면 기록이 없는 것이고, 기존 파일은 그대로 읽습니다.

### 5. Semantic Diff·Merge

1. 가이드 하나 = 소유 Frame `Node` 객체의 `[0xED01] + 가이드 id` 속성(값 = 텍스트 원본 한 줄). 그리드 목록 = `[0xED02]` 속성
   하나(값 = 그리드 줄을 `\n`으로 이은 텍스트). View 변수 하나 = `View` 객체의 `[0xED10] + 변수 id` 속성(값 = 변수 canonical
   bytes). Variant 하나 = `Variant` 객체(parent = View)이고 override는 `add_variant`와 같은 patch별 속성, 조건은 `[0xED11]`
   속성입니다. 번호는 core-types `reserved::guides`의 `0xEDxx` 대역입니다.
2. 범주(`semantic_diff::guides::category`): `0xED01`·`0xED02`는 `guides`, `0xED10`·`0xED11`은 `view-variables`입니다.
3. Merge는 기존 속성 단위 3-way 규칙 그대로입니다: 다른 가이드 동시 편집은 다른 경로라 충돌 없음, 같은 가이드를 양쪽에서
   옮기거나 한쪽 이동·한쪽 삭제는 그 경로 충돌, 그리드 목록은 값 하나라 양쪽이 다르게 바꾸면 값 충돌(ADR-0117 결정 2.4).
4. 역투영(`document_records`)은 같은 속성에서 기록을 다시 만들고, 정규형이 아닌 줄·경로와 다른 id는 고쳐 넣지 않고 거부합니다.

### 6. CLI·MCP 조회

`stellar guides <경로> <문서 id> [--node <Frame id>]`와 MCP `document.guides`(읽기 전용)가 같은 ops 함수
`stellar_ops::guides::read_snapshot_guides`를 씁니다. 출력은 Frame별 가이드(id·축·CSS 길이 위치·줄)와 그리드(줄·표시·Frame
크기로 `LayoutGrid::tracks`가 계산한 트랙), View별 변수(이름·타입·값 문장)·Variant(조건 문장·patch 수)·진단입니다. CLI·MCP로
가이드를 **편집**하는 입력(`ops::mcp_command_input`)은 이 ADR 범위 밖이며, 앱·CLI 편집 연결 때 같은 command를 씁니다.

## Alternatives

- 별도 chunk kind(`guide-set`): Context처럼 manifest 절과 storage kind 표가 필요하고, Frame 하나의 가이드 편집이 문서 전체
  chunk를 다시 씁니다. 국소 데이터라 subtree 확장 절을 골랐습니다.
- `SubtreeRecords`에 필드 추가: commands crate 등 30곳이 넘는 struct literal을 바꿔야 하고, 삽입 command payload의 의미까지
  넓어집니다. 확장 절을 값 밖(`SubtreeSections`)으로 주고받고, 모르는 경로는 거부하게 했습니다.
- View model을 `ViewDefinition`·manifest에: 문서 정의 bytes와 manifest 형식을 바꿔야 하고 큰 override 묶음이 manifest에
  들어갑니다.
- 그리드를 그리드마다 속성으로: 그리드에는 영속 id가 없고 순서가 그리는 순서라 목록 전체가 값 하나입니다(ADR-0117 결정 2.3).
- View 변수·Variant를 `SetViewVariants`(model 전체 교체) command로: 단순하지만 IX.7.8 command의 정확한 inverse·orphan 규칙을
  다시 만들게 되어 문서 crate command를 그대로 감쌌습니다.

## Consequences

- 가이드·레이아웃 그리드와 View 변수·Variant가 문서 레코드·chunk·텍스트 원본·Semantic Diff·Merge·외부 변경 반영·history를
  같은 경로로 지납니다. CLI·MCP로 조회할 수 있습니다.
- Frame을 복제·붙여넣기(`DuplicateSubtree`, `Paste`)하면 가이드·그리드는 따라가지 않습니다(layout record와 같은 남은 범위).
- Variant override는 텍스트 원본에서 hex입니다. 사람이 읽는 patch 줄 문법은 Variant 편집 입력(CLI·MCP)과 함께 정합니다.
- 셸 overlay·눈금자·⌘R 연결은 UI 재설계 때 합니다.

## Compatibility

- 확장 절이 없는 문서의 chunk bytes·텍스트 원본 bytes는 그대로입니다. 확장 절을 쓴 chunk와 새 키를 쓴 텍스트 원본은 옛 앱이
  거부합니다(조용히 버리지 않음).
- command codec version 7. 옛 history reader는 새 종류·tag를 담은 journal의 복원만 포기합니다(ADR-0079).
- 보호 경로 `schemas/command-transaction.schema.json`의 canonical command 종류 목록에 `set-node-guides`·`edit-view-variants`가
  늘어납니다(`cargo xtask generate-command-schema` 생성 결과). 다른 schema 항목은 그대로입니다.
- MCP tool 하나(`document.guides`)와 CLI 명령 하나(`guides`)가 늘어납니다. FFI는 바꾸지 않습니다.

## Performance

- 저장·열기는 기록 수에 선형입니다. owner 하나당 가이드 1,024개·그리드 16개 상한(ADR-0117)이 chunk 크기를 유계로 둡니다.
- Semantic snapshot 속성은 가이드 수 + Frame당 그리드 1 + View 변수 수 + Variant patch 수만큼 늘어납니다.
- command 적용은 소유 Frame 하나의 기록 복제(최대 1,024 가이드)입니다.

## Security

- 모든 decoder는 길이·순서·정규형을 검사하고 남는 bytes를 거부합니다(확장 절 tag·record 수 0·정렬, 가이드 정규 줄, View
  model 재인코딩 비교). 비유한 수는 `NodeGuides` 검사에서 거부합니다.

## Migration

없습니다. 새 기록은 선택 값이고 기록이 없으면 가이드·View 변수가 없는 것입니다.
