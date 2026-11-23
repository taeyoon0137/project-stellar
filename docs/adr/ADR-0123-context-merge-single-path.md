# ADR-0123: Component Context 병합 경로 하나(문서 범위 무손실 투영, 텍스트 원본 merge, 외부 변경 분석)

## Status

Accepted (2026-11-24). engine lane `context-semantic-merge`의 결정입니다. ADR-0099 결정 10과 ADR-0114 결정 2의 후속이며,
대상 행은 XXX.7.10의 남은 범위와 XIX.7.1·XIX.7.3(외부 변경 분석·분할의 Context)입니다. ADR-0114 Consequences의 "문서 범위
경로는 requirement·Slot 공개 목록이 있는 문서를 거부합니다", "requirement·Slot 공개 목록 변경을 semantic stage로 고를 수
없습니다", "agent 외부 변경 분석은 레코드만 비교합니다"와 ADR-0113 결정 5의 "Component Context는 통째로 비교"(텍스트 원본
merge의 단위 3-way)는 이 ADR 결정 1\~3이 대체합니다. ADR-0099·ADR-0113·ADR-0114 본문은 고치지 않습니다.

## Context

- Context 병합이 두 경로였습니다. 창 reconciliation·Git stage·병합 해결 저장·Merge Composer는 semantic snapshot의 Context
  객체(key·provision)를 엔진으로 병합했고, 텍스트 원본 merge driver(`stellar_ops::merge_text`)는 문서 Context 전체를 값 하나로
  보는 3-way(한쪽만 바뀌면 그쪽, 둘 다 바뀌면 병합 거부)였습니다. 같은 변경이 driver에서는 병합 실패, 창에서는 자동 병합이
  되는 등 판정이 갈렸습니다.
- 문서 범위 투영(`stellar_ops::diff::snapshot_of`)은 컴포넌트 library를 넣지 않아 requirement·Slot 공개 목록을 붙일
  Component·Slot 객체가 없었습니다(ADR-0114 결정 2). 그래서 그런 항목이 있는 문서는 무손실 왕복 검사로 거부됐고, driver가
  Context를 레코드 snapshot 밖에서 따로 다룬 이유도 이것이었습니다.
- 외부 변경 분석(`stellar_agent_edit::agent_history::analyze_external`·`respond_to_external`)과 분할
  (`stellar_commands::agent_external::partition_external`)은 레코드만 비교했습니다. 밖에서 바꾼 Context는 분석 diff에 없고,
  분할의 "바로 적용" 집합에도 없었습니다.
- semantic snapshot 속성 경로는 root 객체 + key 열 + 선택적 map key segment(`PathSegment::Key`)입니다. Document 객체는 이미
  Text style 표를 map key segment 속성으로 담습니다.

## Decision

### 1. 문서 범위의 requirement·Slot 공개 목록 투영

- `SnapshotBuilder::add_context`는 requirement·Slot 공개 목록을 그 Component·Slot 객체가 snapshot에 있으면 지금처럼 그 객체의
  `0xE705`·`0xE716`(Ids, 정렬) 속성으로 붙이고, 없으면 소유 문서의 Document 객체에 붙입니다: 경로는 같은 key(`0xE705`·
  `0xE716`)에 map key segment(컴포넌트·Slot StableId 문자열), 값은 정렬한 key Ids입니다. Document 객체도 없으면
  `SnapshotError::ContextOwnerMissing`입니다.
- 대상마다 속성 하나라 병합 단위는 Component 객체에 붙을 때와 같습니다. 같은 컴포넌트·Slot의 목록을 두 쪽이 다르게 바꾸면
  속성 conflict, 서로 다른 대상이면 자동 병합입니다. 새 병합 규칙은 없습니다.
- `context_from_snapshot`은 그 문서 Document 객체의 두 key 속성(map key segment 필수, 빈 목록 거부)과 지금처럼 같은 문서
  출처의 Component·Slot 객체 속성을 함께 읽습니다. 그래서 문서 범위 "문서 → snapshot → 문서" 왕복이 무손실이고, 문서 범위
  경로(창 reconciliation, Git semantic stage, 병합 해결 저장, Merge Composer)가 그런 문서를 거부하지 않습니다. 목록 변경은
  Document 객체 속성 변경이라 semantic stage로 고를 수 있습니다.
- `stellar_ops::diff::projectable_context(_, None)`과 `add_contexts(_, _, None)`(문서 범위)는 Context 전부를 넣습니다. 프로젝트
  범위(library 있음)는 지금 규칙 그대로입니다(library에 있는 대상만 Component·Slot 객체에 합집합으로 붙임).
- semantic merge의 "provision 삭제 대 소비 추가" 판정(ADR-0099 결정 10)은 `0xE705`·`0xE716` 경로를 소비로 세지 않습니다.
  requirement·공개 목록은 값을 읽는 binding이 아니라 공개 interface 선언이고, Document 객체에 붙으면 tree 위치가 없어 "같은 문서
  안 위치 모르는 소비"로 잘못 conflict가 되기 때문입니다.

### 2. 텍스트 원본 merge의 Context

- `merge_text`의 Context 단위 3-way(`three_way(&base.context, …)`, `TextMergeError::Library { unit: "context" }`)를 지웁니다.
  세 쪽 문서는 레코드와 영속 Context(`DocumentSource::of_model`)이고, 투영·무손실 검사·역투영은 창 reconciliation과 같은
  `merge_apply::document::lossless_snapshot`·`project_documents`입니다. 결과 Context는 병합 snapshot의 역투영입니다.
- 그래서 driver·conflict 세션·창·Composer가 같은 Context 변경에 같은 판정을 냅니다. 같은 항목을 다르게 바꾸면 엔진 conflict
  목록으로 나옵니다(driver는 nonzero exit, ADR-0081).
