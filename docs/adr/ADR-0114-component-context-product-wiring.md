# ADR-0114: Component Context 제품 경로 연결(외부 반영·diff·merge·CLI·MCP·binding graph·prototype)

## Status

Accepted (2026-11-24). engine lane `context-wiring`의 결정입니다. ADR-0099가 "후속"으로 남긴 제품 경로를 잇습니다. 대상 행은
XXX.7.2·XXX.7.3·XXX.7.8·XXX.7.10·XXX.7.11의 남은 범위와 XXX.2.4·XXX.2.5입니다. ADR-0099 Consequences의 "그 전까지
Context를 만드는 제품 입력(CLI·MCP·UI)은 두지 않습니다"와 결정 3·11·12의 후속 문장은 이 ADR 결정 1\~6이 대체합니다.
ADR-0099 본문은 고치지 않습니다. 셸 FFI와 macOS UI(XXX.7.12)는 범위 밖입니다(2026-11-23 사용자 방향).

## Context

- ADR-0099는 Context를 `DocumentModel` 거울(레코드 밖 `ContextSet`)로 두었습니다. 그래서 문서 레코드만 다루는 제품 경로는
  Context를 따로 넘기지 않으면 잃습니다. 앞선 통합(`748181c2`)이 창 저장·원본 변환을 고쳤고, 남은 경로는 외부 원본 반영
  (`ApplyExternalSnapshot`, 창 동기화·reconciliation, 텍스트 원본 session), 제품 semantic snapshot 조립
  (`stellar_ops::diff::snapshot_of`, compare, Git stage, 병합 결과 저장), headless 편집·조회 입력이었습니다.
- requirement·Slot 공개 목록은 semantic snapshot에서 Component·Slot 객체의 속성(`0xE705`·`0xE716`)이라, 컴포넌트 library를
  투영하지 않는 문서 범위 snapshot에는 넣을 수 없습니다(`SnapshotError::ContextOwnerMissing`).
- 해석기(document)와 binding graph(bindings)는 서로 의존하지 않고 ops도 bindings에 의존하지 않습니다. 둘을 함께 쓰는 제품
  crate는 prototype(core-types·expressions·document·bindings)과 integrity입니다.
- storage `INITIAL_KINDS`(보호 경로)에는 context-set kind가 없어 `stellar inspect`의 chunk 요약이 이름을 몰랐습니다.
- 확인한 결함: `stellar apply`가 Context가 있는 문서를 두 번째 저장할 때 base의 context-set pointer를 그대로 두고 새 chunk도
  써서 ChunkId 중복으로 실패했습니다. 텍스트 원본 session(`TextSourceSession::project`)도 레코드 사본이라 저장이 Context 원본을
  지웠습니다.

## Decision

### 1. 외부 원본 반영(XXX.7.2)

- `RecordChange::Context { before, after }`(tag 8)를 더하고 command codec version을 6으로 올립니다. 변경은 문서 Context 한
  단위입니다. `before`는 모델 거울 전체(삭제한 provider의 provision 포함), `after`는 외부 원본의 영속 Context
  (`ContextSet::live`)입니다. 사전 조건은 `model.context() == before`이고, 적용은 `EditContext::replace_context`(journal
  `UndoOp::Context`)라 rollback·Undo가 같은 경로이며 Undo가 거울 전체를 되살립니다. 바뀐 key·provision(이전·새 provider)·
  requirement·Slot 공개 목록 항목마다 Context 편집 command와 같은 무효화를 보고합니다.
- `ApplyExternalSnapshot::between_with_context`(레코드 + 외부 Context)와 `between_models`를 둡니다. 영속 Context가 같으면
  변경을 만들지 않습니다(거울에만 있는 provision은 비교하지 않음). 기존 `between`은 "Context 비교 없음"으로 남깁니다(Context를
  모르는 agent 분할 경로). 제품 반영 경로(창 `sync_external`, reconciliation 결과, 텍스트 원본 session `poll_external`)는 모두
  Context를 넘깁니다. 텍스트 원본 session의 저장 사본도 Context를 옮깁니다.

