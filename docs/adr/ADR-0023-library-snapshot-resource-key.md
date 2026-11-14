# ADR-0023: 읽기 전용 library snapshot, resource key와 외부 참조 형식

## Status

Accepted (2026-11-14, 구현 기록)

## Context

XV.5.1은 library 프로젝트가 export한 ComponentDefinition·TokenSet index를 읽는 `LibrarySnapshot`을
요구하고, 모든 resource key에 LibraryId + commit + StableId를 넣으라고 합니다(아키텍처 30.3의 참조 키
`DependencyInstanceId + LibraryId + ResolvedCommit + ResourceStableId`). 같은 library의 여러 버전을
한 프로젝트에서 동시에 로드하므로 v1·v2가 같은 StableId를 export해도 충돌하면 안 됩니다.

제약은 다음과 같습니다.

- `stellar-dependencies`는 core-types에만 의존합니다(`tools/xtask/src/crate_graph.rs`).
  ComponentDefinition은 `stellar-document`, TokenSet은 `stellar-bindings`에 있어 이 crate에서 직접
  쓸 수 없습니다. project crate도 bindings에 의존할 수 없습니다.
- 저장된 프로젝트의 ComponentLibrary(kind 4)·TokenSet(kind 5) chunk를 문서 모델로 읽는 경로는 아직
  없습니다(`crates/project/src/document_io`는 document manifest와 view tree만 다룹니다).
- library export는 외부 저장소에서 오는 비신뢰 입력입니다.

## Decision

1. **resource key**는 `LibraryResourceKey { node: DependencyNodeKey, resource: StableId }`입니다.
   `DependencyNodeKey`가 LibraryId + exact commit이므로 key 하나에 세 값이 모두 들어갑니다. 정렬은
   LibraryId → commit → StableId, 표시 형식은 `<library>@<commit>#<stable id>`입니다.
   DependencyInstanceId(어느 선언 경로로 도달했는지)는 resource 식별에 넣지 않습니다. 같은 node는
   그래프에서 dedup되므로(XV.2.2) 같은 resource입니다. 선언 경로는 참조 해석 결과에 따로 담습니다.
2. **형식 중립 export record**를 이 crate에 둡니다. `ExportedComponent`(id, 표시 이름, property 목록,
   slot 목록), `ExportedProperty`(id, 이름, 타입 표기, 공개 여부, default encoding),
   `ExportedTokenSet`, `ExportedToken`(id, 이름, 타입 표기, 값 encoding)입니다. 타입 표기와
   encoding(`ExportFingerprint`)은 producer가 정한 canonical 값이고, 이 crate는 같음 비교에만
   씁니다.
3. 저장 데이터를 export record로 바꾸는 일은 `LibraryExportSource` trait 구현이 맡습니다. 기존
   `LibraryProjectValidator`·`LibraryManifestSource`와 같은 방식입니다. 운영 구현은 document·bindings
   codec과 storage loader를 함께 쓸 수 있는 통합 계층이 제공합니다.
4. **읽기 전용**: `LibrarySnapshot`은 필드가 비공개이고 변경 API가 없습니다. 검증 생성자
   `LibrarySnapshot::new`만 값을 만듭니다.
5. **검증과 상한**(`SnapshotLimits`, 기본값): component 100,000개, component당 property·slot 각
   4,096개, TokenSet 10,000개, token 합계 1,000,000개, 이름 1,024 byte, 타입 표기 256 byte(비어 있지
   않고 제어 문자 없음), encoding 하나 1 MiB. 같은 library 안 resource(component·TokenSet·token)
   StableId 중복, component 안 property·slot 중복, TokenSet 안 token 이름 중복을 거부합니다. 오류에는
   이름·값 원문을 담지 않습니다.
6. **cache 읽기**: `load_cached_snapshot`은 cache entry를 공유 read lock(XV.3.6)을 잡은 채 읽고,
   snapshot을 만든 뒤 lock을 풉니다. Git·network를 쓰지 않습니다. 기대 content hash(lock 값)가 cache
   index 값과 다르면 export를 읽지 않고 실패하며, 다른 entry를 고르지 않습니다(XV.4.4와 같은 정책).
