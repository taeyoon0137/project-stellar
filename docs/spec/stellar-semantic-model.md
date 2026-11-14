# Stellar 의미 모델과 Git diff·merge 규칙 명세 — 구현 기준선

- 문서 종류: 규범(normative) 명세와 현재 구현 기록. 구현된 규칙을 코드에서 옮겨 적었습니다.
- WBS: XXVIII.1.3
- 기준 commit: `24728e3` 위의 구현. 이 문서의 모든 규칙은 근거로 적은 crate·module과 일치해야 합니다.
  코드와 이 문서가 다르면 이 문서가 틀린 것이며, 문서를 고칩니다.
- 공개 상태: 내부 명세입니다. 기술 아키텍처 §35의 공개 전 조건은 아직 충족되지 않았습니다.
- 관련 문서: 저장 형식(chunk envelope·manifest·DocumentManifest·ViewTree 바깥 틀)은
  [`stellar-format.md`](stellar-format.md)(이하 "형식 명세"), 보안 한도는
  [`stellar-format-security.md`](stellar-format-security.md), 버전 호환성은
  [`stellar-format-compatibility.md`](stellar-format-compatibility.md)에 있습니다.

## 0. 표기

규범 용어(**해야 합니다**, **하면 안 됩니다**, **할 수 있습니다**, **현재 구현**)와 바이트 표기는 형식 명세
§0과 같습니다. 이 문서에서 **현재 구현**은 규칙이 아니라 기준 commit에서 관찰한 동작이며, 재현한 항목은
그렇다고 적었습니다.

근거 표기의 crate 접두어:

| 접두어 | crate | 경로 |
|---|---|---|
| `core-types` | `stellar-core-types` | `crates/core-types` |
| `document` | `stellar-document` | `crates/document` |
| `semantic-diff` | `stellar-semantic-diff` | `crates/semantic-diff` |
| `semantic-merge` | `stellar-semantic-merge` | `crates/semantic-merge` |
| `git-porcelain` | `stellar-git-porcelain` | `crates/git-porcelain` |
| `project` | `stellar-project` | `crates/project` |

## 1. 범위

이 명세가 다루는 것:

1. 문서 의미 레코드의 저장 형식: node 레코드, `SubtreeRecords`, `DocumentDefinition`, `ModifierStack`과
   그 안의 numeric registry(§2). 형식 명세 §1이 이 문서로 미룬 부분입니다.
2. Semantic Snapshot 모델: 객체, 속성 경로, 값, 속성 key, 영속 레코드에서 snapshot을 만드는 규칙(§3).
3. Semantic Diff 규칙: 객체 대응, 위치·속성 비교, 변경 종류와 분류, 출처, 정렬, 적용과 역변경(§4).
4. snapshot·diff·merge 결과의 교환용 canonical 인코딩(§5).
5. 3-way Semantic Merge 규칙: 자동 병합, conflict 종류와 분류, 후처리, 결과 정렬(§6).
6. Git 연결 규칙: snapshot 원천, 경로 분류, unmerged stage 수집, conflict side 구성(§7).

다루지 않는 것:

- node payload 종류별 내부 bytes(Frame·Shape·Path·Text·Image). 각 payload의 encoder·decoder가 정합니다
  (`document::frame`, `document::shape`, `document::path`, `document::text`, `document::image`). 이
  문서는 payload를 감싸는 node 레코드와 종류 분기(§2.3)만 정합니다.
- 컴포넌트 library·token·asset record·expression AST·View Variant의 저장 형식. 기준 commit의 chunk model은
  이들을 저장하지 않습니다(§2.8).
- Semantic Status 분류(HEAD→Index Staged, Index→WorkingTree Unstaged), 선택 Staging, conflict 해결.
  구현되지 않았습니다(§7.6).
- 성능 정책. 포맷 규칙이 아닙니다.

## 2. 문서 의미 레코드 (저장 계층)

근거: `document::records`, `document::record_codec`, `document::definitions`, `document::modifier`,
`document::node_kind`, `document::validate`.

이 계층의 bytes는 형식 명세 §9의 DocumentManifest payload(`definition` 필드)와 subtree chunk payload에 그대로
들어갑니다. document crate는 storage에 의존하지 않으므로 같은 canonical 규칙(LE 고정 폭, `u32` 길이
프리픽스, UTF-8)을 각 module이 직접 구현합니다. runtime 핸들(`RuntimeEntityId`)은 레코드에 없으므로 이
형식에 기록될 수 없습니다.

### 2.1 NodeKind

`document::node_kind::NodeKind`의 `u16` 값은 저장 형식의 일부입니다(golden test
`golden_numeric_kinds_are_stable`).

| 값 | 종류 | 이름 | 레코드 저장 |
|---|---|---|---|
| 1 | Frame | `frame` | 예 |
| 2 | Fragment | `fragment` | 예 |
| 3 | Shape | `shape` | 예 |
| 4 | Path | `path` | 예 |
| 5 | Text | `text` | 예 |
| 6 | Image | `image` | 예 |
| 7 | ComponentInstance | `component-instance` | 아니오(§2.3) |

컴포넌트 정의는 scene node가 아니라 문서 resource입니다. scene에는 ComponentInstance만 배치됩니다.

### 2.2 node 레코드

`encode_node_record` / `decode_node_record`:

| 순서 | 필드 | 형식 |
|---|---|---|
| 1 | id | raw 16 byte StableId |
| 2 | kind | `u16` LE(§2.1) |
| 3 | parent | flag `u8`(`0` 없음, `1` 있음) + 있으면 raw 16 byte StableId |
| 4 | order key | `u32` LE 길이 + bytes |
| 5 | visible | `u8`, `0` 또는 `1` |
| 6 | locked | `u8`, `0` 또는 `1` |
| 7 | display name | `u32` LE 길이 + UTF-8 |
| 8 | payload | `u32` LE 길이 + 종류별 payload bytes(§2.3) |

decoder는 다음을 거부해야 합니다.

| 입력 | 오류 |
|---|---|
| 모르는 kind 값 | `RecordCodecError::UnknownNodeKind` |
| parent flag가 0·1이 아님 | `Malformed { reason: "parent flag" }` |
| order key가 비었거나 `00`으로 끝남(`OrderKey::from_bytes`) | `Malformed { reason: "잘못된 OrderKey" }` |
| visible·locked가 0·1이 아님 | `Malformed { reason: "bool은 0 또는 1" }` |
| display name이 UTF-8이 아님 | `Malformed { reason: "표시 이름 UTF-8 아님" }` |
| 입력 부족 | `Malformed { reason: "입력 부족" }` |
| 레코드 끝 뒤 남은 bytes | `Malformed { reason: "trailing bytes" }` |

- 표시 이름은 식별에 쓰지 않습니다. 같은 이름이라도 StableId가 다르면 다른 node입니다.
- node 종류는 payload 종류에서 유도합니다(`NodeRecord::kind`). 그래서 메모리 레코드에서는 kind와 payload가
  어긋날 수 없습니다.

### 2.3 payload 종류 분기

`encode_payload` / `decode_payload(kind, bytes)`:

| kind | decoder | 실패 시 |
|---|---|---|
| Frame | `FramePayload::decode` | `RecordCodecError::Frame(...)` |
| Fragment | `FragmentPayload::decode` — payload는 비어 있어야 함 | `Fragment(UnexpectedPayload { len })` |
| Shape | `ShapeGeometry::decode` | `Shape(...)` |
| Path | `IdentifiedPath::decode` | `Path(...)` |
| Text | `TextPayload::decode` | `Text(...)` |
| Image | `ImagePayload::decode` | `Image(...)` |
| ComponentInstance | 없음 | `UnsupportedNodeKind { kind }` |

- 각 payload decoder는 끝까지 정확히 소비하고 값 범위를 검증해야 합니다.
- **현재 구현**: `FramePayload::decode`와 `ModifierStack::decode`(Opacity)는 float 자리의 `-0.0` bit 패턴
  (`00000080`)을 거부하지 않고 `+0.0`으로 정규화해 받습니다(`FiniteF32::new`). 그래서 그 입력을 다시
  인코딩하면 bytes가 달라집니다. 두 경우를 재현했고, 다른 payload decoder는 확인하지 않았습니다. storage
  canonical decoder(형식 명세 §3.2, `InvalidFloat`)와 semantic codec(§5.1)은 같은 입력을 거부합니다.

### 2.4 SubtreeRecords

subtree chunk(형식 명세 §9.3, family 3) payload입니다.

```text
subtree = root:raw16
        ‖ node_count:u32 LE ‖ (len:u32 LE ‖ node_record[len])[node_count]
        ‖ stack_count:u32 LE ‖ (len:u32 LE ‖ modifier_stack[len])[stack_count]
```

- `nodes`는 root부터 pre-order이며, 형제는 `(OrderKey, StableId)` 오름차순입니다.
- `modifier_stacks`는 소유 node StableId 오름차순입니다.
- 각 node 레코드와 stack은 자기 길이 프리픽스 안을 정확히 소비해야 합니다.
- decoder는 첫 node가 선언된 root가 아니면 `SubtreeRootMismatch`, subtree에 없는 node의 stack이면
  `ForeignModifierStack`, 끝에 bytes가 남으면 `Malformed { reason: "trailing bytes" }`로 거부해야 합니다.
  node가 0개이면 비교 대상은 `StableId::NIL`입니다(root가 NIL이고 node가 없으면 통과합니다. 재현했습니다).
- 부모·순서·참조 등 문서 불변식은 decoder가 아니라 문서 validator(§2.7)가 검사합니다.
- **현재 구현**: decoder는 첫 node 외의 pre-order 순서와 stack의 소유 node 순서를 검사하지 않습니다. 다시
  합칠 때 정렬하므로(아래) root가 첫 node이고 나머지 순서만 다른 subtree bytes도 같은 레코드가 됩니다
  (재현했습니다).

View별 분할(`DocumentRecords::view_subtrees`):

- 정책은 View당 subtree 하나입니다. View 순서는 정의 순서(page 순서 → page 안 View 순서)이고, 각 subtree의
  root는 View의 root frame입니다.
- 어느 View root에서도 닿지 않는 node가 있으면 `PartitionError::UnreachableNodes`, 한 node가 두 subtree에
  나오거나 같은 id 레코드가 두 개면 `SharedNode`, 어느 subtree에도 속하지 않는 stack이나 같은 소유자의 stack이
  두 개면 `UnassignedModifierStacks`로 거부합니다. 데이터를 버리면서 분할하면 안 됩니다.