### 2. semantic 투영 범위와 병합 결과(XXX.7.10)

- `stellar_ops::diff::DocumentInput`(레코드 + Context)과 `DocumentSource`(소유)를 투영 입력으로 씁니다.
  - **문서 범위** `snapshot_of`: 레코드와 Context key·provision. requirement·Slot 공개 목록은 투영하지 않습니다. 이 범위를 쓰는
    병합·stage·저장 경로는 모두 "문서 → snapshot → 문서" 무손실 왕복을 검사하므로, 그런 항목이 있는 문서는 조용히 잃지 않고
    거부됩니다(`Lossy`·`Unsupported`).
  - **프로젝트 범위** `add_contexts(…, Some(library))`·`project_snapshot_of`: library 투영 뒤에 Context를 넣고,
    requirement·Slot 공개 목록은 그 컴포넌트·Slot이 library에 있을 때만 넣습니다(`projectable_context`). 없는 대상의 항목은
    validator의 orphan 진단 대상이며 diff에는 나타나지 않습니다. requirement·공개 목록은 library의 Component·Slot 객체 하나에 붙는
    속성이라, 여러 문서가 같은 컴포넌트·Slot에 항목을 두면 합집합 하나로 붙입니다(문서마다 붙이면 같은 속성이 두 번 들어가
    투영 전체가 실패함). `stellar diff`·compare·HEAD 비교·project merge 보기·
    `stellar apply` 결과 diff가 이 범위입니다. HEAD 단일 컴포넌트 비교는 HEAD 문서의 requirement·공개 목록을 그 컴포넌트
    투영에 다시 붙여 같은 범위로 비교합니다.
  - 역투영 `document_from_snapshot`은 `document_records`와 `context_from_snapshot`을 함께 불러 레코드와 Context를 돌려줍니다.
- 병합 결과 반영: 창 reconciliation(`merge_document`)이 Context까지 병합해 `ApplyExternalSnapshot::between_with_context`로
  기록합니다. Git stage(`serialize_candidate`), 병합 해결 저장(`serialize_merged_generation`), Merge Composer 결과
  (`write_documents`)는 `serialize_records_with_context`로 context-set chunk를 쓰고, 다시 쓰는 문서의 base context-set pointer는
  재사용하지 않습니다(`stellar apply`도 같음, 위 결함 수정). 새 병합 규칙은 없습니다(semantic merge 엔진 그대로).

### 3. CLI·MCP Context 편집 입력(XXX.7.8)

- `ops::mcp_command_input`에 연산마다 필드 종류 하나를 둡니다: `create-context-key`, `rename-context-key`,
  `set-context-key-default`, `delete-context-key`, `create-context-provision`, `set-context-provision-value`,
  `delete-context-provision`, `add-context-requirement`, `remove-context-requirement`, `add-slot-context-key`,
  `remove-slot-context-key`. 결과는 `edit-context`(ADR-0099 결정 8) canonical 입력이라 적용·검증 경로는 하나입니다. key 복원
  (tag 12)은 역연산 전용이라 필드 종류가 없습니다.
- scope(`project`·`library <id>`·`document <id>`)·owner(`view <id>`·`component <id>`)·값 타입 문장과 리터럴은 텍스트 원본
  `design/context.stellar.yaml`의 함수(`stellar_project::document_io::source_v4::context`의 `parse_scope`·`parse_owner`·
  `parse_type`·`literal_expression`)로 읽습니다. 식 필드는 `<타입> <리터럴>`(root id = key·provision StableId),
  `context <key>`(key 읽기 하나), `ast <hex>`(canonical AST) 셋 중 하나입니다. key default는 ADR-0099 결정 1대로 닫힌 식만
  받습니다.