- 이미지 record·token set의 단위 3-way는 그대로입니다(문서 밖 의미, 이 ADR 범위 밖).

### 3. 외부 변경 분석·분할의 Context

- `partition_external(base, current, incoming)`은 세 문서 모델(레코드 + Context)을 받고 `ApplyExternalSnapshot::between_models`로
  비교합니다. Context는 문서 하나에 한 단위(`RecordTarget::Context`, ADR-0114 결정 1)라 창도 Context를 바꿨으면 Context
  전체가 `divergent`(reconciliation 입력)입니다. 항목 단위 병합은 reconciliation의 semantic snapshot이 합니다(결정 1).
  창이 Context를 바꾸지 않았으면 바로 적용하고, 그 변경의 이전 값은 창 모델의 Context 거울입니다(`ApplyExternalSnapshot`
  사전 조건이 거울 전체 비교이므로).
- `analyze_external`·`respond_to_external`은 `base`·`incoming`을 `DocumentSource`(레코드 + 영속 Context)로 받고, 분석 diff와
  창 쪽 diff를 `diff_documents(…, None)`(문서 범위, 결정 1)로 계산합니다. semantic conflict 후보도 Context 객체·속성을
  포함합니다.
- lease 겹침 대상: provision 변경은 provider node 변경(`ExternalTarget::Node(provider)`, 창·외부 모델 어느 쪽이든)입니다.
  key·requirement·Slot 공개 목록은 node에 붙지 않아 lease 대상이 없습니다.

## Alternatives

- **문서 범위 경로에 컴포넌트 library를 넣어 프로젝트 범위로 투영**(ADR-0114가 적은 후속): 창 reconciliation·stage·Composer
  모든 호출자가 library를 읽어 넘겨야 하고, library에 없는 대상의 항목(orphan)은 여전히 투영할 수 없어 무손실 검사가
  그대로 거부합니다. 탈락.
- **항목마다 별도 snapshot 객체(requirement 객체)**: StableId가 없는 (컴포넌트, key) 쌍에 id를 파생해야 하고 새 ObjectKind와
  보호 경로 예약이 필요합니다. 병합 단위도 대상 목록 하나보다 잘게 나뉘어 프로젝트 범위와 달라집니다. 탈락.
- **목록을 ContextKey 객체에 붙임**: requirement의 key는 다른 library·문서의 key일 수 있어 그 객체가 snapshot에 없을 수
  있습니다. 탈락.
- **driver의 Context 3-way 유지**: 두 병합 경로가 남아 판정이 갈립니다(Context 절). 탈락.
- **agent 분할에서 Context를 항목 단위로 나눔**: `RecordChange::Context`가 문서 하나에 한 단위라(ADR-0114 결정 1) 항목 단위
  적용을 만들면 command 형식이 바뀌고 병합 규칙이 하나 더 생깁니다. reconciliation으로 넘기는 쪽을 택했습니다.

## Consequences

- 긍정: Context 병합 경로가 semantic snapshot 하나입니다. requirement·Slot 공개 목록이 있는 문서도 창 reconciliation, Git
  stage, 병합 해결 저장, Merge Composer, 텍스트 원본 driver를 통과합니다. 목록 변경이 `stellar diff`(문서 범위)와 stage에
  나타납니다.
- 긍정: 외부 writer가 Context를 바꾸면 agent 외부 변경 분석 diff와 분할에 나타나고, 창의 저장 전 Context 변경을 덮어쓰지
  않습니다.
- 부정: 문서 범위와 프로젝트 범위의 목록 위치가 다릅니다(Document 객체 대 Component 객체). 두 범위 snapshot끼리 diff하는
  호출자는 없지만, 새 호출자는 같은 범위끼리 비교해야 합니다.
- 부정: 프로젝트 범위 snapshot(합집합으로 붙은 library Component 목록)에서 문서별 목록을 되돌리는 역투영은 여전히 없습니다.
  병합 결과를 쓰는 경로는 모두 문서 범위라 지금은 쓰이지 않습니다.
- 부정: 창과 외부가 모두 Context를 바꾸면 서로 다른 항목이어도 agent 분할은 바로 적용하지 않고 reconciliation으로
  넘깁니다(reconciliation이 항목 단위로 병합함).
- 부정: attached 편집 미리보기 diff(`mcp-core`, `diff_records`)는 여전히 레코드만 비교합니다(이 ADR 범위 밖).

## Compatibility

- 저장 bytes·텍스트 원본·command codec은 바뀌지 않습니다. semantic snapshot은 메모리 구조이며, 문서 범위 snapshot에 Document
  객체 속성이 늘 뿐입니다(Context 목록이 없는 문서의 snapshot은 그대로).
- 공개 API: `partition_external`(세 문서 모델), `analyze_external`·`respond_to_external`(`DocumentSource` 입력),
  `projectable_context(_, None)`(전부 반환), `TextMergeError::Library`의 `context` 단위 삭제. 하위 호출자는 이 diff에서
  갱신했습니다. FFI·MCP catalog는 바뀌지 않습니다.

## Performance

문서 범위 투영은 목록 대상 수만큼 Document 객체 속성을 더합니다. driver는 Context를 파일 비교 대신 snapshot 객체로 병합해
Context 항목 수에 비례하는 일을 더합니다. 외부 변경 분석은 Context 크기에 비례하는 비교를 더합니다.

## Security

새 입력 형식이 없습니다. 역투영은 map key segment를 StableId로만 받고 형식이 다르면 그 객체 id와 함께 거부합니다.

## Migration

없습니다.