- 다시 합치기(`DocumentRecords::from_subtrees`)는 node를 StableId 순, stack을 소유 node StableId 순으로
  정렬합니다(`sort_canonical`).

### 2.5 DocumentDefinition

DocumentManifest payload의 `definition` 필드(형식 명세 §9.2)입니다.

```text
document = id:raw16 ‖ name:string ‖ page_count:u32 LE ‖ page[page_count]
page     = id:raw16 ‖ name:string ‖ view_count:u32 LE ‖ view[view_count]
view     = id:raw16 ‖ name:string ‖ root_frame:raw16
string   = len:u32 LE ‖ UTF-8
```

- 페이지와 View의 순서는 의미가 있습니다(표시 순서).
- decoder는 입력 부족, UTF-8 위반, 끝 잔여 bytes를 `DefinitionError::Malformed`로 거부해야 합니다.
- **현재 구현**: `DocumentDefinition::decode`는 형식만 검사합니다. 페이지·View id 중복과 root frame 공유는
  decode 단계에서 거부하지 않고(재현했습니다), 문서 validator(§2.7)가 검사합니다.
- `ProjectDocumentIndex` 인코딩(`count:u32 LE ‖ document*`)은 정의되어 있지만 기준 commit의 chunk model은
  이 bytes를 저장하지 않습니다(project crate는 메모리 index로만 만듭니다).

### 2.6 ModifierStack

node 하나의 modifier 목록입니다. node payload와 분리되어 subtree에 저장됩니다(§2.4).

```text
stack = node:raw16 ‖ count:u32 LE ‖ entry[count]
entry = id:raw16 ‖ key_len:u32 LE ‖ key[key_len] ‖ kind:u16 LE ‖ enabled:u8 ‖ payload_len:u32 LE ‖ payload[payload_len]
```

- entry는 OrderKey 오름차순으로 **엄격히 증가**해야 합니다(`Malformed { reason: "OrderKey 정렬 위반" }`).
  id 중복은 `DuplicateId`입니다.
- `enabled`가 0·1이 아니면 `Malformed { reason: "enabled flag" }`, 모르는 kind는
  `Malformed { reason: "알 수 없는 modifier kind" }`, 끝 잔여는 `Malformed { reason: "trailing bytes" }`입니다.
- 비활성 modifier는 평가에서 건너뛰지만 저장에는 남습니다.

`ModifierKind`(`u16`, golden test로 고정)와 payload:

| 값 | kind | payload bytes | 길이 | 값 검증 |
|---|---|---|---|---|
| 1 | `transform` | `f32` LE × 6(a, b, c, d, tx, ty) | 24 | 유한 |
| 2 | `boolean` | op `u8` ‖ count `u32` LE ‖ node id raw16 × count | 5 + 16n | op ∈ 0..=3 |
| 3 | `stroke-expand` | width `f32` | 4 | 비음수 |
| 4 | `corner` | radius `f32` | 4 | 비음수 |
| 5 | `mask` | source node id raw16 | 16 | — |
| 6 | `clip` | source node id raw16 | 16 | — |
| 7 | `fill` | Color 17 byte | 17 | Color 규칙 |
| 8 | `stroke` | Color 17 byte ‖ width `f32` | 21 | width 비음수 |
| 9 | `shadow` | Color 17 byte ‖ offset x `f32` ‖ offset y `f32` ‖ blur `f32` | 29 | blur 비음수 |
| 10 | `blur` | radius `f32` | 4 | 비음수 |
| 11 | `blend` | mode `u8` | 1 | mode ∈ 0..=5 |
| 12 | `opacity` | value `f32` | 4 | 0..=1 |

- Color 17 byte는 `core-types::color::Color::to_bytes`입니다: 색공간 tag `u8`(`01` sRGB, `02` Display P3) ‖
  r ‖ g ‖ b ‖ a(각 `f32` LE). alpha는 0..=1이어야 합니다.
- BooleanOp: `0` Union, `1` Subtract, `2` Intersect, `3` Exclude. BlendMode: `0` Normal, `1` Multiply,
  `2` Screen, `3` Overlay, `4` Darken, `5` Lighten.
- payload 길이가 kind의 길이와 다르면 `Malformed { reason: "payload 길이 불일치" }`, 모르는 op·mode는
  `Malformed`(`"알 수 없는 boolean op"`, `"알 수 없는 blend mode"`), 비유한 float·잘못된 색은
  `Malformed`(`"비유한 값"`, `"잘못된 색"`), 값 범위 위반(위 표의 비음수·0..=1)은 `InvalidPayload`입니다.
- 참조형(boolean·mask·clip)의 `StableId::NIL`은 미지정 대상입니다. 참조 의존성 목록에서 빠집니다
  (`ModifierPayload::referenced_nodes`).

### 2.7 문서 불변식

`document::validate::validate_document`가 저장 전·로드 후에 검사합니다. 위반은 모두 정확한 위치와 함께
보고합니다.

1. 문서·페이지·View·node·modifier·path point/handle id가 문서 안에서 유일하고 NIL이 아님.
2. 각 View의 root는 존재하는 부모 없는 Frame이고 View끼리 공유하지 않으며, 부모 없는 node는 어떤 View의
   root임.
3. 부모가 존재하고 자식을 가질 수 있는 종류(Frame·Fragment)이며 부모 chain에 cycle이 없음.
4. 같은 부모의 형제끼리 OrderKey가 다름.
5. 참조가 다른 종류 객체를 가리키지 않음(Frame clip은 Path node, modifier 참조는 node). 대상이 없는 참조는
   위반이 아니라 미해결 진단입니다.
6. stack 소유 node 존재·유일, 빈 stack 레코드 금지, Fragment의 modifier 금지, modifier 참조 cycle 금지.
7. node depth ≤ `MAX_NODE_DEPTH` = 64(View root = 0).

### 2.8 저장되지 않는 의미 객체

**현재 구현**: project crate의 문서 chunk model은 kind `document-manifest`와 `view-tree`만 씁니다
(`project::document_io::serialize`). 다음은 메모리 모델·encoder가 있어도 프로젝트 파일에 저장되지 않습니다.

| 객체 | 메모리·인코딩 근거 | 형식 명세 kind |
|---|---|---|
| 컴포넌트 library(정의, 정의 tree, Slot) | `document::component::definition::ComponentLibrary::encode` | `component-library`(등록만 됨) |
| ComponentInstance node | `document::component::instance::ComponentInstancePayload::encode` | 레코드 불가(§2.3) |
| token | `bindings::token`의 메모리 모델(`TokenSet`, `TokenDefinition`) | `token-set`(등록만 됨) |
| asset record | `assets::record`(형식 명세 §15) | `asset-metadata`(등록만 됨) |
| expression AST | `expressions::codec`(AST canonical bytes) | 없음 |
| View Variant override | `document::component::variant_override::VariantOverrideSet::encode` | 없음 |

Semantic Snapshot(§3)은 이 객체들도 표현할 수 있으며, 호출자가 별도 입력으로 넣습니다.

## 3. Semantic Snapshot 모델

근거: `semantic-diff::snapshot`, `semantic-diff::value`, `semantic-diff::keys`, `semantic-diff::build`,
`core-types::property`, `core-types::order_key`, 기술 아키텍처 §26.

Semantic Snapshot은 한 시점(HEAD·Index·Working Tree 등)의 프로젝트를 **영속 의미 단위 객체의 평평한 StableId
map**으로 펼친 것입니다. 저장 형식이 아닙니다. diff·merge의 입력이며, 교환이 필요할 때만 §5의 bytes를
씁니다.

### 3.1 원칙

- **runtime cache 제외**: RuntimeEntityId·평가 결과·layout·render cache를 담을 필드가 없습니다. 같은 영속
  레코드는 runtime 이력과 무관하게 같은 snapshot이 되어야 합니다(`record_order_and_runtime_history_do_not_matter`).
- **저장 배치 무관**: 객체는 StableId 순으로만 있고 chunk·파일 배치를 기록하지 않습니다. 레코드 순서나 chunk
  분할만 바뀌면 snapshot은 같아야 합니다.
- **식별**: 객체는 StableId로만 찾습니다. 이름·geometry 유사도를 쓰면 안 됩니다.
- **속성 소유**: 모든 속성 경로의 root는 소유 객체의 StableId여야 합니다.

### 3.2 객체

`SemanticObject`:

| 필드 | 형식 | 뜻 |
|---|---|---|
| `id` | StableId | 식별자. snapshot 안에서 유일 |
| `kind` | `ObjectKind` | 종류 |
| `origin` | `Origin` | 소속 범위 |
| `parent` | StableId 또는 없음 | 부모 객체 |
| `order_key` | OrderKey 또는 없음 | 형제 순서 |
| `properties` | `PropertyPath → SemanticValue` map | 경로 오름차순 |

`ObjectKind` 안정 code(`ObjectKind::code`, `u16`):

| code | 종류 |
|---|---|
| `0x0001` | Project |
| `0x0002` | Document |
| `0x0003` | Page |
| `0x0004` | View |
| `0x0100 \| NodeKind` | Node(NodeKind, §2.1) — 예: Frame `0x0101`, ComponentInstance `0x0107` |
| `0x0200 \| ModifierKind` | Modifier(ModifierKind, §2.6) — 예: Fill `0x0207` |
| `0x0301` | Component |
| `0x0302` | ComponentSlot |
| `0x0401` | Token |
| `0x0501` | Asset |
| `0x0601` | ExpressionNode |
| `0x0701` | Variant |

`ObjectKind::from_code`는 상위 byte가 `0x01`·`0x02`이면 하위 byte를 NodeKind·ModifierKind로 해석하고, 그 밖의
code는 위 표와 정확히 같아야 합니다. 모르는 code(`0x0000`, `0x0100`, `0x01FF`, `0xFFFF` 등)는 `None`입니다.

`Origin`:

| 값 | 뜻 | 소유 문서 |
|---|---|---|
| `Project` | 프로젝트 범위(metadata, 프로젝트 token, asset) | 없음 |
| `Document(document)` | 문서 scene·정의 범위 | `document` |
| `Component { document, component }` | 컴포넌트 정의 범위(정의 resource, 정의 tree node, Slot) | `document` |
| `ProjectComponent(component)` | 프로젝트 공용 컴포넌트 정의와 그 tree·Slot | 없음 |

두 객체가 같은 정체성이라는 것은 `id`, `kind`, `origin`이 모두 같다는 뜻입니다(`same_identity`).

