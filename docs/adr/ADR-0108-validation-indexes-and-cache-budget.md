# ADR-0108: command 검증은 저장소가 편집과 함께 유지하는 index를 읽고, cache는 priority 기반 공통 예산으로 줄인다

## Status

Accepted (2026-11-23).

- 계획서 XVI.4.7\~XVI.4.9(검증 경로 증분 index)와 XVI.5.2\~XVI.5.4, XVI.5.6(cache 예산)을 구현하면서 정한 결정입니다.
- 사용자 방향(2026-11-23, 엔진·기능 먼저)에 따라 앱 셸·FFI 연결은 이 결정에서 다루지 않습니다. 엔진 API와 Rust 시험으로
  검증합니다.

## Context

- command 검증이 호출마다 문서·binding 전체를 다시 훑었습니다.
  - 문서: `crates/commands/src/tree.rs`의 `id_owners`(모든 영속 id와 종류)·`ChildIndex::build`(부모 → 자식 표)가 각 O(N)입니다.
    50,000 node 문서에서 이름만 바꾸는 RestoreNodeRecords validate가 28.9 ms였습니다(VII.2.7 사후 수정, 10 ms Undo/Redo 예산 초과).
  - binding: 토큰 Delete의 inbound 검사(`TokenOrphanPlan::current`)가 모든 binding source를, 참조 타입 조회(`StoreTypes`)가
    Property 참조마다 모든 orphan을 훑었습니다.
  - graph: 토큰·binding·orphan command 검증이 호출마다 `TokenDependencyGraph::build`·`DependencyGraph::build`로 전체 graph를
    만들었습니다.
- `DocumentModel`(document crate)은 이미 부모별 형제 집합, inbound/outbound 참조 index(`references()`), 영속 id 소유 수
  (`contains_persistent_id`)를 편집 API와 함께 증분 갱신합니다. 느린 것은 commands 쪽 helper였습니다.
- `TokenRegistry`(토큰 ID → 정의)와 `BindingSet`(target 속성 → binding)도 producer 조회 index를 편집과 함께 갱신합니다.
- 아키텍처 §10.4는 예산 기반 다단계 cache와 보존·제거 순서를, §6.3은 UI thread 단일 점유 2 ms를 정합니다. XVI.5.1이 OS 메모리
  단계를 bucket·압박 단계로 주지만, glyph atlas·tessellation·이미지 mipmap cache는 예산·제거 기준이 없었고 썸네일 cache는
  없었습니다. XI.5.4 `RecoveryTracker`는 GPU 메모리 부족에 "purge 후 한 번 재시도"를 정해 두었지만 실제로 purge할 대상이
  없었습니다.
- 작업 범위는 `crates/commands/src/tree.rs`, `crates/bindings/src/{store,graph}*`, `crates/runtime/src/cache/**`,
  `crates/renderer-core/src/cache*`입니다. document crate와 기존 cache 파일(`tessellation.rs`, `texture_cache.rs`,
  `glyph/atlas.rs`)은 바꾸지 않습니다.

## Decision

1. **문서 command 검증 index(XVI.4.7)**: 새 index를 두지 않고 `DocumentModel`이 이미 증분 유지하는 index를 읽습니다.
   - `ChildIndex`는 모델을 빌리는 view입니다(만들 때 O(1), `children`은 그 부모의 자식만 복사).
   - 영속 id 조회 `id_use(model, id)`는 `Node(kind)` 또는 `Other`(문서·페이지·View·typography·modifier·path element)를 돌려줍니다.
     command 검증은 node 종류와 "node가 아닌 영속 id인가"만 구분하므로(clip 대상 `None | Node(Path)`, modifier 참조
     `None | Node(_)`, 새 path element 사용 여부) node가 아닌 쪽을 한 값으로 묶어 O(log N)에 답합니다.
   - 영속 id 공간은 문서 validator(VI.10.1)와 같게 typography resource id를 포함합니다. 옛 `id_owners`는 typography를 빠뜨려,
     typography id를 path element id나 clip 대상으로 쓰는 command를 받아들인 뒤 결과 문서가 validator를 어겼습니다. 이 경우를
     `DuplicateId`·`ReferenceTypeMismatch`로 거부합니다. validator가 거부하는 결과를 막는 쪽으로만 바뀝니다. 같은 id를 node와
     다른 영속 id가 함께 쓰는 문서(이미 validator 위반)에서는 node 종류를 우선합니다.
   - 문서 전체에서 다시 만드는 함수(`ChildIndex::rebuild`, `reference_index`, `id_owners`)는 증분 index와 비교하는 test oracle로만
     남깁니다.
