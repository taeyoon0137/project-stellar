# ADR-0099: Component Context 코어(저장 형식·해석·진단·명령·diff)

## Status

Accepted (2026-11-23). ADR-0025 결정 3(Component Context)이 구현 행에 미룬 저장 형식, numeric 값, 공개 API를
고정합니다. 번호는 IX.12.1 예약(ADR-0075 결정 8, `stellar_core_types::reserved::context`)을 그대로 씁니다. 대상 행은
XXX.7.1\~XXX.7.11과 XXX.7.13의 엔진 부분입니다. macOS 인스펙터 연결(XXX.7.12)과 셸 FFI는 범위 밖입니다(2026-11-23
사용자 방향: 엔진·기능 먼저, UI는 나중에 다시 설계).

## Context

- ADR-0025 결정 3은 Context를 선언된 인터페이스로 정했습니다. `ContextKey`(StableId·이름·TypeSpec·필수 닫힌 default·
  scope), 노드에 붙는 `ContextProvision`, ComponentDefinition의 `ContextRequirement`, Slot의 공개 Context key 목록,
  가장 가까운 조상 provision 규칙, slot 콘텐츠의 작성 scope 해석(결정 3.11), 진단 종류, provenance, override 결합을
  정했고 저장 형식·tag·API는 구현 행에서 정하기로 했습니다.
- IX.12.1은 번호만 예약했습니다: `ReferenceTarget` Context tag `0x05`, chunk kind `0x8000_0007`(`context-set`),
  record kind `ContextKey` `0x01`·`ContextProvision` `0x02`, ComponentDefinition·Slot 확장 section tag `0x01`, semantic
  snapshot key `0xE705`·`0xE716`과 대역 `0xECxx`.
- 크레이트 경계: `stellar-bindings`는 core-types·expressions에만, `stellar-document`는 core-types·expressions에만,
  `stellar-commands`는 document에만 의존합니다. 그래서 해석기(document)와 binding graph(bindings)는 서로 직접 부르지
  못하고 StableId 수준의 경로표로 이어야 합니다. 텍스트 원본(`stellar-project`)은 아직 표현식 문법이 없어 token 기본값을
  리터럴로만 씁니다(ADR-0077 결정 3).
- 컴포넌트 정의(`ComponentLibrary`)는 문서 모델(`DocumentModel`) 밖의 resource이고, 텍스트 원본에서도 Property·Slot
  interface를 아직 쓰지 않습니다.

## Decision

### 1. 모델(XXX.7.1, XXX.7.2)

- `stellar_document::context::key::ContextKey`: id, 이름(비지 않고 앞뒤 공백 없음), `ContextScope`(Project·
  Library(id)·Document(id)), 값 타입, default. default는 `CheckedExpression::closed`(IX.1.2와 같은 검사)를 통과해야
  하며 생성자가 default를 요구합니다. 다른 key를 읽는 default도 열린 식이라 거부합니다. 참조는 StableId만 씁니다.
- `ContextProvision`: id, owner(`ContextOwner::View(view)` 또는 `ContextOwner::Component(component)`), provider node
  StableId, key StableId, 값 expression(AST). 한 provider에 같은 key provision은 하나뿐입니다. owner를 기록하는 이유는
  "소유 definition·View 안의 node에만 붙는다"를 검사할 수 있게 하려는 것입니다. View owner의 provider는 그 View의
  scene node, Component owner의 provider는 그 컴포넌트 정의 tree의 node여야 하며 자기 tree 안의 ComponentInstance
  node는 허용합니다(다른 컴포넌트 내부 node는 거부).
- `ContextRequirement`는 컴포넌트 StableId → key 집합, Slot 공개 목록은 Slot StableId → key 집합입니다.
- 네 가지를 문서 하나의 `ContextSet`에 둡니다. `DocumentModel`이 거울로 들고(`context()`, `replace_context`), 영속 레코드
  (`DocumentRecords`)와 ComponentDefinition·Slot bytes는 바꾸지 않습니다. ComponentDefinition 확장 section 자리(예약
  tag `0x01`)는 그대로 비워 두고, 같은 tag 값을 context-set 안 requirement·Slot section 표지로 씁니다. 근거: 컴포넌트
  interface는 텍스트 원본에 아직 없고(ADR-0077 결정 3), Context 네 요소를 한 의미 단위로 두면 diff·merge·저장이 한 곳에서
  끝납니다.
