# ADR-0127: 외부 library를 제품 열기·적용 경로에 잇고 token set을 동반 staging 단위로 더한다

## Status

Accepted (2026-11-24). engine lane `library-wire`의 결정입니다. XV.5.5, XV.5.3, XXV.2.1(library 진단 연결), XIV.14.3,
XIV.14.4를 다룹니다. ADR-0119(library entry 검증·읽기 전용 사전 검사), ADR-0095(무결성 진단 index), ADR-0080(Git 엔진·동반
단위), ADR-0088·ADR-0113(텍스트 원본과 chunk cache)을 전제로 합니다.

## Context

- ADR-0119는 `stellar_project::library_open::open_project_libraries`(lock 기준 부분 열기)와
  `stellar_project::library_guard::check_component_edits`(읽기 전용 사전 검사)를 만들었지만 제품 경로(창 session, CLI·MCP
  열기·적용)가 부르지 않았습니다. 비판 검토가 XV.5.5·XV.5.3을 `[~]`로 둔 이유입니다.
- 두 함수는 cache entry store(`LibraryEntryStore<H, K>`)를 받습니다. 제품 repository keyer가 없었습니다(mirror 문서는 "로컬 비밀
  key 기반 keyed hash"를 요구). 그래서 CLI·MCP·앱이 공용 cache를 열 방법이 없었습니다.
- 사용 프로젝트에서 instance override를 바꾸는 제품 command는 `Command::SwapInstance`(캔버스 override Set·Reset·교체,
  ADR-0118 결정 5)와 override를 가진 instance의 `Command::InsertNode`입니다. document의 `PropertyOverrideCommand`는 closed
  command enum에 없습니다. `SwapInstance`는 component library 없이 검증하므로 override 규칙을 보지 않습니다.
- `stellar-ops`는 `stellar-dependencies`에 의존하지 않았습니다(crate graph). 진단 수집 `diagnose_snapshot`은 library cache가
  없어 library 생산자를 실행하지 않았습니다(ADR-0095 결정 7).
- 컴포넌트 텍스트 원본은 `local-tokens`(token StableId 목록)를 저장하고 token은 `design/tokens/<slug>.stellar.yaml`에
  있습니다. 동반 단위 계산(`companion_closure`)은 컴포넌트 정의만 봤습니다(XIV.14.3 남은 범위).
- 텍스트 원본 conflict 세션 단위(ADR-0113 결정 5)가 생겼지만 XIV.14.4 시나리오는 generation 저장 프로젝트로만 있었습니다.
- 사용자 방향(2026-11-23): 엔진만 만들고 Swift UI·셸 FFI는 더하지 않습니다.

## Decision

### 1. 제품 keyer와 공용 cache 위치

`stellar_ops::library`가 제품 진입점입니다.

- cache root: OS 사용자 데이터(`stellar_project::trust_store::os_user_data_base`) 아래 `xyz.taeyoon.stellar/library-cache`
  (저장 잠금·복구본과 같은 앱 폴더, `stellar_ops::lock::APP_DATA_DIR`). `LibraryCache::os_default`가 엽니다.
- keyer(`ProductKeyer`): cache root의 `repository-key`(32 bytes, OS 난수, 소유자 전용 `0600`, symlink·길이 다른 파일은 손상)로
  repository 비교 키의 HMAC-SHA256(저장 content hash와 같은 SHA-256)을 계산하고 앞 16 bytes(32 hex)를 씁니다. 같은 비교 키는
  같은 key이고 URL만으로 mirror 이름을 추측할 수 없습니다. `Debug`는 key를 출력하지 않습니다.
- key는 materialize 쪽(`LibraryCache::create_in_app_data`)만 만듭니다. 읽기 쪽은 key 파일이 없으면 저장하지 않는 일회용 key로
  열어 모든 node가 `MissingCacheEntry`가 되고 파일을 만들지 않습니다.

### 2. 문서를 연 뒤 library를 로드하고 진단 index에 싣는다

`open_libraries(root, cache, now)`는 lock을 먼저 읽습니다. lock이 없으면 cache를 건드리지 않습니다. cache를 열지 못했으면 lock에
의존성이 있을 때만 그 문제를 남깁니다. 나머지는 `open_project_libraries`입니다(실패하지 않음). `ProductLibraries::contribution`은
프로젝트 단위 진단 기여 하나(origin `libraries`)입니다.

| 내용 | code | 심각도 |
|---|---|---|
| 사용할 수 없는 library node | `sources::libraries`(기존 `library-unavailable/*`) | 경고 |
| lock 읽기·해석·구조 오류 | `library-lock/unreadable`·`library-lock/invalid`·`library-lock/structure` | 오류 |
| cache를 열지 못함(사용자 데이터 위치·key) | `library-cache/unavailable` | 경고 |

종류는 모두 `MissingLibrary`입니다(진단 kind 표·codec 불변). 호출자:

- CLI `stellar diagnose`와 MCP `document.diagnostics`: 둘이 같이 쓰는 `stellar_ops::integrity::diagnose_snapshot`이
  `open_libraries_default(snapshot.root()).submit_to(engine)`을 부릅니다. ADR-0095 결정 7의 "library cache 생산자는 실행하지 않음"을
  이 결정으로 바꿉니다.
- 창 session: `ProjectWindowSession::load`가 문서를 연 뒤 `reload_libraries`로 `stellar_runtime::library::WindowLibraries`를
  만들고 `libraries()`로 냅니다. `WindowLibraries::diagnostics`는 위와 같은 기여입니다. 앱 진단 index·표시 연결은 UI 재설계 때
  합니다. 열린 창은 lock·cache 변경 때 자동으로 다시 로드하지 않습니다(`reload_libraries`를 부르는 쪽이 정함).

ProjectSnapshot에는 library를 싣지 않습니다. entry 검증이 모든 파일을 해시하므로(ADR-0119 Performance) 진단·적용처럼 필요한
경로만 엽니다.

### 3. instance override를 바꾸는 command는 적용 전에 같은 사전 검사를 거친다

`stellar_ops::library::check_commands(own, libraries, model, commands)`는 command마다 instance override 변화를 document command로
옮겨 `check_component_edits` 하나로 판정합니다(판정 규칙은 ADR-0119 결정 4 그대로).

- `SwapInstance`: 직전 payload(같은 transaction 앞 command 결과, 없으면 문서)와 비교해 같은 component면 추가·변경된 직접 override는
  `Set`, 사라진 직접 override는 `Reset`입니다. component가 바뀌면 새 payload의 직접 override가 모두 새 component에 대한 `Set`입니다.
- `InsertNode`: 삽입 record의 instance payload 직접 override가 모두 `Set`입니다.
- 바뀌지 않은 override는 넣지 않습니다(update 뒤 orphan이 된 override가 다른 편집을 막지 않음). nested 경로 override, 복제·외부
  snapshot 반영·record 복원은 이미 있는 payload를 옮길 뿐이라 넣지 않습니다(ADR-0119 결정 5와 같은 범위).
- 판정 대상 instance만 담은 1-node scene(`NodeColumns` + payload)을 만들어 넘깁니다. 각 instance가 command 뒤에 가리킬 component로
  분류되므로 교체·삽입도 같은 함수로 판정합니다.
- 로드한 snapshot이 하나도 없으면 외부 component가 있을 수 없어 건너뜁니다.

호출자:

- headless 적용 `stellar_ops::apply::apply`(CLI `stellar apply`, MCP 편집): command 해석 뒤 transaction 실행 전에
  `guard_transaction`이 판정하고 거부를 `ApplyError::Rejected { index, reason }`(입력 위치)로 돌려줍니다. 대상 command가 있을 때만
  library를 엽니다. 자기 component library를 읽지 못하면 빈 library로 봅니다(fail closed: library와 같은 StableId의 로컬
  component만 외부로 판정).
- 창 session: `apply`·`apply_grouped`·`insert_and_select`·`insert_grouped_and_select`·`apply_and_select`가 history 기록 전에
  `WindowLibraries::check`를 부르고 거부를 `WindowSessionEditError::Library`로 돌려줍니다. 문서·history는 바뀌지 않습니다. Undo·Redo와
  외부 snapshot 반영은 이미 있던 상태로 되돌리는 것이라 검사하지 않습니다. 자기 component library는 창이 읽은 원본 library,
  없으면(cache로 빠르게 연 경우·chunk 전용) 디스크의 `project_library`입니다.

### 4. 컴포넌트 정의 → token set을 필수 동반 단위로 계산한다

동반 단위 계산을 `stellar_ops::git::companion`으로 옮기고 token 의존을 더합니다. 후보는 "단위에 속한 경로는 원천(stage: 작업 트리,
unstage: HEAD), 나머지는 기준 index"입니다.