### 3.3 속성 경로

`core-types::property::PropertyPath` = root StableId + `PropertyKey`(`u32`) 시퀀스 + 선택 tail(`Index(u32)` 또는
`Key(String)`).

- 순서는 (root, key 시퀀스 사전식, tail) 순입니다. tail은 없음 < `Index` < `Key`이고, `Index`는 숫자 순,
  `Key`는 문자열 byte 순입니다(derive `Ord`).
- `PropertyPath::to_canonical_bytes`는 BE 형식입니다: root 16 ‖ key 수 `u32` BE ‖ key `u32` BE* ‖ tail
  (`00` 없음, `01` ‖ index `u32` BE, `02` ‖ 길이 `u32` BE ‖ UTF-8). 이 bytes는 View Variant 속성 patch의 key
  segment(§3.6)에만 쓰입니다. §5의 교환 인코딩은 LE 형식을 따로 씁니다.
- key segment에 StableId를 쓸 때는 `StableId`의 표시 형식(소문자 hyphen UUID 36자)을 씁니다.

### 3.4 값

`SemanticValue`는 닫힌 집합이며 재귀 구조가 없습니다.

| tag | 종류 | 내용 |
|---|---|---|
| `0x01` | Bool | 불리언 |
| `0x02` | Uint | `u64` |
| `0x03` | Text | UTF-8 문자열 |
| `0x04` | Id | StableId 하나 |
| `0x05` | Ids | 순서 있는 StableId 목록 |
| `0x06` | Floats | 순서 있는 유한 `f32` 목록 |
| `0x07` | Typed | `core-types::typed_value::TypedValue` |
| `0x08` | Bytes | 불투명 canonical bytes |

- float은 `FiniteF32`로만 담습니다. NaN·±Infinity는 없고 `-0.0`은 `+0.0`으로 정규화되어 있습니다.
- 값 비교는 `canonical_eq`이며 canonical bytes(§5.2)가 같은 것과 정확히 같아야 합니다. float은 **bit
  단위**로 비교하며 epsilon을 쓰면 안 됩니다. bit가 다른 유한 float은 항상 다른 값입니다.
- 값 타입 `ValueType`은 (종류 tag, Typed이면 `TypedValue::type_tag`)입니다. 같은 종류의 다른 값은 같은
  타입입니다. merge의 type-change 분류(§6.4)에 씁니다.

### 3.5 속성 key

문서 crate가 정한 key는 그대로 재사용합니다. 문서 crate에 key가 없는 필드는 **semantic snapshot 전용 대역
`0xE000`~`0xEFFF`**에 둡니다. 이 대역은 저장 형식이 아니라 diff·merge 식별용이며, 문서 crate가 같은 필드의
공식 key를 정하면 그 key로 바꿉니다.

재사용 key:

| key | 이름 | 근거 |
|---|---|---|
| `0x0101` | frame size | `document::frame::keys::SIZE` |
| `0x0102` | frame transform | `TRANSFORM` |
| `0x0103` | frame clipping | `CLIPPING` |
| `0x0104` | frame background | `BACKGROUND` |
| `0x0105` | frame border | `BORDER` |
| `0x0106` | frame corner radius | `CORNER_RADIUS` |
| `0x0107` | frame layout mode | `LAYOUT_MODE` |
| `0x0601` | image asset 참조 | `document::image::IMAGE_PROP_ASSET` |
| `0x0A01` | instance 정의 참조 | `document::component::instance::keys::COMPONENT_REF` |
| `0x0A02` | instance Property override | `PROPERTY_OVERRIDES` |
| `0x0A03` | instance Slot override | `SLOT_OVERRIDES` |
| `0x1001` | modifier enabled | `document::modifier_metadata::keys::ENABLED` |
| `0x1011` | transform matrix | `TRANSFORM_MATRIX` |
| `0x1021`, `0x1022` | boolean op, operands | `BOOLEAN_OP`, `BOOLEAN_OPERANDS` |
| `0x1031` | stroke-expand width | `STROKE_EXPAND_WIDTH` |
| `0x1041` | corner radius | `CORNER_RADIUS` |
| `0x1051`, `0x1061` | mask source, clip source | `MASK_SOURCE`, `CLIP_SOURCE` |
| `0x1071` | fill color | `FILL_COLOR` |
| `0x1081`, `0x1082` | stroke color, width | `STROKE_COLOR`, `STROKE_WIDTH` |
| `0x1091`, `0x1092`, `0x1093` | shadow color, offset, blur | `SHADOW_COLOR`, `SHADOW_OFFSET`, `SHADOW_BLUR` |
| `0x10A1` | blur radius | `BLUR_RADIUS` |
| `0x10B1` | blend mode | `BLEND_MODE` |
| `0x10C1` | opacity value | `OPACITY_VALUE` |

snapshot 전용 key(`semantic-diff::keys`):

| key | 상수 | key | 상수 |
|---|---|---|---|
| `0xE001` | `VISIBLE` | `0xE601` | `IMAGE_SIZE` |
| `0xE002` | `LOCKED` | `0xE602` | `IMAGE_FIT` |
| `0xE003` | `DISPLAY_NAME` | `0xE603` | `IMAGE_CROP` |
| `0xE101` | `DOCUMENT_PAGES` | `0xE604` | `IMAGE_SAMPLING` |
| `0xE102` | `PAGE_VIEWS` | `0xE605` | `IMAGE_COLOR_SPACE` |
| `0xE103` | `VIEW_ROOT_FRAME` | `0xE701` | `COMPONENT_ROOT` |
| `0xE201` | `PROJECT_METADATA` | `0xE702` | `COMPONENT_PROPERTIES` |
| `0xE301` | `SHAPE_KIND` | `0xE703` | `COMPONENT_SLOTS` |
| `0xE311` | `RECT_SIZE` | `0xE704` | `COMPONENT_LOCAL_TOKENS` |
| `0xE312` | `RECT_CORNER_RADIUS` | `0xE711` | `SLOT_HOST` |
| `0xE321` | `ELLIPSE_BOUNDS` | `0xE712` | `SLOT_ALLOWED_KINDS` |
| `0xE331` | `LINE_START` | `0xE713` | `SLOT_MIN` |
| `0xE332` | `LINE_END` | `0xE714` | `SLOT_MAX` |
| `0xE333` | `LINE_CAP` | `0xE715` | `SLOT_DEFAULT_CONTENT` |
| `0xE341` | `POLYGON_BOUNDS` | `0xE801` | `TOKEN_NAME` |
| `0xE342` | `POLYGON_SIDES` | `0xE802` | `TOKEN_TYPE` |
| `0xE343` | `POLYGON_ROTATION` | `0xE803` | `TOKEN_VALUE` |
| `0xE344` | `POLYGON_CORNER_RADIUS` | `0xE804` | `TOKEN_EXPRESSION` |
| `0xE401` | `PATH_DATA` | `0xE901` | `ASSET_CONTENT_HASH` |
| `0xE402` | `TEXT_PAYLOAD` | `0xE902` | `ASSET_MEDIA_TYPE` |
| | | `0xEA01` | `EXPR_PAYLOAD` |
| | | `0xEA02` | `EXPR_CHILDREN` |
| | | `0xEB01` | `VARIANT_PROPERTY_PATCH` |
| | | `0xEB02` | `VARIANT_STRUCTURE_PATCH` |
| | | `0xEB03` | `VARIANT_SLOT_CHANGE` |

- 모든 key는 서로 달라야 합니다(`keys_are_unique`). 전용 key는 `0xE000`~`0xEFFF` 안에 있어야 합니다
  (`snapshot_only_keys_stay_in_reserved_band`).
- asset 참조 key 목록 `ASSET_REFERENCE_KEYS`는 `[0x0601]` 하나입니다. 경로의 첫 key가 이 목록에 있으면 asset
  참조 속성입니다.

### 3.6 snapshot 구성 규칙

근거: `semantic-diff::build::SnapshotBuilder`.

입력과 만들어지는 객체:

| 입력(`add_*`) | 객체 | Origin | parent | order key |
|---|---|---|---|---|
| `ProjectMetadata` | Project 1개 | Project | 없음 | 없음 |
| `DocumentRecords` | Document, Page, View | Document | Page → 문서, View → page | 없음 |
| `DocumentRecords` | scene Node | Document | 레코드 parent | 레코드 OrderKey |
| `DocumentRecords` | Modifier(stack 항목마다) | Document | 소유 node | 항목 OrderKey |
| `ComponentLibrary` | Component, 정의 tree Node, ComponentSlot | 문서 라이브러리는 Component, 프로젝트 라이브러리는 ProjectComponent | 정의 Node만 레코드 parent | 정의 Node만 OrderKey |
| `InstanceNode` | scene ComponentInstance Node | Document | 입력 parent | 입력 OrderKey |
| `TokenEntry` | Token | 문서가 없으면 Project, 있으면 Document | 없음 | 없음 |
| `AssetEntry` | Asset | Project | 없음 | 없음 |
| `ExpressionNodeEntry` | ExpressionNode(AST node마다) | 문서가 없으면 Project, 있으면 Document | owner | 없음 |
| `VariantEntry` | Variant | Document | owner | 없음 |

- 각 `add_*`는 원자적이어야 합니다. 같은 StableId가 이미 있거나 한 묶음 안에서 겹치면
  `SnapshotError::DuplicateObject`로 묶음 전체를 넣지 않습니다. 같은 경로 속성이 두 번이면
  `DuplicateProperty`(예: metadata 이름 중복)입니다.
- 레코드 순서와 builder 호출 순서는 결과에 영향을 주면 안 됩니다.

속성 펼치기. 경로는 별도 표시가 없으면 `[key]` 하나입니다.