- 삭제된 scene node를 provider로 하는 View provision은 layout record(ADR-0086)처럼 StableId로 남아 있다가 node가
  되돌아오면(삭제 undo) 다시 보입니다. 영속 출력(`ContextSet::live`)에는 살아 있는 provider의 provision만 나옵니다.

### 2. 저장 형식

- canonical bytes(`ContextSet::encode`): `version:u16 LE(=1)`, record 수 `u32`, record(`kind:u8` + `u32` 길이 + bytes,
  kind는 `RECORD_CONTEXT_KEY`(0x01)·`RECORD_CONTEXT_PROVISION`(0x02), key → provision 순, 각각 StableId 오름차순),
  `COMPONENT_SECTION_CONTEXT_REQUIREMENTS`(0x01) + 항목 수 + (component id + key 수 + key id*), `SLOT_SECTION_CONTEXT_KEYS`
  (0x01) + 같은 구조. 디코더는 다시 인코딩한 bytes가 입력과 같아야 받습니다.
- chunk cache: Context가 비어 있지 않으면 `context-set` chunk(kind `0x8000_0007`, required)를 하나 쓰고 DocumentManifest
  payload 끝에 그 ChunkId를 선택 절로 붙입니다. Context가 비면 절과 chunk를 쓰지 않아 기존 bytes가 그대로입니다. 이
  kind를 모르는 reader는 V.2.7 규칙대로 오류입니다(required).
- 텍스트 원본 v4: Context가 비어 있지 않으면 `design/context.stellar.yaml`(`format: stellar-context`) 하나를 씁니다. key·
  provision은 StableId 순 목록이고, expression은 root가 리터럴 하나면 `expression`(root node id) + `type` + `value`(token
  원본과 같은 리터럴 문법), 그 밖이면 `ast`(canonical AST bytes의 hex)로 씁니다. 손실 저장은 없습니다.

### 3. 참조와 binding graph(XXX.7.3)

- `ReferenceTarget::Context(StableId)`(tag `0x05`, payload = key id 16 bytes). 기존 tag 0x01\~0x04와 bytes는 바뀌지
  않습니다.
- `DependencyNode::ContextKey(key)`(key default 값, producer `Producer::ContextDefault`)와
  `DependencyNode::ContextProvision{provision, instances}`(provision 하나가 인스턴스 chain 한 곳에서 내는 값, producer
  `Producer::ContextProvision`)를 공통 node로 둡니다. 정의 tree node의 provision은 인스턴스마다 값이 달라질 수 있어 chain을
  식별에 넣습니다.
- 소비 binding의 `Context(K)` 읽기는 소비 지점의 해석 결과로 연결합니다: producer별 경로표(`ContextRoutes`: key →
  provision node 또는 key node)를 `BindingEvaluator::from_specs_with_context`에 넘기면 의존 간선·평가·타입 조회·provenance가
  모두 같은 경로표를 씁니다. 경로표가 없으면 key node(default)입니다. provision 값 식이 같은 key를 읽으면 경로표가 바깥 조상
  해석을 가리킵니다. 서로 다른 key가 서로를 읽는 cycle은 이 graph의 cycle 검사(VIII.6.3)가 거부합니다.

### 4. 해석기(XXX.7.4)

- 소비 지점은 `ContextSite{instances, node}`입니다(instances가 비면 scene node, 아니면 scene 인스턴스부터 안쪽으로의 chain과
  마지막 인스턴스가 펼치는 정의의 node). instance expansion view(IX.2.2)의 `ExpandedNodeId::Virtual`과 같은 식별입니다.
- 위로 올라가는 규칙(작성 scope 기준): node 자신 → 작성 부모. 정의 root에 이르면 그 정의를 펼친 instance node로 이어
  갑니다. 작성 부모가 ComponentInstance이고 node가 그 인스턴스의 slot override 콘텐츠이면, 찾는 key가 그 Slot 공개 목록에
  있을 때만 slot host(인스턴스 chain을 한 단계 늘린 site)부터 컴포넌트 내부 조상을 찾고, 없으면 instance node로 이어
  갑니다. 공개하지 않은 key는 내부를 건너뛰고 instance node로 갑니다(ADR-0025 결정 3.11). 끝까지 없으면 key default입니다.