7. **외부 참조(XV.5.2)**: 문서가 저장하는 참조는 `ExternalComponentRef`·`ExternalTokenRef`이고, 값은
   참조하는 쪽 `stellar.yaml`의 의존성 logical name(아키텍처 30.3의 DependencyInstanceId), LibraryId,
   resource StableId 세 개입니다. **commit은 저장하지 않습니다.** commit은 참조가 속한 scope의 lock이
   정하므로 update 후에도 문서를 다시 쓰지 않고 같은 StableId로 새 버전에 연결됩니다(아키텍처 30.2).
   canonical 텍스트는 다음과 같습니다.

   ```text
   component:<logical name>@<library id>/<component StableId>
   token:<logical name>@<library id>/<token StableId>
   ```

   logical name은 `@`·`/`·공백을 쓸 수 없어 구분자가 모호하지 않습니다. UUID는 소문자 hyphen 형식만
   받고, canonical이 아닌 입력과 종류가 다른 참조(component 자리의 token 참조)는 거부합니다. 최대 길이는
   `component:` + 128 + 1 + 36 + 1 + 36 byte입니다.
8. **참조 scope(XV.5.4)**: 참조의 선언 이름은 참조가 속한 scope에서 해석합니다. 사용 프로젝트 문서는 lock의
   root edge, library resource(export record의 `references`)는 그 node에서 나가는 edge를 씁니다. scope 표는
   resolver 그래프나 lock entry의 canonical dependency path에서 만듭니다. 사용 프로젝트는 직접 선언하지 않은
   전이 library를 참조할 수 없고, 선언 이름이 다른 library를 가리키면 거부합니다.

## Alternatives

- **dependencies가 document·bindings에 의존:** crate 의존 방향을 바꿔야 하고, 이 작업 범위에서
  바꿀 수 없어 탈락했습니다.
- **snapshot을 project crate에 둠:** project는 bindings(TokenSet)에 의존할 수 없고, resolver·cache·lock
  모델과 snapshot이 서로 다른 crate에 흩어집니다. 탈락했습니다.
- **resource key에 선언 경로 포함:** diamond로 같은 node에 두 경로로 도달하면 같은 resource가 두
  key를 갖게 됩니다. 탈락했습니다.
- **외부 참조에 commit 저장:** update마다 사용 프로젝트의 모든 문서 참조를 다시 써야 하고, lock과 문서가
  서로 다른 commit을 가리킬 수 있습니다. 탈락했습니다.
- **외부 참조에 선언 이름 대신 LibraryId만 저장:** 같은 library를 두 이름으로 다른 버전에 선언하면 참조가
  어느 버전인지 정할 수 없습니다. 탈락했습니다.

## Consequences

- v1·v2가 같은 StableId를 export해도 key의 commit이 달라 한 index에 함께 담깁니다.
- 실제 ComponentLibrary·TokenSet chunk를 읽는 `LibraryExportSource` 구현은 후속 통합 작업입니다. 그
  전까지 snapshot은 trait 구현(테스트 fake 포함)으로만 채워집니다.
- export 타입 표기·encoding 규칙이 바뀌면 diff(XV.7.2) 결과가 달라지므로 producer는 같은 규칙을
  유지해야 합니다.

## Compatibility

- snapshot은 메모리 모델이고 저장하지 않습니다.
- 외부 참조 canonical 텍스트(결정 7)는 문서에 저장될 값의 형식입니다. 문서 chunk에 넣는 통합은 후속
  작업이며, 그 전까지 기존 문서 포맷은 바뀌지 않습니다. 형식이 바뀌면 새 ADR이 필요합니다.
- 공개 API: `stellar_dependencies::library_snapshot`, `stellar_dependencies::external_ref` 모듈이
  추가됩니다. FFI 영향은 없습니다.

## Performance

- snapshot 생성은 export record 수에 선형이고(BTreeMap 삽입 log 비용), 상한 안에서만 일어납니다.
  cache entry read lock은 export를 읽는 동안만 잡습니다.

## Security

- 비신뢰 export의 개수·크기·중복을 생성 시점에 검사합니다. 검증하지 않은 export는 snapshot이 될 수
  없습니다.
- 오류·표시 문자열에 repository URL·경로·이름 원문을 넣지 않습니다(XV.1.4).

## Migration

- 해당 없음. 이전에 library snapshot을 만든 코드가 없습니다.