- 후보 컴포넌트 원본의 `local-tokens` token이 후보 token set 원본에 없으면: 원천에서 그 token을 담은 token set 파일이 동반입니다.
  원천 token set에도 없고 원천 컴포넌트가 그 참조를 뺐으면 그 컴포넌트 파일이 동반입니다.
- 원천에서도 끊긴 참조(프로젝트 token set 밖의 token, 작업 트리 자체가 끊긴 상태)와 기준 index에서 이미 끊긴 참조는 막지 않습니다.
  semantic dependency graph(XIV.5.2)가 같은 diff 안에서 추가·제거되는 대상만 의존으로 보는 것과 같은 범위입니다.
- `CompanionReason`에 `token: Option<StableId>`를 더합니다. token 이유면 `component`는 그 token을 참조하는 정의입니다.
- token set 원본은 정규 파일만, YAML loader 상한까지 읽고 제품 codec(`source_v4::token::decode`)으로 해석합니다. 읽지 못하면
  `PrepareStageError::TokenSource`입니다(fail closed).

### 5. 텍스트 원본 branch conflict 시나리오

XIV.14.4에 텍스트 원본 프로젝트 시나리오를 더합니다. merge driver 없이 시스템 Git `git merge`로 View 원본 텍스트 conflict를 만들고
제품 conflict 세션(`ConflictSession` 텍스트 원본 단위) → Semantic 해결 → `apply_session`(`git add`)까지 실행해 unmerged 0, merge
state 유지, conflict marker 없음, 결과 의미를 확인합니다. 제품 코드는 바꾸지 않습니다.