| 객체 | 경로 | 값 | 조건 |
|---|---|---|---|
| Project | `[PROJECT_METADATA]` + Key(항목 이름) | Text(값) | 항목마다 |
| Document | `DISPLAY_NAME` | Text | |
| Document | `DOCUMENT_PAGES` | Ids(page id, 정의 순서) | |
| Page | `DISPLAY_NAME`, `PAGE_VIEWS` | Text, Ids(View id, 정의 순서) | |
| View | `DISPLAY_NAME`, `VIEW_ROOT_FRAME` | Text, Id | |
| 모든 Node | `VISIBLE`, `LOCKED`, `DISPLAY_NAME` | Bool, Bool, Text | |
| Frame | `frame::SIZE` | Floats[width, height] | |
| Frame | `frame::TRANSFORM` | Floats[a, b, c, d, tx, ty] | |
| Frame | `frame::CLIPPING` | Uint(0 disabled, 1 frame bounds) 또는 Id(clip path node) | |
| Frame | `frame::BACKGROUND` | Typed(Color) | 배경이 있을 때 |
| Frame | `[frame::BORDER]` + Index(0), Index(1) | Typed(Color), Floats[thickness] | border가 있을 때 |
| Frame | `frame::CORNER_RADIUS` | Floats[tl, tr, br, bl] | |
| Frame | `frame::LAYOUT_MODE` | Id | layout mode가 있을 때 |
| Fragment | 없음 | — | |
| Shape | `SHAPE_KIND` | Uint(`ShapeKind::tag`) | |
| Shape Rectangle | `RECT_SIZE`, `RECT_CORNER_RADIUS` | Floats[w, h], Floats[tl, tr, br, bl] | |
| Shape Ellipse | `ELLIPSE_BOUNDS` | Floats[x, y, w, h] | |
| Shape Line | `LINE_START`, `LINE_END`, `LINE_CAP` | Floats[x, y], Floats[x, y], Uint(0 butt, 1 round, 2 square) | |
| Shape Polygon | `POLYGON_BOUNDS`, `POLYGON_SIDES`, `POLYGON_ROTATION`, `POLYGON_CORNER_RADIUS` | Floats[x, y, w, h], Uint, Floats[도], Floats[r] | |
| Path | `PATH_DATA` | Bytes(payload 전체 encoding) | |
| Text | `TEXT_PAYLOAD` | Bytes(payload 전체 encoding) | |
| Image | `IMAGE_ASSET`(`0x0601`) | Id(asset) | |
| Image | `IMAGE_SIZE`, `IMAGE_FIT`, `IMAGE_CROP`, `IMAGE_SAMPLING` | Floats[w, h], Uint(0 fill, 1 contain, 2 cover, 3 none), Floats[4], Uint(0 linear, 1 nearest) | |
| Image | `IMAGE_COLOR_SPACE` | Uint(`ColorSpace::tag`) | override가 있을 때 |
| ComponentInstance | `instance::COMPONENT_REF` | Id(정의) | |
| ComponentInstance | `[instance::PROPERTY_OVERRIDES]` + Key(override segment) | Typed(override 값) | override마다 |
| ComponentInstance | `[instance::SLOT_OVERRIDES]` + Key(slot StableId) | Ids(content) | slot override마다 |
| Modifier | `modifier::ENABLED` | Bool | |
| Modifier | kind별 key(§3.5 재사용 key) | Floats·Uint·Ids·Id·Typed(Color) | payload 필드마다 |
| Component | `DISPLAY_NAME`, `COMPONENT_ROOT` | Text, Id | |
| Component | `COMPONENT_PROPERTIES`, `COMPONENT_SLOTS`, `COMPONENT_LOCAL_TOKENS` | Ids(정의 순서) | |
| ComponentSlot | `SLOT_HOST`, `SLOT_MIN`, `SLOT_DEFAULT_CONTENT` | Id, Uint, Ids | |
| ComponentSlot | `SLOT_ALLOWED_KINDS` | Bytes(NodeKind `u16` LE, 오름차순) | |
| ComponentSlot | `SLOT_MAX` | Uint | 상한이 있을 때 |
| Token | `TOKEN_NAME`, `TOKEN_TYPE` | Text, Uint | |
| Token | `TOKEN_VALUE` / `TOKEN_EXPRESSION` | Typed / Id(AST root) | 있을 때 |
| Asset | `ASSET_CONTENT_HASH`, `ASSET_MEDIA_TYPE` | Bytes, Text | |
| ExpressionNode | `EXPR_PAYLOAD` | Bytes(호출자가 준 canonical payload) | |
| ExpressionNode | `EXPR_CHILDREN` | Bytes(`u32` LE 개수 ‖ (slot code `u32` LE ‖ raw16 id)*, slot 순서) | |
| Variant | `[VARIANT_PROPERTY_PATCH]` + Key(대상 경로 `to_canonical_bytes`의 소문자 hex) | Typed | patch마다 |
| Variant | `[VARIANT_STRUCTURE_PATCH]` + Key(node StableId) | Bytes(그 항목 하나만 담은 `VariantOverrideSet::encode`) | patch마다 |
| Variant | `[VARIANT_SLOT_CHANGE]` + Key(`인스턴스 StableId/slot StableId`) | Bytes(그 항목 하나만 담은 `VariantOverrideSet::encode`) | slot change마다 |

- override segment는 `OverrideKey` 경로의 인스턴스 StableId를 순서대로 각각 뒤에 `/`를 붙여 잇고, 마지막에
  Property StableId를 붙인 문자열입니다(`override_segment`).
- 컴포넌트 정의 tree node의 payload는 저장된 canonical bytes를 해석해 scene node와 같은 규칙으로 펼칩니다.
  ComponentInstance 정의 node는 `ComponentInstancePayload::decode`, 그 밖은 `decode_payload`를 씁니다. 해석
  실패는 `SnapshotError::DefinitionPayload`입니다.
- Path와 Text는 내부 offset·point id가 서로 묶여 있어 payload 전체를 값 하나로 둡니다. 그 밖의 payload는
  필드별 속성으로 나눠 같은 객체의 다른 필드 변경이 따로 비교·병합되게 합니다.
- expression AST는 node마다 객체 하나이고, Variant는 patch 항목마다 속성 하나입니다. operator 하나·patch
  하나의 변경은 변경 하나가 되어야 합니다(`ast_operator_change_is_one_node_change`,
  `variant_patch_change_is_one_entry_change`).

### 3.7 snapshot 연산

- `children(parent)`: parent가 같은 객체를 `(order_key, StableId)` 오름차순으로 돌려줍니다. order key가 없는
  객체가 먼저입니다.
- `asset_references()`: 첫 key가 asset 참조 key이고 값이 Id인 속성을 모아 asset StableId → 경로 집합으로
  돌려줍니다.
- 오류(`SnapshotError`): `DuplicateObject`, `PropertyRootMismatch`, `DuplicateProperty`, `NonFiniteValue`,
  `DefinitionPayload`.

## 4. Semantic Diff 규칙

근거: `semantic-diff::diff`, `semantic-diff::change`, `semantic-diff::apply`.

`diff(base, target)`은 base에서 target으로 가는 변경 목록 `SemanticDiff`를 돌려줍니다. 두 snapshot을 StableId
오름차순으로 한 번 merge-join하므로 비용은 객체·속성 수에 선형입니다.

### 4.1 객체 대응

1. base에만 있는 객체는 삭제(RemoveObject, token이면 RemoveToken)입니다. 변경에 삭제 전 객체 전체를 담습니다.
2. target에만 있는 객체는 추가(AddObject, token이면 AddToken)입니다. 변경에 추가된 객체 전체를 담습니다.
3. 같은 StableId인데 정체성(§3.2: kind 또는 origin)이 다르면 삭제(base 객체) + 추가(target 객체)입니다.
   속성 단위로 비교하면 안 됩니다. kind가 같고 origin만 달라도 교체입니다(재현했습니다).
4. 같은 정체성이면 위치(§4.2)와 속성(§4.3)을 비교합니다.

이름·geometry·내용 유사도로 rename이나 이동을 추정하면 안 됩니다. 같은 이름·같은 모양이라도 StableId가
다르면 삭제 + 추가입니다.

### 4.2 위치

- parent가 다르면 MoveObject입니다(새 OrderKey 포함).
- parent가 같고 OrderKey가 다르면 ReorderObject입니다.
- 위치 변경은 `from`(base 위치)과 `to`(target 위치)를 담습니다.
- 순서는 OrderKey의 byte 사전순입니다. 레코드 배열 순서·chunk·파일 배치는 snapshot에 없으므로 저장 chunk
  이동만 있으면 변경이 0이어야 합니다(`rechunk_only_change_has_zero_semantic_diff`).

### 4.3 속성

- 두 속성 map을 경로 오름차순으로 merge-join해, 값이 다른 경로마다 변경 하나를 냅니다.
- 한쪽에만 있는 경로는 `None` ↔ 값 변경입니다(`old` 또는 `new`가 `None`).
- 값 비교는 `canonical_eq`(§3.4)입니다.

### 4.4 변경 종류

`SemanticChangeKind`(안정 code `u8`, 진단 이름):

| code | 종류 | 이름 | 단위 | 담는 내용 |
|---|---|---|---|---|
| 1 | AddObject | `add-object` | 객체 | 추가된 객체 전체 |
| 2 | RemoveObject | `remove-object` | 객체 | 삭제 전 객체 전체 |
| 3 | MoveObject | `move-object` | 객체 | 대상 id, from, to |
| 4 | ReorderObject | `reorder-object` | 객체 | 대상 id, from, to |
| 5 | SetProperty | `set-property` | 속성 | 경로, old, new |
| 6 | AddToken | `add-token` | 객체 | 추가된 token 전체 |
| 7 | RemoveToken | `remove-token` | 객체 | 삭제 전 token 전체 |
| 8 | ChangeToken | `change-token` | 속성 | 경로, old, new |
| 9 | ComponentInterfaceChange | `component-interface-change` | 속성 | 경로, old, new |
| 10 | AssetRefChange | `asset-ref-change` | 속성 | 경로, old, new |

속성 변경 종류는 대상 객체 종류와 경로의 첫 key로 정해야 합니다(`SemanticChangeKind::property_for`). 위에서
먼저 맞는 규칙을 씁니다.

1. 객체가 Token이면 ChangeToken.
2. 객체가 ComponentSlot이면(경로 무관), 또는 Component이고 첫 key가 `COMPONENT_PROPERTIES`·`COMPONENT_SLOTS`이면
   ComponentInterfaceChange. Component의 `DISPLAY_NAME`·`COMPONENT_ROOT`·`COMPONENT_LOCAL_TOKENS`는 이 종류가
   아닙니다.
3. 첫 key가 asset 참조 key(§3.5)이면 AssetRefChange.
4. 그 밖은 SetProperty.

모든 변경은 이전·이후 상태를 함께 담습니다. 그래서 적용 전에 base 상태를 검증할 수 있고(§4.7) 역변경을 정보
손실 없이 만들 수 있습니다(§4.8).

### 4.5 출처

`Provenance`(안정 code `u8`):

| code | 출처 | 담는 id |
|---|---|---|
| 1 | Project | 없음 |
| 2 | Document | document |
| 3 | ComponentDefinition | document, component |
| 4 | InstanceOverride | document, instance, component |
| 5 | ProjectComponentDefinition | component |