- provision 값 식의 같은 key 읽기는 provider의 작성 부모부터 찾습니다(바깥 조상 값). 다른 key 읽기는 provider 자신부터
  찾습니다(같은 node의 다른 key provision이 보임).
- 결과는 `ContextResolution`(key, 출처 = Provision{id, provider, instances} 또는 Default, 지나간 site 경로, 공개 목록으로
  들어간 Slot)이고 `ContextResolver`가 (site, key, 시작 포함 여부)별로 캐시합니다(runtime index). 결과는 입력의 함수입니다.

### 5. validator(XXX.7.5)

`ContextDiagnostic`의 종류와 수준:

| 종류 | 수준 |
|---|---|
| 정의 안(그 정의가 작성한 slot 콘텐츠 포함)의 선언 없는 소비 | 오류 |
| 타입 불일치(provision 값, 소비 binding의 기대 타입) | 오류 |
| provision의 owner 범위 밖 provider | 오류 |
| 존재하지 않는 key의 provision·Slot 공개 목록 항목·requirement | orphan |
| 쓰지 않는 requirement | 경고 |
| default로 해석됨, shadowing | 정보 |

View 최상위 트리의 소비는 선언이 필요 없습니다. K1↔K2 cycle은 provision 값 식끼리의 해석 graph에서 찾아 오류로 내고, 저장
경로는 같은 판정을 binding graph cycle 검사로 거부합니다.

### 6. 증분 재평가(XXX.7.6)

`ContextChange`(provision 추가·삭제·값 변경, key default 변경, node reparent·이동·삭제, instance ComponentRef 교체, slot 콘텐츠
삽입·삭제·재정렬, Slot 공개 목록 변경, View Variant 구조 patch 적용·해제, Fragment 묶기·풀기)마다 영향받는 소비 지점은
"그 변경이 해석 경로에 들어가는 지점"입니다. 문서 쪽 `affected_consumers`가 변경 대상 node의 작성 subtree(인스턴스 펼침
포함)에 있는 소비 지점, 그 key를 읽는 지점, 그 provision을 해석 결과로 쓰는 지점을 고르고, bindings 쪽
`ContextEvaluator::apply_routes`가 그 소비 binding만 경로를 바꾼 뒤 위상 순서로 다시 평가합니다. 무관한 subtree의 소비는
평가하지 않습니다.

### 7. provenance(XXX.7.7)

`ContextEvaluator::context_provenance`는 VIII.6.5 chain에 Context 단계(`ContextHop`: 읽은 key, 출처 provision StableId·provider
node·인스턴스 chain 또는 default, 공개 목록을 지난 Slot)를 붙입니다.

### 8. command(XXX.7.8)

`CommandKind::EditContext`(`0x00A0`, `edit-context`) 하나에 `ContextEdit` 연산(key 생성·rename·default 변경·삭제, provision
생성·값 변경·삭제, requirement 추가·삭제, Slot 공개 목록 추가·삭제)을 담습니다. inner tag는 1\~11 고정입니다. 역연산은 같은
종류의 반대 연산이며, key 삭제의 역은 key와 함께 지운 provision·requirement·공개 목록 항목을 되살리는 복원 연산입니다. 소비가
있는 key를 지우면 문서는 key를 지우고, binding 쪽 orphan validator(VIII.8.1)가 `Context(K)` 참조의 타입을 찾지 못해 소비
binding을 `OrphanedBinding`으로 바꿉니다(`SourceMissing`). provision만 지우면 다음 해석이 default로 갑니다(orphan 아님).

### 9. override(XXX.7.9)

provision 값의 PropertyPath는 `(provision StableId, [PROVISION_VALUE = 0xEC10])`입니다. 리터럴 값 provision은 이 경로로 View
Variant(IX.7.3)·Target Profile override(IX.10.7)의 공통값이 되고, 우선순위는 `stellar_target_profile::overrides::
resolve_for_profile`의 기존 규칙(공통값 → profile override → variant override → profile × variant)을 그대로 씁니다. 같은
구체성 Variant 충돌은 variant resolver의 `VariantConflict`를 Context 진단으로 옮깁니다. ops `context::resolve_overrides`가 이
두 resolver를 잇습니다(document는 target-profile에 의존할 수 없음).