### 6. crate graph

`stellar-ops`의 허용 직접 의존에 `stellar-dependencies`를 더합니다(xtask `crate_graph`, `docs/architecture/crate-dependency-graph.md`).
dependencies는 core-types에만 의존하므로 금지 전이 도달이 생기지 않습니다. runtime은 `stellar_ops::library` 재수출만 씁니다.

## Alternatives

- **keyer를 비밀 없는 SHA-256으로**: 공개 저장소 URL은 이름을 다시 계산해 cache에 어떤 저장소가 있는지 알 수 있습니다(XV.1.4
  privacy). 시험 keyer와 같은 방식이라 운영 keyer로 쓰지 않습니다.
- **ProjectSnapshot을 열 때 항상 library 로드**: 모든 CLI 명령이 entry 전체 해시 비용을 냅니다. 진단·적용만 엽니다.
- **PropertyOverrideCommand를 closed command enum에 추가**: commands·codec·MCP schema를 바꾸는 큰 변경입니다. 제품 편집은 이미
  `SwapInstance`로 기록되므로 그 변화를 같은 판정 함수의 요청으로 옮깁니다.
- **적용 뒤 결과 비교로 판정**: headless는 가능하지만 창 session은 history에 기록한 뒤 되돌려야 합니다. 두 경로가 같은 적용 전
  판정을 쓰도록 했습니다.
- **token 의존을 semantic dependency graph로만**: 원본 단위(File) 선택에는 변경 ID graph가 없습니다. 파일 단위 동반 계산에 같은
  범위의 규칙을 둡니다.

## Consequences

- `stellar diagnose`·MCP `document.diagnostics`·창 session이 같은 library 진단을 냅니다. lock이 있는 프로젝트의 진단 수집은 cache entry
  검증 비용을 냅니다.
- 사용 프로젝트에서 library component의 비공개·없는·타입이 다른 property override는 headless·창 모두 적용 전에 거부됩니다.
- token set을 빠뜨린 컴포넌트 stage·unstage는 동반 승인 전까지 index를 바꾸지 않습니다. CLI JSON의 `reasons`에는 아직 `token`
  필드가 없습니다(출력 형식 변경은 CLI 소유 작업).
- 열린 창은 lock·cache 변경을 자동으로 따라가지 않습니다.

## Compatibility

파일 포맷·lock 형식·generation manifest·C ABI·FFI 헤더를 바꾸지 않습니다. 보호 경로를 바꾸지 않습니다. 사용자 데이터 cache root에
`repository-key` 파일이 생깁니다(materialize 쪽). 공개 API 추가: `stellar_ops::library`, `stellar_runtime::library`,
`ProjectWindowSession::{libraries, reload_libraries, set_libraries}`, `WindowSessionEditError::Library`,
`PrepareStageError::TokenSource`, `CompanionReason::token`.

## Performance

창 열기와 lock이 있는 프로젝트의 진단 수집·instance command 적용은 entry마다 파일 전체 SHA-256과 chunk 읽기를 한 번 더 합니다
(ADR-0119 Performance). 창 열기는 문서를 연 뒤 한 번이고 인터랙션 예산(§6) 경로가 아닙니다. 창 적용 사전 검사는 snapshot이 있고
instance command가 있을 때만 1-node scene과 판정을 만듭니다. 동반 계산은 token set 파일 수와 컴포넌트 `local-tokens` 수에 선형입니다.

## Security

key 파일은 소유자 전용이고 symlink를 따르지 않으며 key·cache 경로·URL을 오류·`Debug`·진단에 넣지 않습니다. token set 원본은 symlink를
따르지 않고 loader 상한까지만 읽습니다.

## Migration

없음. 기존 시험 cache는 시험 keyer를 그대로 씁니다. 제품 cache는 첫 materialize 때 key를 만듭니다.