- 객체의 기본 출처는 Origin에서 유도합니다: Project → Project, Document → Document,
  Component → ComponentDefinition, ProjectComponent → ProjectComponentDefinition(`object_provenance`). 추가·삭제는 그 객체, 위치·속성 변경은 target 객체의
  Origin을 씁니다.
- 속성 변경의 출처(`property_provenance`): target 객체가 Origin Document의 ComponentInstance Node이고 경로의 첫
  key가 `PROPERTY_OVERRIDES`·`SLOT_OVERRIDES`이면 InstanceOverride입니다. component는 target의
  `COMPONENT_REF` 값, 없으면 base의 값입니다. 둘 다 없으면 기본 출처입니다. `COMPONENT_REF` 교체와 공통
  필드는 scene 변경(Document)입니다. 컴포넌트 정의 tree 안의 nested instance override는 정의의 일부이므로
  ComponentDefinition입니다.
- 출처는 표시 정보입니다. 적용 결과(§4.7)에 영향을 주면 안 됩니다.

### 4.6 정렬

`SemanticDiff`는 항상 `canonical_order`로 정렬되어 있어야 합니다. 정렬 key는 순서대로:

1. 소유 문서(`provenance.document()`) — 프로젝트 범위(`None`)가 먼저.
2. 대상 객체 StableId.
3. 속성 경로 — 객체 단위 변경(`None`)이 먼저, 그 다음 경로 순서(§3.3).
4. 변경 종류 code.
5. 남은 동률은 변경 canonical bytes(§5.3) 사전순.

객체 경로에는 diff 양쪽에서 바뀔 수 있는 tree 경로 대신 불변 StableId를 씁니다. 같은 변경 집합은 만드는 방법
(입력 레코드 순서, builder 호출 순서, 목록 순서)과 무관하게 같은 순서·같은 bytes가 되어야 합니다
(`diff_bytes_ignore_input_order`). 같은 객체의 교체는 이 순서에 따라 정렬됩니다(같은 문서면 AddObject(1)이
RemoveObject(2)보다 먼저).

### 4.7 적용

`apply(base, changes)`는 새 snapshot을 돌려줍니다. 실패하면 아무것도 돌려주지 않고 base는 바뀌지 않아야
합니다. `apply(base, diff(base, target)) == target`이어야 합니다(`apply_diff_reproduces_target_property`).

목록 검증(base와 무관):

| 조건 | 오류 |
|---|---|
| 삭제·추가 종류가 객체 종류와 안 맞음(token인데 RemoveObject 등) | `InvalidChange` |
| MoveObject인데 parent가 같음, ReorderObject인데 parent가 다르거나 OrderKey가 같음 | `InvalidChange` |
| 속성 변경의 old·new가 둘 다 없거나 canonical로 같음(효과 없음) | `InvalidChange` |
| 같은 객체의 삭제를 두 번, 추가를 두 번, 위치를 두 번 다룸 | `DuplicateTarget { path: None }` |
| 같은 속성 경로를 두 번 다룸 | `DuplicateTarget { path: Some(...) }` |

같은 객체의 삭제 하나와 추가 하나(교체)는 허용합니다.

단계별 적용(목록 순서와 무관):

| 단계 | 규칙 | 오류 |
|---|---|---|
| 1 삭제 | 객체가 있어야 하고 기록된 객체 전체와 같아야 함 | `MissingObject`, `ObjectMismatch` |
| 2 추가 | 같은 id가 없어야 하고 모든 속성 root가 그 객체여야 함 | `ObjectExists`, `InvalidChange` |
| 3 위치 | 객체가 있어야 하고 현재 위치가 `from`과 같아야 함 | `MissingObject`, `PositionMismatch` |
| 4 속성 | 객체가 있어야 하고, `property_for(객체 종류, 경로)`가 변경 종류와 같아야 하며, 현재 값이 `old`와 canonical로 같아야 함(없음 = `None`) | `MissingObject`, `InvalidChange`, `PropertyMismatch` |

기록된 이전 상태와 base가 다르면 오래된(stale) diff로 보고 거부합니다. `new`가 `None`이면 속성을 지웁니다.

### 4.8 역변경

`SemanticChange::inverse`: 추가 ↔ 삭제(token 종류 포함), 위치 `from` ↔ `to`, 속성 `old` ↔ `new`. 종류 분류와
출처는 유지됩니다.

- `change.inverse().inverse() == change`.
- `inverse(changes)`는 canonical 순서로 다시 정렬한 `SemanticDiff`입니다.
- `apply(target, inverse(diff(base, target))) == base`여야 합니다(`apply_inverse_restores_base`).

## 5. 교환용 canonical 인코딩

근거: `semantic-diff::codec`, `semantic-merge::model`.

이 bytes는 프로젝트 파일이 아니라 diff·merge 결과를 결정론적으로 비교·전달하기 위한 형식입니다. Semantic
Snapshot은 저장하지 않습니다.

### 5.1 공통

- 정수는 LE, 길이·개수는 `u32`입니다. flag는 `u8` `0`·`1`만 허용합니다.
- decoder는 신뢰하지 않는 입력을 받습니다. panic하면 안 되고, 개수 필드로 미리 할당하면 안 됩니다. 개수
  × 항목 최소 크기가 남은 bytes보다 크면 `Truncated`입니다. 항목 최소 크기는 key 4, Ids 16, Floats 4, 객체
  속성 23, snapshot 객체 25, 변경 2, conflict 22 byte입니다.
- 비정규 값은 거부해야 합니다: float은 비유한이거나 `-0.0`처럼 정규화되는 bit 패턴이면, Typed는 다시
  인코딩한 bytes가 입력과 다르면 `InvalidValue`입니다.

### 5.2 기본 요소

| 요소 | 형식 |
|---|---|
| 값 | tag `u8`(§3.4) ‖ Bool `u8` 0/1 \| Uint `u64` \| Text `u32` 길이 ‖ UTF-8 \| Id raw16 \| Ids `u32` 개수 ‖ raw16* \| Floats `u32` 개수 ‖ `f32`* \| Typed `u32` 길이 ‖ `TypedValue::to_bytes` \| Bytes `u32` 길이 ‖ bytes |
| 선택 값 | flag ‖ 값 |
| 속성 경로 | root raw16 ‖ key 수 `u32` ‖ key `u32`* ‖ tail tag `u8`(`0` 없음, `1` index, `2` key) ‖ (index `u32` \| `u32` 길이 ‖ UTF-8) |
| OrderKey | `u32` 길이 ‖ bytes. 비었거나 `00`으로 끝나면 `InvalidValue { field: "order_key" }` |
| 선택 id, 선택 OrderKey | flag ‖ 내용 |
| Origin | `1` Project \| `2` ‖ document \| `3` ‖ document ‖ component \| `4` ‖ component |
| 객체 | id raw16 ‖ kind code `u16`(§3.2) ‖ Origin ‖ 선택 parent ‖ 선택 OrderKey ‖ 속성 수 `u32` ‖ (경로 ‖ 값)* |
| snapshot | `SSNP` ‖ version `u16` = 1 ‖ 객체 수 `u32` ‖ 객체* |

- 객체의 속성 경로는 엄격히 증가해야 하고 root가 그 객체여야 합니다(`NonCanonicalOrder { field:
  "object.properties" }`, `InvalidValue { field: "object.property_root" }`).
- snapshot의 객체는 StableId 순으로 엄격히 증가해야 합니다(`NonCanonicalOrder { field: "snapshot.objects" }`).

예(golden test `canonical_bytes_golden`): Uint `0x0102` → `02 0201000000000000`, Text `"ab"` → `03 02000000 6162`,
Floats `[1.0]` → `06 01000000 0000803f`, Bool true → `01 01`.

### 5.3 변경과 diff

| 요소 | 형식 |
|---|---|
| 출처 | code `u8`(§4.5) ‖ 종류별 id(2: document, 3: document ‖ component, 4: document ‖ instance ‖ component, 5: component) |
| 위치 | 선택 parent ‖ 선택 OrderKey |
| 변경 | kind code `u8`(§4.4) ‖ 출처 ‖ 본문 |
| 본문: 추가·삭제(1, 2, 6, 7) | 객체 |
| 본문: 위치(3, 4) | id raw16 ‖ from 위치 ‖ to 위치 |
| 본문: 속성(5, 8, 9, 10) | 경로 ‖ 선택 old ‖ 선택 new |
| diff | `SDIF` ‖ version `u16` = 1 ‖ 변경 수 `u32` ‖ 변경* |

`SemanticDiff::from_canonical_bytes`는 변경이 `canonical_order`로 **엄격히 증가**하지 않으면(같은 변경 반복 포함)
`NonCanonicalOrder { field: "diff.changes" }`로 거부해야 합니다.

### 5.4 merge 입력·결과

| 요소 | 형식 |
|---|---|
| MergeInput | `SMIN` ‖ version `u16` = 1 ‖ snapshot(base) ‖ snapshot(ours) ‖ snapshot(theirs) |
| MergeResult | `SMRS` ‖ version `u16` = 1 ‖ snapshot(merged) ‖ 변경 수 `u32` ‖ 변경* ‖ conflict 수 `u32` ‖ conflict* |
| conflict | kind `u8`(§6.4) ‖ 출처 ‖ 객체 id raw16 ‖ 대상 ‖ 상태(base) ‖ 상태(ours) ‖ 상태(theirs) |
| 대상 | `1` 객체 \| `2` 위치 \| `3` ‖ 속성 경로(root = 객체 id) |
| 상태 | `0` 없음 \| `1` ‖ 객체 \| `2` ‖ 위치 \| `3` ‖ 선택 값 |

- conflict 속성 경로의 root가 conflict 객체와 다르면 `InvalidValue { field: "conflict.path_root" }`입니다.
- MergeResult의 변경 목록은 canonical 순서로 정렬한 결과와 같아야 합니다(`NonCanonicalOrder { field:
  "merge.changes" }`).
- **현재 구현**: MergeResult decoder는 정렬한 결과와 같은지만 보므로 같은 변경의 반복을 받습니다. conflict
  순서(§6.9)와 merged·변경 목록의 일치도 검사하지 않습니다. 세 가지 모두 재현했습니다(빈 merged에 같은 변경
  두 개인 결과, conflict를 뒤집은 결과가 decode됨).

### 5.5 decoder 오류