2. **binding 저장소 index(XVI.4.8)**: `BindingStore`를 `crates/bindings/src/store.rs`로 옮기고(경로 `orphan::BindingStore`는
   re-export로 유지) 두 index를 함께 갱신합니다.
   - 토큰 → `(binding ID, live|orphan)` 역참조. 위치를 함께 둬 같은 ID가 두 쪽에 겹쳐도 한쪽 제거가 다른 쪽을 지우지 않습니다.
   - orphan target → orphan ID 집합. 조회는 가장 작은 ID(옛 `orphans().find`와 같은 결과)입니다.
   - 저장소를 바꾸는 방법은 index를 갱신하는 메서드뿐입니다(`live_mut()` 삭제, `insert_live`·`remove_live`·`insert_orphan`·
     `remove_orphan`·`orphan_broken`).
3. **토큰·통합 graph(XVI.4.9)**: 별도 graph를 증분 유지하지 않고 두 저장소를 인접 목록으로 쓰는 view(`TokenGraphView`,
   `BindingGraphView`)를 둡니다. 간선은 producer 하나의 참조이고 producer는 저장소가 ID·target으로 바로 찾으므로, 편집이 저장소를
   바꾸는 순간 view의 간선도 바뀝니다(어긋날 상태가 없음). cycle 검사는 새 참조에서 도달하는 부분만 BFS로 훑습니다. 전체 graph
   두 개와 view가 같은 BFS 함수(`graph_view::first_cycle_path`)를 써서, 인접 목록이 같으면 판정과 보고 경로가 같습니다.
4. **cache priority(XVI.5.2)**: `CachePriority`는 제거 순서대로 `BackgroundThumbnail` → `NearViewport` → `VisibleGlyphGeometry`이고
   `DocumentScene`(현재 문서 모델·Scene Snapshot)은 지우지 않습니다. 같은 priority 안에서는 §10.4 "우선 제거" 목록 순서(썸네일 →
   이미지 mipmap → tessellation → glyph atlas page), 같은 종류는 등록 순서, cache 안은 LRU입니다. 공통 manager
   (`stellar_renderer_core::cache::CacheBudgetManager`)는 renderer-core에 둡니다. 제거 대상 cache 대부분이 renderer-core에 있고,
   runtime은 renderer-core에 의존하므로 양쪽이 같은 manager를 씁니다.
5. **사용량 등록(XVI.5.3)**: 기존 cache 파일은 그대로 두고 감싸는 타입(`BudgetedTessellationCache`, `BudgetedMipmapCache`,
   `BudgetedGlyphAtlas`, runtime `ThumbnailCache`)이 원래 cache API를 거치며 entry별 priority·byte·LRU 순번을 기록합니다(공용
   `LruBudget`). 사용량은 실제로 든 resource byte입니다. 현재 문서·scene은 `ReportedUsage`로 사용량만 등록합니다.
6. **증분 제거(XVI.5.4)**: runtime `PressureEvictor`가 압박 단계(진단 `MemoryPressure`와 같은 뜻: Normal → 상한, Warning → 상한의
   절반, Critical → 0, 모두 Document/CurrentScene 사용량 위)로 목표를 정하고, UI thread tick마다 `run_chunk`로 2 ms 예산 안에서
   낮은 priority부터 지웁니다. deadline은 "지난 시간 + 단위 하나의 최대 비용(처음 100 µs, 관측값이 더 크면 그 값)"이 예산
   안일 때만 다음 단위를 지웁니다. 지운 resource의 메모리 반환은 chunk 안에서 하지 않고 pass 동안 모았다가 pass가 끝나는 chunk
   보고로 한 번 내주며, 호출자가 worker thread에서 버립니다. 사용량은 지운 즉시 줄어든 것으로 봅니다.
7. **GPU OOM(XVI.5.6)**: `with_oom_retry`는 메모리 부족 오류이면 `RecoveryTracker`의 판단(`PurgeCachesAndRetry`)대로 GPU에 올라가는
   cache(mipmap·tessellation·glyph atlas)의 지울 수 있는 단위를 모두 지우고 그 자리에서 해제한 뒤 작업을 한 번 더 합니다. 재시도도
   메모리 부족이면 tracker에 알려(`SkipFrame`) 오류를 돌려주고 반복하지 않습니다. 재시도 횟수는 함수 안에서도 1로 막습니다.