### 10. Semantic Diff·Merge(XXX.7.10)

- 객체 종류 `ObjectKind::ContextKey`(`0x0801`)·`ObjectKind::ContextProvision`(`0x0802`), 속성 key는 `0xEC01`(이름)·`0xEC02`
  (scope)·`0xEC03`(값 타입)·`0xEC04`(default bytes)·`0xEC11`(owner)·`0xEC12`(provider)·`0xEC13`(key)·`0xEC10`(값 bytes)입니다.
  requirement는 Component 객체의 `0xE705`, Slot 공개 목록은 Slot 객체의 `0xE716`(Ids, 정렬)입니다.
- merge 전용 conflict: `ContextProvisionRemovedVsConsume`(11, 한쪽 provision 삭제 대 다른 쪽이 그 provider subtree에서 같은
  key 소비 추가), `ContextKeyTypeVsProvisionValue`(12, 한쪽 key 값 타입 변경 대 다른 쪽 그 key provision 값 변경).

### 11. 조회 API(XXX.7.11)

`stellar_document::context::query`: 선택 속성 binding이 Context 소비면 key·해석 값·provider node 경로를, Value Binding 선택기
후보로 그 위치에서 소비 가능한 key(이름·StableId 순)와 선언 필요 여부를 돌려줍니다. 셸 FFI는 만들지 않습니다. headless는
`stellar_ops::context`가 같은 함수를 씁니다.

## Alternatives

- **requirement·공개 목록을 ComponentDefinition·Slot bytes 확장 section에 저장**: 예약 위치를 그대로 쓰는 안입니다. 컴포넌트
  interface가 텍스트 원본에 아직 없고, Context 요소가 세 저장 위치로 흩어져 diff·merge·저장 경로가 셋이 됩니다. 탈락했습니다.
  예약 tag는 비워 둡니다.
- **`DocumentRecords`에 Context 필드 추가**: 레코드 생성 지점 30여 곳과 subtree 분할 규칙을 바꿔야 하고 Context는 subtree에
  속하지 않습니다. 탈락했습니다.
- **소비 binding의 AST를 provision 경로로 다시 쓰기**: 저장 AST와 평가 AST가 달라 provenance·orphan 판정이 둘이 됩니다.
  경로표로 같은 AST를 해석합니다.
- **렌더 트리 기준 slot 해석**: ADR-0025 결정 3.11이 탈락시킨 안입니다.

## Consequences

- 긍정: Context 네 요소가 한 resource라 저장·diff·merge·command가 각각 한 경로입니다.
- 긍정: 해석은 StableId 경로표로 binding graph에 들어가 기존 scheduler·provenance·orphan 경로를 그대로 씁니다.
- 부정: `DocumentModel` 거울이라 문서 레코드만 다루는 경로(외부 원본 반영 `ApplyExternalSnapshot`)는 Context를 따로 넘겨야
  합니다.
- 부정: 정의 tree node를 target으로 하는 binding은 binding graph에서 인스턴스마다 다른 값을 갖지 못합니다(기존 binding graph
  한계). 해석기·`ContextEvaluator`는 site별 값을 계산하지만 binding scheduler는 binding당 site 하나를 씁니다.

## Compatibility

- 기존 파일·chunk·AST bytes는 바뀌지 않습니다. Context가 있는 문서만 새 chunk·manifest 선택 절·텍스트 파일을 씁니다.
- `ReferenceTarget`·`DependencyNode`·`Producer`·`ObjectKind`·`ConflictKind`·`CommandKind`에 variant가 늘어 exhaustive match가
  있는 하위 코드는 갱신이 필요합니다(이 diff에서 갱신).

## Performance

해석은 (site, key)별 캐시이고 조상 탐색은 깊이에 비례합니다. 증분 재평가는 영향받는 소비 binding 수에 비례합니다(무관 subtree
평가 0).

## Security

문서에 실행 코드를 넣지 않습니다. 값 식은 닫힌 표현식 연산 집합만 씁니다.

## Migration

새 형식은 추가 전용입니다. Context가 없는 기존 문서는 그대로 열리고 저장 bytes도 같습니다.