`DecodeError`: `Truncated`, `TrailingBytes { remaining }`, `BadMagic`, `UnsupportedVersion { version }`,
`UnknownTag { field, value }`(모르는 tag·code, 0·1이 아닌 flag), `InvalidValue { field }`,
`NonCanonicalOrder { field }`. 최상위 decoder는 끝 잔여 bytes를 `TrailingBytes`로 거부해야 합니다.

## 6. 3-way Semantic Merge 규칙

근거: `semantic-merge::merge`, `semantic-merge::model`, `core-types::order_key_merge_fixtures`, 기술 아키텍처
§27.1.

### 6.1 입력과 결과

- 입력 `MergeInput { base, ours, theirs }`: 공통 조상과 양쪽의 Semantic Snapshot.
- 결과 `MergeResult`:
  - `merged`: 자동 병합한 candidate snapshot. conflict 대상은 base 상태를 유지해야 합니다.
  - `changes`: `diff(base, merged)`(canonical 순서).
  - `conflicts`: 자동 병합하지 못한 `SemanticConflict` 목록. 비어 있으면 clean입니다(`is_clean`).
- conflict는 base·ours·theirs 세 상태를 모두 기록해야 합니다. 해결 단계는 이 기록만으로 선택지를 만듭니다.

### 6.2 대상 단위

`diff(base, ours)`와 `diff(base, theirs)`를 객체 StableId별로 묶고, 객체 안에서 세 대상으로 나눕니다.

| 대상 | 변경 |
|---|---|
| 구조 | AddObject, RemoveObject, AddToken, RemoveToken(교체면 추가·삭제 둘 다) |
| 위치 | MoveObject, ReorderObject |
| 속성 | 속성 경로별 SetProperty, ChangeToken, ComponentInterfaceChange, AssetRefChange |

한쪽 묶음의 "삭제만"은 구조 변경이 있고 모두 RemoveObject·RemoveToken인 경우입니다.

### 6.3 객체별 판정

한쪽만 변경한 객체는 그 쪽 변경을 모두 받아들입니다. 양쪽이 모두 변경한 객체는 다음 순서로 판정해야 합니다.

1. 양쪽 구조 변경이 있고 서로 같으면(변경 값 전체가 같음) 한 번만 받아들입니다. 같은 삭제, 같은 추가, 같은
   교체입니다. 구조 변경이 있는 객체에는 다른 대상의 변경이 없습니다(§4.1).
2. 한쪽이라도 구조 변경이 있으면 객체 conflict 하나를 만들고 그 객체의 양쪽 변경은 하나도 받아들이지 않습니다.
   종류는 §6.4의 구조 판정입니다.
3. 위치: 한쪽만 바꿨으면 받아들입니다. 양쪽의 `to`가 같으면 한 번 받아들입니다. 다르면 conflicting-reorder
   (대상 위치)입니다.
4. 속성: 두 쪽 경로의 합집합을 경로 순으로 봅니다. 한쪽만 바꾼 경로는 받아들입니다. 양쪽의 `new`가 canonical로
   같으면(둘 다 삭제 포함) ours 변경 하나를 받아들입니다. 다르면 속성 conflict(대상 속성)입니다.

위치와 속성, 서로 다른 속성 경로는 서로 겹치지 않습니다. 받아들인 변경 목록은 semantic-diff `apply`(§4.7)로
base에 적용해 candidate를 만듭니다.

### 6.4 conflict 종류와 분류

`ConflictKind`(안정 code `u8`):

| code | 종류 | 이름 | 대상 |
|---|---|---|---|
| 1 | PropertyValue | `property-value` | 속성 |
| 2 | DeleteVsModify | `delete-vs-modify` | 객체 |
| 3 | DeleteVsMove | `delete-vs-move` | 객체 |
| 4 | TypeChangeVsValueChange | `type-change-vs-value-change` | 속성 |
| 5 | ConflictingReorder | `conflicting-reorder` | 위치 |
| 6 | InconsistentDefinition | `inconsistent-definition` | 객체 |

구조 판정(§6.3 2단계):

| ours | theirs | 종류 |
|---|---|---|
| 삭제만 | 삭제만이 아님 | theirs가 위치만 바꿨으면 DeleteVsMove, 그 밖(속성 변경, 교체 등)은 DeleteVsModify |
| 삭제만이 아님 | 삭제만 | ours 기준으로 같은 규칙 |
| 그 밖 | 그 밖 | InconsistentDefinition(서로 다른 추가, 서로 다른 교체, 교체 대 수정) |

속성 판정(§6.3 4단계): 양쪽 변경이 모두 old·new 값을 가지고, 한쪽은 값 타입(§3.4 `ValueType`)을 바꾸고 다른
쪽은 같은 타입 안에서 값만 바꿨으면 TypeChangeVsValueChange입니다. 그 밖(양쪽 모두 타입 변경, 한쪽이 속성
추가·삭제)은 PropertyValue입니다.

### 6.5 conflict 기록

| 대상 | 출처 | base·ours·theirs 상태 |
|---|---|---|
| 객체 | base 객체(없으면 ours, theirs 순)의 기본 출처 | 각 snapshot의 객체 전체 또는 `Missing` |
| 위치 | base 객체(없으면 ours)의 기본 출처 | 각 snapshot의 위치 또는 `Missing` |
| 속성 | ours 속성 변경의 출처 | 각 snapshot의 값(`Value(None)` = 속성 없음) 또는 객체가 없으면 `Missing` |

`SemanticConflict::property_values()`는 대상이 속성이고 세 상태가 모두 값일 때만 세 값을 돌려줍니다.

**현재 구현**: 기록할 객체가 base·ours(위치 conflict)나 base·ours·theirs(객체 conflict)에 없으면 conflict를
기록하지 않고 넘어갑니다(`object_conflict`, `position_conflict`). 위치 conflict는 theirs에만 있는 객체를 기록하지
않습니다.

### 6.6 후처리 1 — 고아 방지

candidate를 만든 뒤, parent가 candidate에 없는 객체를 찾습니다. 그 객체를 그 parent 아래에 둔 쪽 snapshot에
parent가 있던 경우만 merge가 만든 고아로 봅니다. 입력에 이미 있던 고아는 건드리지 않습니다.

- 고아가 있으면 그 parent의 삭제(RemoveObject·RemoveToken)를 받아들인 목록에서 빼고, parent에 객체 conflict를
  추가한 뒤 다시 적용합니다. 고아가 base에 있던 객체(다른 쪽이 옮겨 온 객체)면 DeleteVsMove, 새로 추가된
  객체면 DeleteVsModify입니다.
- 뺄 삭제가 없으면 멈춥니다. 매번 받아들인 변경이 줄어들므로 유한 번에 끝납니다.
- **현재 구현**: parent의 삭제만 되돌립니다. 같은 쪽이 함께 삭제한 다른 자식의 삭제는 그대로 받아들입니다
  (base: 11 → 13, ours: 11·13 삭제, theirs: 11 아래 22 추가 → candidate에 11·22는 있고 13은 없음. 재현했습니다).

### 6.7 후처리 2 — parent cycle

서로 다른 객체의 독립 이동이 합쳐져 candidate에 parent cycle이 생기면 그 cycle의 이동을 되돌립니다.

- 받아들인 MoveObject 대상에서 시작해 parent 사슬을 반복문으로 따라가고 방문 집합으로 끝을 보장합니다(재귀
  없음). cycle 구성원 중 받아들인 MoveObject가 하나도 없으면 입력에 이미 있던 cycle이므로 무시합니다.
- cycle이 있으면 cycle 구성원의 MoveObject를 받아들인 목록에서 빼고, cycle 구성원마다 conflicting-reorder
  (대상 위치)를 기록한 뒤 다시 적용합니다. 뺄 이동이 없으면 멈춥니다.
- **현재 구현**: cycle 구성원 전원에게 conflict를 기록하므로, 이동하지 않은 구성원도 base·ours·theirs 위치가
  모두 같은 conflicting-reorder를 받습니다(base: R → A, B; A → A1; B → B1. ours: A를 B1 아래로, theirs: B를 A1
  아래로 → A, B, A1, B1 네 conflict. 재현했습니다).

### 6.8 후처리 3 — 독립 삽입 OrderKey 재배정

`OrderKey::between`은 결정론적이므로 두 branch가 같은 이웃 사이에 삽입하면 같은 key가 나옵니다. 순서는
`(OrderKey, StableId)`로 이미 정해지므로 key만 유일하게 만듭니다(문서 불변식 4, §2.7).

- candidate의 parent마다 자식을 `(OrderKey, StableId)` 순으로 정렬하고 같은 key 묶음을 찾습니다.
- 묶음에서 StableId가 가장 작은 객체는 key를 유지합니다. 나머지 각 객체는 묶음 순서대로 다음 두 조건을 모두
  만족하면 `OrderKey::between(직전 key, 묶음 다음의 다른 key)`로 옮깁니다. "직전 key"는 처음에 묶음의 key이고,
  객체를 옮길 때마다 그 새 key로 바뀝니다.
  1. merge가 새로 배치함: base에 없거나 base와 parent·key가 다름.
  2. 그 객체와 앞선 묶음 구성원 전부가 같은 parent·key로 함께 있던 입력(ours 또는 theirs)이 없음.
- 시각 순서는 바뀌지 않아야 합니다. base와 parent·key가 같은 객체의 key는 바뀌지 않고, 입력에 이미 있던 같은
  key 충돌도 바꾸지 않습니다.
- 이 단계는 conflict를 만들지 않으며, 결과 `changes`에 새 key가 반영됩니다.

### 6.9 결과 정렬과 실패

- conflict는 `(소유 문서, 객체 StableId, 대상, 종류 code)` 순으로 정렬해야 합니다. 대상 순서는 객체 < 위치 <
  속성(경로 순)이고, 소유 문서는 프로젝트 범위(`None`)가 먼저입니다.
- 받아들인 변경을 적용할 수 없으면 `MergeError::Apply(ApplyError)`로 실패합니다.

### 6.10 알려진 위험과 미구현 (현재 구현)