## Alternatives

- **document crate에 id 종류 index 추가**: modifier·path element·정의를 구분하는 index를 모델에 두는 방법입니다. command 검증이 그
  구분을 쓰지 않으므로 필요 없고, 작업 범위 밖 crate를 바꿔야 해 택하지 않았습니다.
- **commands `DocumentState`가 별도 index를 들고 ChangeSet으로 갱신**: 모델이 이미 같은 index를 유지하므로 이중 상태가 되고,
  `Command::validate(&DocumentModel)` 공개 API에 index를 넘길 수 없어 택하지 않았습니다.
- **증분 graph 구조체를 저장소 옆에 두고 편집마다 간선 갱신**: 저장소가 이미 producer별 간선을 들고 있어 같은 정보를 두 번
  들게 됩니다. 갱신 누락 위험만 늘어 택하지 않았습니다.
- **기존 cache 파일에 예산 필드 추가**: 작업 범위 밖 파일 수정이 필요하고, 감싸는 타입으로 같은 결과(실제 byte·LRU)를 얻을 수
  있어 택하지 않았습니다.
- **지운 resource를 chunk 안에서 해제 / pass 도중 worker가 계속 해제**: 측정에서 allocator 경합으로 단위 하나가 수 ms 걸리는
  경우가 run마다 생겨(XVI.5.4 보고서) 택하지 않았습니다.
- **runtime이 diagnostics에 직접 의존해 `MemoryPressure`를 받음**: crate graph(`docs/architecture/crate-dependency-graph.md`)에
  없는 간선이라 runtime이 같은 뜻의 `PressureLevel`을 두고 연결 코드가 옮기게 했습니다.

## Consequences

- 이름만 바꾸는 RestoreNodeRecords validate가 50,000 node에서 0.3 µs, 토큰 Delete·binding Replace·토큰 SetExpression·orphan
  Reconnect validate가 binding 10배에서 1배 안팎입니다(각 행 보고서).
- typography id를 path element·clip 대상으로 쓰는 command는 이제 거부됩니다(validator와 같은 판정).
- `BindingStore`의 live 모음을 바깥에서 직접 바꿀 수 없습니다(`live_mut()` 삭제).
- cache 사용처가 감싼 타입을 쓰도록 바꾸는 앱 연결, OS 압박 알림·UI tick·worker 해제 연결, Metal 할당을 `with_oom_retry`로 감싸는
  연결은 UI 재설계 때 합니다. 그 전까지 엔진 API와 시험만 있습니다.
- 물리 메모리 반환은 제거 pass 길이만큼 늦어집니다.
- 남은 O(N): `InsertNode`가 modifier stack을 넣을 때의 `existing_stacks`, `TokenRegistry::referrers`(토큰 수 비례), 저장 전 전체
  검증과 평가기의 전체 graph.

## Compatibility

- 파일 포맷·FFI·schema 변경 없음.
- 공개 API: `stellar_bindings::orphan::BindingStore` 경로 유지(re-export). `BindingStore`에 `orphan_by_target`·`bindings_reading`·
  `remove_live`(crate 안) 추가. 새 module `stellar_bindings::{store, graph_view}`, `stellar_renderer_core::{cache, cache_adapters}`,
  `stellar_runtime::cache`.
- `DependencyGraph::cycle_if_dependencies`·`TokenDependencyGraph::cycle_if_references`의 판정·경로는 그대로입니다(같은 BFS로 옮김).

## Performance

- `config/performance-budgets.yaml`은 바꾸지 않았습니다. 측정 기준은 각 행 검증 열과 보고서에 있습니다.
- cache 제거 chunk는 2 ms 예산(아키텍처 6.3)을 따릅니다. 부하 상태의 OS 선점은 막지 못하며 따로 기록합니다.

## Security

- 영향 없음. 사용자 내용이 진단·로그로 나가지 않습니다.

## Migration

- 저장 데이터 migration 없음. 코드에서 `BindingStore::live_mut()`를 쓰던 곳(orphan command)은 `insert_live`·`remove_live`로 바꿨습니다.
