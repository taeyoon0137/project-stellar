# ADR-0137: 외부 component 참조를 stellar.yaml에 저장하고 library 제품 명령(add·update·fetch·import)을 둔다

## Status

Accepted (2026-11-24). engine lane `library-materialize`의 결정입니다. ADR-0127(library 제품 열기·적용 경로)의 후속이며
XV.5.2(차이: 문서가 외부 참조를 저장하지 않음), XV.5.5(문서 참조 단위 진단), XV.4.3, XV.7.4, XV.7.5를 다룹니다. ADR-0021(의존성
매니페스트·lock 형식), ADR-0027 결정 7(외부 참조 형식), ADR-0119(library entry 검증·읽기 전용 사전 검사)를 전제로 합니다.

## Context

- 문서 instance payload는 component StableId(`ComponentRef`)만 담습니다. 그래서 instance가 어느 의존성의 component인지
  `stellar_project::library_refs::classify_component`가 **추론**했습니다(로컬 → 직접 의존성 snapshot 가운데 그 StableId를 export한
  것). 추론은 snapshot이 있어야 하므로 offline이거나 아직 받지 않은 library의 instance는 "미해결"이 되어 읽기 전용 사전 검사를 통과하고,
  진단도 나오지 않았습니다(XV.5.2 보고서 차이).
- ADR-0027 결정 7은 외부 참조의 저장 값을 의존성 logical name·LibraryId·resource StableId로 정했지만 저장 위치가 없었습니다. document
  crate는 dependencies crate에 의존할 수 없습니다(crate graph). instance payload 형식을 바꾸면 document codec·텍스트 원본·semantic
  diff·merge가 함께 바뀝니다.
- library 진단은 프로젝트 단위(node 사용 불가)뿐이었고, 문서 참조 단위 생산자(`stellar_integrity::sources::library_refs`)는 호출자가
  없었습니다.
- lock을 쓰고 cache를 채우는 제품 명령이 없었습니다. resolver·mirror·cache entry·lock writer·후보 검증은 모두 있었지만 시험 helper만
  이어 붙였습니다. XV.4.3(lock commit 우선), XV.7.4(승인 뒤 lock·in-memory snapshot 갱신), XV.7.5(실패 시 이전 상태 유지)는 차단
  상태였습니다.
- 열린 창은 lock·cache가 바뀌어도 library를 다시 로드하지 않았습니다(ADR-0127 Consequences).
- CLI `stellar git` 실패 JSON `reasons`에 `CompanionReason::token`이 없었습니다(ADR-0127 Consequences).
- 사용자 방향(2026-11-23): 엔진만 만들고 Swift UI·셸 FFI는 더하지 않습니다. 기능은 Rust 시험·CLI로 검증합니다.

## Decision

### 1. 외부 component 참조는 `stellar.yaml` 의존성 선언의 `components`에 저장한다

```yaml
dependencies:
  ds:
    source: git
    library_id: "0195c530-3acd-7bc1-8414-c213f61cb961"
    repository: "git@example.com:team/ds.git"
    version: v2
    components:
      - "0195c530-3acd-7bc1-8414-c213f61cb9a0"
```

- 한 항목은 `ExternalComponentRef`의 세 값(선언 이름 = 매핑 키, LibraryId = `library_id`, StableId = 항목) 가운데 StableId입니다.
  commit은 저장하지 않고 lock이 정합니다(ADR-0027 결정 7 그대로). update 뒤 문서를 다시 쓰지 않습니다.
- 형식: canonical 소문자 UUID 시퀀스, 중복 금지, 최대 4096개(`MAX_COMPONENT_IMPORTS`). writer는 StableId 오름차순으로 쓰고 비어
  있으면 키를 쓰지 않습니다. 입력 순서는 정규화합니다. 오류는 `InvalidComponentId`·`DuplicateEntry`·`TooManyEntries`입니다
  (`stellar_dependencies::declaration`, `docs/spec/stellar-format.md`).
- `stellar_dependencies::component_imports::ComponentImports`가 선언을 component → 참조 표로 바꾸고, `LibraryGraphInstance`가
  그 표를 들고 다닙니다(`with_imports`·`imports`). `stellar_project::library_open::open_project_libraries`(와 cache 없이 읽는
  `declared_only`)가 사용 프로젝트 `stellar.yaml`을 읽어 붙입니다. lock이 없어도 저장된 참조는 남습니다. 매니페스트를 읽지 못하면
  `LibraryOpenProblem::ManifestInvalid`(진단 `library-manifest/invalid`)입니다.