- 보호 경로 `schemas/command-transaction.schema.json`을 `cargo xtask generate-command-schema`로 다시 생성합니다(필드 종류
  11개 추가).

### 4. storage kind 표(보호 경로)

`stellar_storage::chunk::kinds`에 `KIND_CONTEXT_SET`(`0x8000_0007`, required, 이름 `context-set`)을 등록합니다. 번호와 이름은
core-types 예약 상수 그대로이고, project의 같은 상수는 이 값을 다시 내보냅니다. 기존 kind 번호·bytes는 바뀌지 않습니다.
ADR-0099 결정 12의 "storage 등록은 후속"을 이 결정이 닫습니다.

### 5. binding graph 입력의 제품 경로(XXX.7.3·XXX.7.6)

- `stellar_prototype::context`: `binding_consumers`(binding 모음에서 `Context(K)`를 읽는 binding을 소비로, 판정은 문서 crate
  `context_reads`), `context_graph`(해석기의 set과 key registry로 key default·site별 provision producer·소비 경로표),
  `affected_nodes`(문서 crate `affected_consumers` 판정을 graph node로), `hop_info`·`consumer_hops`. 종단 시험 도우미는 이
  함수들을 그대로 부릅니다(변환 이중 구현 제거).
- `stellar_bindings::RouteChange::between`: 두 graph 입력의 차이(바뀐 key·provision upsert, 사라진 provision 제거, `affected`
  소비 reroute)를 만듭니다. 사라진 key node는 남기고 그 key를 읽는 binding은 orphan validator가 `OrphanedBinding`으로
  바꿉니다(ADR-0099 결정 8).

### 6. 조회(XXX.7.11)

`stellar_ops::context::read_context`·`read_snapshot_context`가 문서 하나의 key·provision·requirement·Slot 공개 목록과 validator
진단을, 위치(`ContextSite`)를 주면 key마다 해석 값·출처·provider 경로·Slot 경유와 Value Binding 선택기 후보를 돌려줍니다.
해석·후보·진단은 문서 crate 함수(`ContextResolver`·`ContextEvaluator`·`binding_candidates`·`validate_context`) 그대로이고,
값·식·타입 문장은 결정 3 입력과 같은 문법(`value_text`·`expression_text`·`type_name`)입니다. CLI `stellar context <경로>
<문서 id> [--node <id> [--instances <id,…>]]`와 headless MCP 읽기 tool `document.context`(Read 권한, 모든 backend)가 같은
함수를 씁니다. binding은 아직 문서 원본에 저장되지 않으므로 소비 binding 단위 조회(`inspect_binding`)는 앱 경로의 몫이고
headless 조회는 위치 단위입니다.

### 7. prototype 연결(XXX.2.4·XXX.2.5)

- `PrototypeDocument`가 문서 `ContextSet`을 들고, step `ContextRequirement { component, key, add }`(역은 반대 연산,
  `ContextSet::add_requirement`·`remove_requirement`)를 둡니다. `ExposeRequest::ContextRequirement { component, key }`는 미리보기
  항목 하나와 transaction 하나입니다. 이미 선언한 key는 `InUse`, 문서에 없는 key는 `Missing`입니다.
- projection은 `Context(K)` 참조 block마다 key 단위 ContextRead node 하나(StableId = key)와 `value → reference` edge를
  만듭니다. Context를 읽지 않는 그래프의 bytes는 그대로입니다.
- `context_graph_diagnostics`는 validator 진단을 옮깁니다: 선언 없는 소비·없는 key 소비·default 해석·shadowing → 그 key를 읽는
  참조 block의 `reference` socket, 타입 불일치 → PropertyOutput `value` socket, 쓰지 않는·없는 requirement → ContextRead node.
  provision·Slot 공개 목록 진단은 그래프 위치가 없습니다.

## Alternatives

- **`DocumentRecords`에 Context를 넣어 모든 경로가 자동으로 옮기게 함**: ADR-0099가 탈락시킨 안입니다(레코드 생성 지점과
  subtree 분할 규칙 변경).