| ID | 내용 | 근거 |
|---|---|---|
| M1 | 후처리 순서가 고아 방지 → cycle입니다. cycle 되돌림이 이동을 base 위치로 돌리면서 그 base parent가 이미 받아들인 삭제로 없을 수 있는데, 고아 방지를 다시 하지 않습니다. 결과 candidate에 고아가 남고 그 parent 삭제에 대한 conflict는 없습니다. base: R → P, Y; P → X. ours: X를 Y 아래로 + P 삭제. theirs: Y를 X 아래로 → conflict는 X·Y conflicting-reorder 둘이고 candidate의 X는 없는 P를 parent로 가집니다. 재현했습니다 | `merge` 후처리 순서 |
| M2 | merged snapshot을 문서 validator로 검증하지 않습니다(XIV.10.8 미구현). M1 같은 결과가 conflict와 함께 그대로 나옵니다 | 계획 XIV.10.8 `[ ]` |
| M3 | token 타입(`TOKEN_TYPE`) 변경 대 값(`TOKEN_VALUE`) 변경은 서로 다른 경로라 conflict 없이 병합됩니다. 아키텍처 §27.1의 "Token 타입 변경 대 값 변경" conflict가 아닙니다. 재현했습니다 | 계획 XIV.10.7 `[ ]` |
| M4 | 컴포넌트 공개 Property 삭제(Component `COMPONENT_PROPERTIES`) 대 인스턴스 override 추가(instance 객체)는 다른 객체라 conflict 없이 병합됩니다. 재현했습니다 | XIV.10.7 `[ ]` |
| M5 | Context·Binding 공급자 삭제 대 참조 추가, drilling chain break 전용 conflict가 없습니다 | XIV.10.7 `[ ]` |
| M6 | 이동하지 않은 cycle 구성원도 conflicting-reorder를 받습니다(§6.7) | `merge` |
| M7 | MergeResult decoder가 변경 반복·conflict 순서·merged 일치를 검사하지 않습니다(§5.4) | `MergeResult::from_canonical_bytes` |

계획서의 XIV.10.9(결과·conflict 결정론 정렬)는 `[ ]`이지만 §6.9의 정렬은 기준 commit에 구현되어 있습니다.
입력 map 순서를 바꿔 같은 bytes인지 보는 검증 기준 fixture는 없습니다.

## 7. Git 연결 규칙

근거: `git-porcelain::snapshot`, `git-porcelain::classify`, `git-porcelain::conflict`,
`git-porcelain::conflict_snapshot`, `project::conflict_open`, `project::scaffold`, 기술 아키텍처 §26.1.

### 7.1 snapshot 원천

| 원천 | spec 문자열 | 의미 | 불변 |
|---|---|---|---|
| Head | `head` | 읽는 시점의 HEAD commit의 tree. unborn이면 빈 snapshot | 아니오 |
| Index | `index` | 현재 Git index의 stage 0 항목 | 아니오 |
| WorkingTree | `worktree` | 디스크에 **저장된** 작업 트리 파일. 편집기의 저장 대기 메모리 상태는 포함하지 않음 | 아니오 |
| Commit | `commit:<oid>` | 지정한 commit의 tree | 예 |
| Tree | `tree:<oid>` | 지정한 tree | 예 |

- `<oid>`는 소문자 16진수 40자 또는 64자여야 합니다. 공백·대문자·다른 형식은 거부합니다
  (`SourceSpecError::UnknownSource`, `InvalidObjectId`).
- Git status도 Git이 디스크 파일을 읽어 계산하므로 같은 기준(`SnapshotBasis::SavedFilesOnDisk`)입니다.
- Git snapshot은 blob을 `.git` 없는 임시 디렉터리에 원시 bytes 그대로 materialize한 뒤 일반 프로젝트처럼
  엽니다(`snapshot::materialize`). `.git`으로 해석될 수 있는 component, 빈 component, `.`·`..`, 절대 경로,
  NUL은 Git 실행·파일 생성 전에 거부하고, 이미 있는 항목·symlink 상위에는 쓰지 않습니다.

### 7.2 경로 분류

`classify_project_path`는 저장소 root 기준 경로를 **정확한 bytes**로 분류해야 합니다.

| 경로 | 분류 |
|---|---|
| `stellar.yaml` | Stellar Manifest |
| `stellar.lock.yaml` | Stellar Lock |
| `design/` 아래(`design/` 자체 제외) | Stellar Design |
| `assets/` 아래(`assets/` 자체 제외) | Stellar Asset |
| 그 밖(`README.md`, `.gitignore`, `.gitattributes`, `.stellar/`, `Design/x`, `docs/stellar.yaml` 등) | non-Stellar |

- non-Stellar 파일은 raw Git file status(Git `XY` 코드 그대로)로만 표시하고 semantic parser로 해석하면 안
  됩니다.
- rename/copy의 두 경로 중 한쪽만 non-Stellar이면 `cross_boundary`로 따로 둡니다(raw 표시, semantic 입력
  아님).

### 7.3 unmerged stage 수집

- `git ls-files -u -z --full-name -- :/`의 항목을 경로별로 묶습니다. stage 1 = base(공통 조상), 2 = ours(현재
  branch), 3 = theirs(병합 대상)입니다. 없는 stage는 "없음"입니다.
- 같은 경로·stage가 두 번 나오거나 stage 0이 섞이면 해석 오류입니다. index는 읽기만 합니다.
- stage 존재 여부의 표시(`unmerged_xy`):

| base | ours | theirs | XY |
|---|---|---|---|
| 있음 | 있음 | 있음 | `UU` |
| 없음 | 있음 | 있음 | `AA` |
| 있음 | 없음 | 있음 | `DU` |
| 있음 | 있음 | 없음 | `UD` |
| 없음 | 있음 | 없음 | `AU` |
| 없음 | 없음 | 있음 | `UA` |
| 있음 | 없음 | 없음 | `DD` |
| 없음 | 없음 | 없음 | `DD` |

- conflict 경로를 §7.2로 나눠 Stellar 파일은 Semantic Merge 대상, non-Stellar 파일은 raw conflict로 둡니다
  (`partition_conflicts`). non-Stellar conflict는 Semantic Merge로 해석하면 안 됩니다.

### 7.4 conflict side 구성

`materialize_conflict_snapshots`는 base·ours·theirs 프로젝트를 각각 임시 디렉터리로 만듭니다
(`side_entries`).

1. 현재 index의 stage 0 항목(Git이 이미 병합해 stage한 결과)을 세 side에 모두 넣습니다.
2. Stellar conflict 경로마다 그 side의 stage blob이 있으면 넣고, 없으면 넣지 않습니다.
3. non-Stellar conflict 경로는 어느 side에도 넣지 않고 raw conflict로 돌려줍니다.
4. 항목은 경로 순으로 정렬합니다. 작업 트리와 index는 읽기만 합니다.

`project::conflict_open::open_conflict_projects`는 세 디렉터리를 실제 프로젝트 열기 loader(`stellar.yaml` →
current pointer → manifest → chunk)로 열고, side별 결과(성공 또는 그 side의 열기 오류)를 따로 보존합니다. 한
side가 실패해도 다른 side와 raw conflict 목록은 그대로 씁니다. 임시 디렉터리는 소유자 전용 session root 아래에
만들고 결과를 drop하면 지웁니다.

### 7.5 저장 형식과 Git 병합

- chunk와 generation manifest 파일은 content-addressed이고 불변입니다(형식 명세 §6.3, §10.3). 두 branch가
  각각 저장하면 내용 hash 이름의 새 파일이 추가됩니다. chunk 경로는 payload hash만으로 정해지고 header의 kind는
  경로에 들어가지 않습니다(형식 명세 §6.1, §6.2).
- current pointer `design/current`는 manifest hash 한 줄입니다(형식 명세 §11). 두 branch가 같은 문서를 서로
  다르게 저장한 뒤 병합하면 Stellar conflict 경로는 `design/current` 하나이고, 세 side는 각자 자기 generation을
  엽니다(통합 테스트 `crates/project/tests/conflict_project_open.rs`).
- 새 프로젝트의 `.gitattributes`는 `* text=auto eol=lf`와 흔한 이미지 확장자 `binary` 표시만 담습니다
  (`project::scaffold::DEFAULT_GITATTRIBUTES`). Stellar 전용 merge driver나 `.starchunk`·`.starmanifest`의
  binary 표시는 없습니다.

### 7.6 미구현 (현재 구현)

| 계획 ID | 내용 | 상태 |
|---|---|---|
| XIV.4.1~XIV.4.3, XIV.4.5 | HEAD→Index Staged, Index→WorkingTree Unstaged semantic status, 세 값 보존, untracked, corrupt 처리 | 없음 |
| XIV.5~XIV.7 | 선택 Staging·Unstage | 없음 |
| XIV.11.3 | 세 side 프로젝트를 Semantic Snapshot으로 바꿔 merge 엔진에 넘기고 conflict 경로 → 객체 mapping 유지 | 없음. 열린 프로젝트(`SavedProject`)를 `SnapshotBuilder` 입력으로 바꾸는 코드가 없습니다 |
| XIV.12 | conflict 해결(resolution action, 검증, generation save, `git add`) | 없음 |

semantic-diff·semantic-merge crate는 기준 commit에서 제품 코드가 호출하지 않습니다. semantic-diff를 쓰는 곳은
semantic-merge crate와 두 crate의 테스트, `xtask`의 crate graph 목록뿐입니다.

## 8. 기술 아키텍처 §26·§27과의 차이

| 아키텍처 | 구현(이 명세) |
|---|---|
| §26.1 HEAD·Index·Working Tree snapshot 비교 | snapshot 원천·materialize는 있음(§7.1). semantic status 계산은 없음(§7.6) |
| §26.2 Binding 경로 변경 | binding 전용 객체·key 없음. binding expression은 owner를 가진 ExpressionNode로 표현 가능(§3.6) |
| §27.1 자동 병합 "충돌하지 않는 자식 삽입" | 구현(§6.3, §6.8) |
| §27.1 conflict "Token 타입 변경 대 값 변경" | 같은 속성 경로의 값 타입 변경 대 값 변경만 분류(§6.4). token의 타입 key와 값 key는 경로가 달라 병합됨(M3) |
| §27.1 conflict "Component Property 삭제 대 사용처 추가" | 없음(M4) |
| §27.1 conflict "Context·Binding 공급자 삭제 대 참조 추가" | 없음(M5) |
| §27.1 "동일 StableId의 불일치 정의" | InconsistentDefinition(§6.4) |
| §27.2 해결 UI, `git add` | 없음(§7.6) |

## 부록 A. merge corpus ↔ 명세 대응표

merge corpus는 `crates/semantic-merge/src/merge.rs`·`model.rs`의 테스트, 그 안의 `conflict_matrix` 행, 그리고
`core-types::order_key_merge_fixtures`의 fixture입니다. 모든 항목이 아래 표에 있습니다.