- 판정(`classify_component`) 순서:
  1. 저장된 참조가 하나면 **외부**(로컬 정의보다 앞섬). 둘 이상의 선언이 같은 StableId를 저장했으면 **모호**.
  2. 저장된 참조가 없고 로컬 정의가 있으면 **로컬**.
  3. 저장된 참조가 없고 직접 의존성 snapshot 하나가 export하면 **저장되지 않은 외부 참조**(`ComponentOrigin::Undeclared`). ADR-0137
     이전 프로젝트의 호환 읽기이고, 사전 검사는 외부와 같이 판정합니다(fail closed). 진단이 저장을 알립니다. 둘 이상이면 모호.
  4. 아니면 **미해결**(문서 validator 몫).
- 저장된 참조는 snapshot이 없어도 외부입니다. offline에서 그 instance의 override 설정은 `LibraryUnavailable`로 거부됩니다(reset은
  허용). 사전 검사를 건너뛰는 조건은 "snapshot도 저장된 참조도 없음"(`stellar_ops::library::has_external`)입니다.
- 한 프로젝트에서 같은 component StableId는 한 의존성에서만 씁니다. 같은 library를 두 이름으로 선언해 같은 component를 instance별로 다른
  버전에서 쓰는 것은 표현하지 않습니다(모호 진단).

### 2. 문서 참조 단위 진단(origin `library-refs`)

`stellar_project::library_refs::document_ref_issues`가 문서 instance마다 출처(결정 1)와 해석(`LoadedLibraries::resolve`)을 보고,
해석된 instance의 override는 후보 검증과 같은 `check_candidate_usages`로 현재 snapshot에 대어 봅니다. `stellar_ops::library::
document_ref_contribution`이 진단으로 바꿉니다. 사용할 수 없는 library의 참조는 integrity 생산자 `sources::library_refs` 그대로입니다.

| 문제 | code | 종류 | 심각도 |
|---|---|---|---|
| 사용할 수 없는 library(offline·손상) | `library-resource-unavailable` | MissingLibrary | 경고 |
| 선언만 있고 lock에 없음 | `library-ref/not-locked` | MissingLibrary | 경고 |
| 사라진 component·다른 library·다른 종류 | `library-ref/unresolved` | MissingComponent | 오류 |
| 저장되지 않은 참조(호환 읽기) | `library-ref/undeclared` | MissingLibrary | 경고 |
| 모호한 참조 | `library-ref/ambiguous` | MissingLibrary | 오류 |
| 현재 library에서 orphan인 override | `library-ref/orphan-override` | MissingLibrary | 경고 |

source는 instance, property path는 instance root, footprint는 `sources::library_refs`와 같은 규칙(instance node, 모든 key, node
삽입)입니다. 진단 kind 표·codec은 바꾸지 않습니다. 호출자: `stellar_ops::integrity::diagnose_snapshot`(CLI `stellar diagnose`·MCP
`document.diagnostics`, `stellar_ops::library::submit_snapshot`)과 창 session(`WindowLibraries::document_diagnostics`).

### 3. lock commit 우선 해석(XV.4.3)

`stellar_dependencies::lock_preference::LockPreferringResolver`가 선택자 해석기를 감쌉니다. 기존 lock entry 가운데 같은 repository
비교 키·같은 선언 선택자의 exact commit을 Git 없이 돌려줍니다. 같은 key가 서로 다른 commit을 가리키면(손상) 고르지 않습니다. update
대상은 `refresh`로 lock 우선을 끕니다. 매니페스트는 `stellar_project::library_resolve::CachedManifestSource`가 cache entry의
`stellar.yaml`을 먼저 읽고, 없을 때만 fetch한 mirror를 읽습니다. cache에 있는 commit은 `materialize`가 index에서 찾으므로 선택자가
바뀌지 않은 의존성은 network 없이 graph·lock을 다시 만듭니다.

### 4. 제품 명령(`stellar_ops::library_materialize`, CLI `stellar library`)

| 명령 | 하는 일 |
|---|---|
| `add <경로> <이름> <repository> (--tag\|--branch\|--commit) <값> [--component <id>]...` | 선택자를 해석해 그 commit의 `project.id`로 library id를 정하고 선언을 더한 뒤 graph 전체를 해석·받기·검증하고 lock을 씁니다. `--component`는 새 library가 export해야 합니다 |
| `update <경로> <이름> [선택자] [--allow-orphans] [--dry-run]` | 그 의존성만 다시 해석(선택자 생략 시 branch 이동 확인)하고, 저장된 모든 문서의 외부 instance를 후보에 대어 봅니다(XV.7.3). 끊기는 참조가 있으면 승인(`--allow-orphans`) 없이 거부합니다. 결과에 이전·새 commit과 resource diff 개수(XV.7.2)를 냅니다 |
| `fetch <경로>` | lock의 모든 node를 공용 cache에 받고 검증합니다. 받은 content hash가 lock 값과 다르면 실패합니다(XV.4.4). lock·`stellar.yaml` 불변 |
| `import <경로> <이름> <component id>... [--remove]` | 저장된 참조를 더하거나 지웁니다. 더할 때는 lock 기준으로 cache에서 연 그 의존성 library가 export해야 합니다(Git 없음) |