- **문서 범위 snapshot에 requirement·공개 목록을 넣으려고 Component·Slot 자리표시 객체를 만듦**: semantic snapshot에 문서
  밖 객체를 만들어 diff·merge 결과가 거짓 Component 변경을 냅니다. 탈락.
- **문서 범위에서 requirement·공개 목록을 세 입력이 같을 때만 옮김**: 새 병합 규칙이 생깁니다. 무손실 검사로 거부하는 쪽을
  택했습니다(후속에서 문서 범위 경로에 library를 넣으면 풀림).
- **CLI 입력을 `edit-context` 한 종류 + 하위 연산 필드로 받음**: `FieldSpec`은 종류마다 고정 필드라 schema가 조건부가 됩니다.
  연산마다 종류 하나가 schema·오류 문구가 단순합니다.
- **binding graph 변환을 integrity나 ops에 둠**: ops는 bindings에 의존하지 않고(crate graph), integrity는 진단 집계 crate라
  평가기 조립의 자리가 아닙니다. prototype이 이미 평가기(프리뷰)와 그래프 진단을 가집니다.

## Consequences

- 긍정: 밖에서 바꾼 `design/context.stellar.yaml`이 열린 창에 반영되고 그 창의 저장이 덮어쓰지 않습니다. `stellar diff`·
  compare·merge·stage·apply dry-run에 Context 변경이 나타나고 병합 결과가 ContextSet으로 돌아옵니다. AI·CLI가 같은 파일의
  Context를 편집·조회할 수 있습니다.
- 부정: 문서 범위 경로(창 reconciliation, Git stage, 병합 해결 저장, Merge Composer)는 requirement·Slot 공개 목록이 있는 문서를
  거부합니다. 문서 범위 투영에 컴포넌트 library를 넣는 후속이 필요합니다.
- 부정: requirement·Slot 공개 목록 변경을 semantic stage로 고를 수 없습니다(문서 범위 snapshot에 없음). 위와 같은 후속입니다.
- 부정: agent 외부 변경 분석(`mcp-core::agent_history`)과 attached 편집 미리보기 diff는 레코드만 비교합니다(`diff_records`,
  소유 밖). Context 편집 proposal의 미리보기 diff는 비어 보입니다.
- 부정: command codec version이 6이 되어 version 5 reader는 tag 8을 담은 journal의 복원만 포기합니다(ADR-0079 규칙).

## Compatibility

- 저장 bytes: 기존 chunk·manifest·텍스트 원본 형식은 바뀌지 않습니다. storage kind 표에 이름이 늘 뿐입니다.
- command codec: tag 8이 없는 `apply-external-snapshot` bytes는 version 5와 같습니다.
- 공개 API: `merge_document`·`WindowMerge::Merged`·`snapshot_of`·`add_contexts`·`ExposeRequest::ContextRequirement`·
  `PrototypeDocument`·`NodeDetail`·`RecordChange`·`RecordTarget`의 형태가 바뀌었고 하위 코드는 이 diff에서 갱신했습니다.
- MCP catalog에 `document.context`가 늘어 catalog golden과 headless tool 목록이 바뀌었습니다. FFI는 바뀌지 않습니다.

## Performance

프로젝트 범위 투영은 Context 항목 수에 비례하는 객체를 더합니다. `stellar apply` 결과 diff는 컴포넌트 원본을 한 번 읽어
양쪽 snapshot에 넣습니다(원본을 못 읽으면 문서 범위 diff). 외부 반영의 Context 비교는 `ContextSet` 크기에 비례합니다.

## Security

새 입력은 닫힌 command enum과 기존 텍스트 원본 parser만 씁니다. MCP tool은 읽기 전용이며 고정 문구 오류만 냅니다.

## Migration

없습니다. 기존 프로젝트와 journal은 그대로 열립니다.