### A.1 semantic-merge 테스트

| 테스트 | 고정하는 규칙 |
|---|---|
| `different_objects_merge_without_conflict` | §6.3 한쪽만 변경한 객체 |
| `same_object_different_paths_merge_without_conflict` | §6.3 4단계 서로 다른 경로, 위치 대 속성 |
| `independent_additions_and_deletions_merge` | §6.3 한쪽만 변경, §3.7 `children` 순서 |
| `one_sided_merge_returns_that_side` | §6.1 `merged`·`changes`, 한쪽만 변경 |
| `overlapping_edits_are_conflicts_not_silent` | §6.3 4단계, §6.1 conflict 대상 base 유지 |
| `disjoint_random_edits_merge_without_conflict` | §6.3 서로 다른 객체(무작위 100 seed) |
| `identical_edit_merges_as_one_change` | §6.3 1·3·4단계 같은 변경 한 번 |
| `same_edits_on_both_sides_property` | §6.3 `merge(base, X, X) == X` |
| `different_values_conflict_preserves_all_three_values` | §6.5 세 상태·출처, §6.9 정렬, §5.4 round-trip |
| `conflict_matrix` | §6.4 분류 표(A.2) |
| `delete_conflict_keeps_base_object_and_records_sides` | §6.5 객체 대상 상태, §6.1 base 유지 |
| `order_key_merge_fixtures_pass` | §6.8 재배정(A.3) |
| `conflicting_reorder_of_same_child` | §6.3 3단계, §6.5 위치 상태 |
| `independent_moves_forming_a_cycle_are_conflicts` | §6.7 cycle 되돌림 |
| `child_added_under_deleted_parent_is_conflict` | §6.6 고아 방지 |
| `conflict_kind_codes_golden` | §6.4 code·이름 |
| `property_values_accessor` | §6.5 `property_values` |
| `merge_result_round_trips` | §5.4 MergeResult, 절단 입력 거부 |
| `merge_input_round_trips` | §5.4 MergeInput, `TrailingBytes`, `BadMagic` |
| `malformed_conflict_is_rejected` | §5.4 모르는 kind, `conflict.path_root` |

### A.2 `conflict_matrix` 행

base는 root 10 ─ 11(a) ─ 13(c), 12(b)입니다. 기대 conflict는 모두 객체 11에 걸립니다.

| 행 | ours | theirs | 기대 | 규칙 |
|---|---|---|---|---|
| `delete vs modify` | 11·13 삭제 | 11 이름 변경 | DeleteVsModify | §6.4 구조 판정 |
| `modify vs delete` | 11 이름 변경 | 11·13 삭제 | DeleteVsModify | §6.4 구조 판정(대칭) |
| `delete vs move` | 11·13 삭제 | 11을 12 아래로 | DeleteVsMove | §6.4 위치만 |
| `reorder vs delete` | 11 재정렬 | 11·13 삭제 | DeleteVsMove | §6.4 위치만(대칭) |
| `delete vs move+modify` | 11·13 삭제 | 11 이동 + 이름 변경 | DeleteVsModify | §6.4 위치 외 변경 포함 |
| `type vs value` | 11 이름을 Uint로 | 11 이름 변경 | TypeChangeVsValueChange | §6.4 속성 판정 |
| `value vs type` | 11 이름 변경 | 11 이름을 Uint로 | TypeChangeVsValueChange | §6.4 속성 판정(대칭) |
| `type vs other type` | 11 이름을 Uint로 | 11 이름을 Bool로 | PropertyValue | §6.4 양쪽 타입 변경 |
| `value vs value` | 11 이름 변경 | 11 이름을 다른 값으로 | PropertyValue | §6.4 |
| `delete parent vs add child` | 11·13 삭제 | 11 아래 50 추가 | DeleteVsModify | §6.6 새 객체 고아 |
| `delete parent vs move child in` | 11·13 삭제 | 12를 11 아래로 | DeleteVsMove | §6.6 base 객체 고아 |
| `replace vs modify` | 11을 token으로 교체 | 11 이름 변경 | InconsistentDefinition | §6.4 그 밖 |
| `delete vs delete` | 11·13 삭제 | 11·13 삭제 | 없음 | §6.3 1단계 |
| `move vs move elsewhere` | 11을 12 아래로 | 11 재정렬 | ConflictingReorder | §6.3 3단계 |

matrix는 여섯 conflict 종류를 모두 냅니다.

### A.3 OrderKey merge fixture

| fixture | 내용 | 규칙 |
|---|---|---|
| `same_gap_deterministic_key_tie_break` | base 0x40·0xC0 사이에 양쪽이 같은 key로 삽입. 기대 순서 c0, a, b, c1 | §6.8 같은 key 묶음 재배정, StableId 순 유지 |
| `disjoint_front_and_back_inserts` | 한쪽은 맨 앞, 다른 쪽은 맨 뒤 삽입. 기대 순서 a, c0, c1, b | §6.3 독립 삽입 |

fixture 자체의 스키마 검증(`validate`: 두 branch 모두 삽입, id 유일, key 정규형, 기대 순서가 `(key, id)` 순)과 그
테스트(`all_fixtures_pass_schema_validation`, `same_gap_fixture_has_colliding_keys_resolved_by_id`,
`validate_rejects_empty_branch`, `validate_rejects_wrong_expected_order`, `validate_rejects_non_permutation`)는
`core-types` 안에 있습니다.

## 부록 B. diff corpus ↔ 명세 대응표

### B.1 semantic-diff 단위 테스트

| module | 테스트 | 고정하는 규칙 |
|---|---|---|
| `value` | `canonical_eq_matches_canonical_bytes` | §3.4 비교 = bytes 비교 |
| `value` | `bit_different_finite_floats_are_different` | §3.4 bit 비교, `-0.0` 정규화 |
| `value` | `value_type_distinguishes_typed_tags` | §3.4 `ValueType` |
| `value` | `canonical_bytes_golden` | §5.2 값 bytes 예 |
| `keys` | `keys_are_unique` | §3.5 key 유일 |
| `keys` | `snapshot_only_keys_stay_in_reserved_band` | §3.5 전용 대역 |
| `keys` | `asset_reference_classification` | §3.5 asset 참조 key |
| `snapshot` | `object_kind_codes_round_trip` | §3.2 `ObjectKind` code |
| `snapshot` | `property_root_must_be_owner` | §3.1 속성 소유, §3.7 오류 |
| `snapshot` | `duplicate_object_is_rejected_and_children_are_ordered` | §3.7 `children`, `DuplicateObject` |
| `build` | `head_index_working_tree_snapshots_build` | §3.6 구성, chunk 경유 동일 |
| `build` | `record_order_and_runtime_history_do_not_matter` | §3.1 runtime·배치 무관 |
| `build` | `ast_operator_change_is_one_node_change` | §3.6 AST node 단위 |
| `build` | `variant_patch_change_is_one_entry_change` | §3.6 Variant 항목 단위 |
| `build` | `project_tokens_assets_and_components` | §3.6 Project·Token·Asset·Component, 원자성 |
| `build` | `duplicate_metadata_names_are_rejected` | §3.6 `DuplicateProperty`, 원자성 |
| `change` | `kind_codes_and_names_golden` | §4.4 code·이름 |
| `change` | `provenance_codes_golden` | §4.5 code |
| `change` | `classification_table` | §4.4 `property_for` |
| `change` | `token_objects_use_token_kinds` | §4.4 token 종류 |
| `diff` | `identical_snapshots_have_no_changes` | §4.1 |
| `diff` | `same_name_different_id_is_add_and_remove` | §4.1 이름 추정 금지 |
| `diff` | `same_id_different_kind_is_replace` | §4.1 3, §4.6 교체 순서 |
| `diff` | `bit_different_finite_float_is_detected` | §4.3, §3.4 |
| `diff` | `property_paths_are_compared_independently` | §4.3 경로별, `None` ↔ 값 |
| `diff` | `negative_zero_is_not_a_change` | §3.4 정규화 |
| `diff` | `property_kinds_follow_classification` | §4.4 AssetRefChange·ChangeToken |
| `diff` | `reorder_within_parent_is_not_move` | §4.2 |
| `diff` | `parent_change_is_move_with_new_key` | §4.2, §4.6 위치가 속성보다 먼저 |
| `diff` | `modifier_reorder_is_reorder` | §4.2 modifier 위치 |
| `diff` | `rechunk_only_change_has_zero_semantic_diff` | §4.2 저장 배치 무관 |
| `diff` | `definition_change_and_instance_override_have_distinct_provenance` | §4.5 InstanceOverride·ComponentDefinition |
| `diff` | `instance_scene_fields_are_document_changes` | §4.5 공통 필드는 Document |
| `diff` | `diff_bytes_ignore_input_order` | §4.6 결정론 |
| `diff` | `canonical_order_key_is_document_object_path_kind` | §4.6 정렬 key |
| `diff` | `diff_bytes_round_trip_and_reject_non_canonical_order` | §5.3 |
| `diff` | `whole_snapshot_add_and_remove` | §4.1 1·2 |
| `codec` | `snapshot_round_trips` | §5.2 snapshot |
| `codec` | `malformed_snapshot_bytes_are_rejected` | §5.5 |
| `codec` | `non_canonical_values_are_rejected` | §5.1 비정규 값, 과할당 금지 |
| `apply` | `apply_diff_reproduces_target_property` | §4.7 |
| `apply` | `apply_inverse_restores_base` | §4.8 |
| `apply` | `inverse_swaps_each_change` | §4.8 |
| `apply` | `stale_base_is_rejected` | §4.7 stale 오류 |
| `apply` | `invalid_change_lists_are_rejected` | §4.7 목록 검증 |

### B.2 semantic-diff fuzz harness (`crates/semantic-diff/tests/semantic_diff_fuzz.rs`)

| 테스트 | 고정하는 규칙 |
|---|---|
| `valid_pairs_apply_and_inverse_reproduce` | §4.7, §4.8, §5.2·§5.3 round-trip(유효 편집·무관 문서) |
| `malformed_bytes_never_panic_and_accepted_inputs_are_canonical` | §5.1 panic 금지, 수용 입력은 byte-identical 재인코딩 |
| `stale_and_foreign_diffs_never_panic` | §4.7 stale·남의 diff는 구조화 오류 |
| `campaign_is_deterministic` | §4.6 결정론 |

반복 수 기본값은 400이고 `STELLAR_SEMANTIC_FUZZ_ITERATIONS`로 늘릴 수 있습니다.