- 한 명령 안에서 선택자 해석·매니페스트·materialize가 mirror 하나를 같이 씁니다(저장소마다 fetch 한 번).
- 받은 entry는 `load_cached_snapshot` + `ProjectLibraryValidator::cache_entry`·`SourceExports`로 검증합니다(일반 Git 저장소·손상 거부).
- `stellar.yaml`은 `ManifestEdit`(`stellar_project::library_resolve`)로 씁니다: 연 시점 bytes hash가 그대로일 때만 쓰고(`write_checked`),
  쓰기 전에 다시 읽혀 같은 모델인지 확인합니다. 텍스트는 매니페스트 canonical writer 출력입니다(style defaults 선택과 같은 경로).
- 공용 cache는 OS 사용자 데이터 위치이고 첫 materialize 때 디렉터리와 keyer key를 만듭니다(`LibraryCache::create_os_default`,
  ADR-0127 결정 1). cache가 사용 프로젝트 안이면 거부합니다.
- 프로젝트 Git 신뢰를 요구하지 않습니다. 이 명령은 사용 프로젝트에서 Git을 실행하지 않고 공용 cache의 bare mirror만 씁니다(template
  없는 init, hook 없음).

### 5. 원자성(XV.7.4·XV.7.5)

resolve·fetch·materialize·entry 검증·후보 검증·텍스트 생성을 **파일을 쓰기 전에** 끝냅니다. 하나라도 실패하면 `stellar.yaml`·lock은
그대로입니다. 공용 cache에는 새 entry가 남을 수 있지만 immutable·content-addressed라 이전 lock이 가리키는 entry를 바꾸지 않습니다.
쓰기는 `stellar.yaml` → lock 순서이고 lock 쓰기가 실패하면 `stellar.yaml`을 연 시점 bytes로 되돌립니다(그 사이 다른 writer가 바꿨으면
되돌리지 않고 `RestoreFailed`). 두 파일을 한 rename으로 바꾸는 방법은 없으므로 이 순서와 되돌리기로 좁힙니다.

### 6. 열린 창은 lock·매니페스트·cache 지문이 바뀌면 다시 로드한다

`stellar_ops::library_watch::LibraryStamp`는 lock·`stellar.yaml`·cache `repository-key`·`entries/` 디렉터리의 metadata(크기·수정
시각·inode)이고, 받지 못한 node가 있을 때만 cache index도 봅니다(index는 entry를 열 때마다 최근 사용 시각을 기록하므로 완전한 로드에서
보면 다른 창이 열 때마다 다시 로드함). `stellar_runtime::library::WindowLibraries`는 위치를 아는 경우(`open`·`open_in_app_data`) 쓸
때마다(상태·진단·instance command 사전 검사) 지문을 비교하고 다르면 그 자리에서 다시 로드합니다. instance command가 아닌 편집은 지문을
보지 않습니다. 창 session이 `Arc`로 빌려 주므로 `&self`로 다시 로드하고, 결과를 세대 목록에 덧붙입니다(지난 세대는 창이 닫힐 때까지 유지,
lock·cache 변경 횟수만큼만 늘어남). instance override는 property StableId로 저장되어 있어 새 snapshot에 그대로 다시 대어집니다.
`open_with`(cache 참조를 받던 생성자)는 위치를 모르므로 `open_in_app_data`로 바꿉니다. 이미 연 결과를 쓰는 `fixed`는 다시 로드하지
않습니다.

### 7. CLI `stellar git` 실패 JSON `reasons`에 `token`

`companion-required` 결과의 `reasons[]`가 `{unit, component, token}`입니다. `token`은 token 의존이면 끊기는 token StableId, 정의 참조면
`null`입니다.

## Alternatives

- **instance payload에 `ExternalComponentRef`를 저장**: 참조가 instance마다 정확하지만 document crate가 dependencies에 의존할 수 없고,
  payload codec·텍스트 원본·semantic diff·merge·MCP schema를 함께 바꿔야 합니다. 저장 값(ADR-0027 결정 7)은 같으므로 프로젝트 단위 표가
  instance마다 이름을 반복하지 않고 Git diff도 선언 한 곳에 모입니다. 같은 component를 instance별로 다른 버전에서 쓰는 경우만 표현하지
  못합니다.
- **별도 파일(`design/library-refs.stellar.yaml`)**: 텍스트 원본 loader·Git 동반 단위·cache 계산이 모르는 파일이 생깁니다. 의존성
  선언 옆에 두는 편이 사람·AI가 함께 고치기 쉽습니다.
- **추론을 없애고 저장된 참조만 인정**: ADR-0137 이전 프로젝트의 외부 instance가 모두 미해결이 되어 읽기 전용 사전 검사가 풀립니다. 호환
  읽기를 남기고 진단으로 저장을 알립니다.
- **창 재로드를 window session 관찰 루프에서**: 창 session 파일을 고쳐야 하고 UI 재설계 전에는 관찰 주기가 정해지지 않았습니다. 쓰는
  시점 비교는 관찰 주기와 무관하게 맞습니다. UI 재설계 때 관찰 루프가 `WindowLibraries::refresh`를 불러도 같은 결과입니다.
- **update를 `prepare_library_update`(XV.7.1)로**: 그 함수는 의존성 하나의 현재·후보 snapshot만 만들고 lock 전체를 만들지 않습니다. 전체
  graph를 한 번 해석하고 후보 판정은 같은 `check_candidate_usages`를 씁니다(이중 판정 없음).

## Consequences

- 저장된 참조가 있는 instance는 offline에서도 외부로 판정되어 override 설정이 거부되고, 문서 참조 진단이 사용할 수 없음을 냅니다.
- `stellar diagnose`·MCP `document.diagnostics`·창 session이 같은 문서 참조 진단을 냅니다. 저장된 참조가 없는 기존 프로젝트는
  `library-ref/undeclared` 경고가 생깁니다.
- `stellar library update`는 끊기는 참조를 승인 없이 적용하지 않습니다. 실패한 명령은 `stellar.yaml`·lock을 바꾸지 않습니다.
- MCP에는 library 명령 도구를 더하지 않았습니다(mcp-core는 이 lane 소유 밖). 같은 함수를 부르는 도구를 더하는 일이 남았습니다.
- 앱 UI(승인 화면, 진단 표시)는 UI 재설계 때 연결합니다.

## Compatibility

`stellar.yaml` 형식에 선택 키 `components`를 더합니다(없으면 이전과 같은 bytes). 이전 reader는 모르는 키로 거부하므로 `components`를
쓴 프로젝트는 이 버전 이상이 필요합니다. lock 형식·generation manifest·C ABI·FFI 헤더·보호 경로는 바꾸지 않습니다. 공개 API 추가:
`DependencyDeclaration::components`, `ComponentImports`, `LibraryGraphInstance::{with_imports, imports}`, `LockPreferringResolver`,
`ValueError::{InvalidComponentId, DuplicateEntry, TooManyEntries}`, `ComponentOrigin::Undeclared`, `DocumentRefIssue`,
`document_ref_issues`, `library_resolve`, `LibraryOpenProblem::ManifestInvalid`, `stellar_ops::library_materialize`,
`stellar_ops::library_watch`, `ProductLibraries::{loaded, has_external, document_contribution, submit_document}`, `submit_snapshot`,
`LibraryCache::{create_os_default, keyer}`, `WindowLibraries::{open_in_app_data, fixed, refresh, reloads, document_diagnostics}`.
`WindowLibraries::open_with`는 `open_in_app_data`로 바꿉니다(제품 호출자 없음). `ExternalUsages`에 `undeclared` 필드가 생깁니다.

## Performance

열기·진단·사전 검사는 `stellar.yaml`을 한 번 더 읽습니다(loader 상한 1 MiB 이하의 작은 파일). 창의 지문 비교는 instance command·진단·상태
조회 때 파일 4\~5개 metadata를 읽고, 바뀌었을 때만 entry 검증 비용(ADR-0119 Performance)을 냅니다. 인터랙션 예산(§6)의 일반 편집 경로는
지문을 보지 않습니다. 문서 참조 진단은 문서 instance 수 × override 수에 선형입니다. 제품 명령은 network·materialize 비용이고 인터랙션
경로가 아닙니다.

## Security

명령 결과·오류·진단에 repository URL·cache 경로를 넣지 않습니다(XV.1.4). `stellar.yaml`·cache entry 매니페스트는 symlink를 따르지 않고
loader 상한까지만 읽습니다. mirror는 template 없는 bare 저장소이고 사용 프로젝트에서 Git을 실행하지 않습니다. `stellar.yaml`은 읽은
bytes가 그대로일 때만 씁니다.

## Migration

기존 프로젝트는 그대로 열립니다. 저장된 참조가 없는 외부 instance는 호환 읽기로 판정되고 `library-ref/undeclared` 진단이 납니다.
`stellar library import <경로> <이름> <component id>...`로 참조를 저장합니다. 기존 시험 cache는 시험 keyer를 그대로 씁니다.
