# ADR-0119: 외부 library entry 검증·읽기 전용 사전 검사·update 전 검증을 제품 경로에 잇는다

## Status

Accepted (2026-11-23). engine lane `library-finish`의 결정입니다. XV.3.4, XV.4.2, XV.4.4, XV.5.3, XV.5.5, XV.6.2, XV.7.3,
IX.9.1, IX.9.2, XIII.4.4, XIII.5.5를 다룹니다. ADR-0021(lock 파일), ADR-0022(cache entry content address), ADR-0027(외부
참조), ADR-0088(텍스트 원본과 chunk cache)을 전제로 합니다.

## Context

- `stellar-dependencies`는 core-types에만 의존합니다. 그래서 cache entry 검증(XV.3.4)·export 읽기(XV.5.1)·Git 실행·SHA-256을
  trait으로만 두었고, 제품 구현이 없었습니다. 판정 정책(entry 거부, lock 무결성, 읽기 전용 정책, 부분 열기)은 fake 검증기로만
  시험했고 제품 경로가 부르지 않았습니다.
- `load_cached_snapshot`은 lock content hash와 cache index 값만 비교했습니다. entry 이름이 content hash라서 이 비교는 lock이
  다를 때만 실패하고, entry 파일 bytes가 바뀐 손상은 잡지 못했습니다. entry 검증(ProjectProbe·storage validator)도 update
  후보에서만 불렸습니다.
- materialize한 cache entry에는 `.git`이 없어 `probe_project`(IV.5.1, `.git` 요구)를 그대로 쓸 수 없습니다.
- 텍스트 원본(ADR-0088)은 Property·Slot·상태 머신을 가진 컴포넌트 정의를 아직 쓰지 못합니다. 그런 정의는 generation의
  component library chunk(`KIND_COMPONENT_LIBRARY` schema 1)에 있습니다.
- 문서 원본은 instance payload에 외부 참조(선언 이름 + LibraryId, ADR-0027)를 저장하지 않습니다(XV.5.2 차이). document·commands
  crate는 dependencies에 의존할 수 없습니다(crate graph).
- 사용자 방향(2026-11-23): 엔진만 만들고 Swift UI·셸 FFI는 더하지 않습니다.

## Decision

### 1. cache 읽기 경로 하나가 entry를 검증한다

`load_cached_snapshot`(lock 기준 열기 `load_locked_libraries`와 update 후보 `CacheUpdateSource`가 함께 씀)은 entry 공유 read
lock을 잡은 채 다음 순서로 검사하고, 모두 통과한 entry에서만 export를 읽습니다.

1. 기대 content hash(lock 값)와 entry 주소 비교(기존).
2. 디스크 bytes에서 content hash를 다시 계산해 entry 주소와 비교합니다(`LibraryEntryStore::verify_entry_bytes`). 다르거나
   symlink 같은 예상하지 않은 항목이 있으면 `SnapshotLoadError::Corrupted`입니다(XV.4.4).
3. `validate_library_entry`(ProjectProbe → library id → storage validator, XV.3.4). 거부는 `SnapshotLoadError::Rejected`입니다.

어느 실패에서도 다른 commit의 entry를 찾지 않습니다. 부분 열기 진단(`UnavailableReason`)은 새 variant를 만들지 않고 기존 값으로
옮깁니다: bytes 손상은 lock content hash 불일치와 같은 손상이므로 `ContentHashMismatch`, 검증 거부는
`ExportUnreadable(ExportReadFailure::InvalidLibrary(..))`입니다. 진단 index(`stellar-integrity`)의 코드 표를 바꾸지 않기 위해서입니다.

제품 검증기는 `stellar_project::library_entry::ProjectLibraryValidator`입니다.

| root | probe | storage validator |
|---|---|---|
| cache entry | `stellar.yaml` 정규 파일(symlink 거부)을 loader 상한까지 읽어 제한 loader·매니페스트 스키마로 해석(`.git` 요구만 뺀 IV.5.1과 같은 parser) | 아래 공통 |
| Git 작업 폴더(local override) | `probe_project` 그대로 | (local override 검증은 probe·id까지, XV.6.2) |

storage validator: `design/current`가 있으면 current pointer → manifest(hash 주소·project id) → **모든 chunk를 읽어**
envelope·payload hash를 확인하고 component library chunk(하나까지, schema 1)를 해석합니다. 텍스트 원본은
`design/document.stellar.yaml`이 있으면 원본 파일 집합 전체를, 없으면 `design/components/`만 해석합니다. generation과 텍스트
원본이 둘 다 컴포넌트 정의를 담으면 손상입니다(`source_v4::project::from_saved`와 같은 정책). 저장 데이터가 없는 library는
export가 없는 정상 library입니다.

시스템 Git 실행(`SystemGitCommands`, git-porcelain `GitRunner`)과 SHA-256(`StorageSha256`, 저장 content hash와 같은 함수)도
`stellar_project::library_git`에 제품 구현을 둡니다.

### 2. export는 library 저장 데이터에서 같은 codec으로 읽는다

`stellar_project::library_exports::SourceExports`가 `LibraryExportSource`의 제품 구현입니다. 컴포넌트 정의는 generation
component library chunk 또는 텍스트 원본(결정 1과 같은 함수), token set은 텍스트 원본에서 읽습니다. property default
fingerprint는 default 표현식 canonical bytes, token 값 fingerprint는 `TypedValue::to_bytes`입니다. 문서 원본이 외부 참조를
저장하지 않으므로 export의 외부 참조 목록은 비어 있습니다. 개수 상한은 만드는 동안 확인합니다.

### 3. 타입 표기는 `TypeSpec` 표시 형식이고 `Optional<T>`는 `T` 값을 받는다

export property 타입 표기는 `TypeSpec`의 표시(`T`, `Optional<T>`), override·token 값 타입 표기는 그 값의 비-nullable `T`입니다
(`value_type_signature`). override 허용 판정(`override_type_accepted`)은 `T`가 `T`와 `Optional<T>` property 모두에 들어가는
`TypeSpec::accepts`와 같은 규칙입니다. 사전 검사(결정 4)와 update 전 검증(결정 5)이 이 함수 하나를 씁니다.

### 4. 사용 프로젝트 component command는 적용 전에 읽기 전용 사전 검사를 거친다

`stellar_project::library_guard::check_component_edits`는 document command(`PropertyCommand`, `PropertyOverrideCommand`)를
형식 중립 요청으로 바꿔 `check_library_edits` 하나로 판정합니다. 정의·property 정의 편집은 거부, override 설정은 공개·타입
일치만 허용, reset과 그 inverse(`Restore`)는 instance 쪽 sparse 값만 바꾸므로 항상 허용합니다. 한 건이라도 거부되면 전체를
거부합니다.

대상 component가 외부인지는 `library_refs::classify_component` 하나로 정합니다. 문서가 외부 참조를 저장하기 전까지의 규칙입니다.

1. 사용 프로젝트 자신의 ComponentLibrary에 있으면 로컬(통과, command 자신이 검증).
2. 아니면 lock root scope가 직접 선언한 의존성 snapshot 가운데 그 StableId를 export한 것이 정확히 하나면 그 선언 이름·LibraryId의
   외부 참조.
3. 둘 이상이면 모호 — 거부합니다(fail closed).
4. 어디에도 없으면 미해결(offline 등) — library 편집으로 판정할 수 없으므로 command 자신의 검증에 맡깁니다. 이때 reset은
   허용되고 override 설정은 정의가 없어 command가 실패합니다.

문서가 외부 참조를 저장하게 되면 2·3단계만 저장된 참조로 바뀌고 사전 검사·후보 검증은 그대로 씁니다.

### 5. 후보 library 검증은 사전 검사와 같은 판정으로 orphan 진단을 낸다

`stellar_dependencies::candidate_check::check_candidate_usages`는 사용 프로젝트의 외부 instance 사용(instance, 외부 component,
direct override와 값 타입)과 외부 token 사용을 후보 snapshot에 대어 봅니다. override마다 `check_library_edit`로 override 설정
요청을 판정하고 거부 사유를 orphan 원인으로 옮깁니다(`UnknownProperty` → 사라짐, `PrivateProperty` → 비공개,
`OverrideTypeMismatch` → 타입 변경). component가 없으면 instance 전체가 orphan 참조입니다. 검증은 문서·lock을 바꾸지 않고 값을
지우지 않습니다(IX.4.6 정책). `LibraryUpdateCandidate::check_usages`와 `library_refs::check_update_candidate`(문서에서 사용 수집)가
진입점입니다. nested 경로 override는 외부 component의 직접 property가 아니므로 사용에 넣지 않습니다(IX.4.2).

### 6. 프로젝트 library 열기는 실패하지 않는다

`stellar_project::library_open::open_project_libraries`는 `stellar.lock.yaml`을 loader 상한까지 읽어 제품 lock parser로 검증하고
결정 1의 검증기·결정 2의 공급자로 `load_locked_libraries`를 부릅니다. Git·network를 쓰지 않습니다. lock이 없으면 의존성 없는
프로젝트, lock 읽기·해석·구조 오류는 `LibraryOpenProblem`, node 단위 문제는 unavailable 진단입니다. 문서 열기(창 session·CLI·MCP)는
이 함수와 독립이며, 열기 경로는 문서를 연 뒤 이 결과로 외부 참조를 해석합니다.

### 7. local override는 실제 Git 작업 폴더 probe로 검증한다

`library_overrides::validate_project_local_overrides`는 선언 매핑(`local_overrides`)과 대상 검증(`validate_local_override`)을
`ProjectLibraryValidator::checkout`(`probe_project`)으로 실행합니다. `stellar.yaml`이 없는 Git 저장소와 `.git`이 없는 폴더는
Stellar 프로젝트가 아니고, 결과는 override마다 따로 돌려줍니다.

### 8. 합성 fixture와 10단계 drilling fixture는 통합 시험으로 둔다

- IX.9.1: document test-support 합성 fixture scene을 영속 레코드(`NodePayload::ComponentInstance`)로 옮겨 override 규칙·instance
  펼침을 확인합니다. 제품 문서 validator는 instance가 slot에 투영하는 instance 소유 자식(IX.3)을 `ParentNotContainer`로 거부합니다.
  validator는 document crate(이 lane 소유 밖)라 고치지 않고, "document validator 0 오류" 시험은 사유를 단 `#[ignore]`로 둡니다.
- IX.9.2: 11개 컴포넌트를 10단계로 중첩하고 제품 drilling 계획·적용으로 가장 안쪽 property를 최상위로 노출합니다. 출처 chain은 제품
  drilling 모음 조회(`link_into`)로 scene override부터 가장 안쪽 property까지 따라갑니다. drilled 값을 계산해 provenance 기록을 내는
  제품 evaluator는 없습니다(document·bindings 소유).

### 9. Update from Source의 AssetRecord 변경은 assets의 closed enum command가 적용한다

`stellar_assets::record_command::AssetRecordCommand::Update`가 validate(AssetId 존재, 현재 record = `previous`, `next` content가
저장소에 실제로 있음)·apply(inverse command 반환)를 갖습니다. 원본이 없으면 `Update from Source`가 command를 만들지 않습니다.
`stellar-commands` closed enum·창 history 등록은 그 crate의 작업입니다(document component command가 IX.2.4에서 같은 방식).

### 10. 외부 symlink target 감시는 runtime 감시 thread가 맞춘다

XIII.4.4의 제품 경로는 `stellar-runtime`의 외부 변경 감시(`app/autosave/external.rs`)가 처음 열 때와 넘침·링크 변화 event에서
`ExternalTargetSync::sync`를 부르는 것입니다(이미 연결됨). 이 lane은 감시기 API를 바꾸지 않습니다.

## Alternatives

- **entry 재해시를 최초 materialize 때만**: cache 폴더는 사용자 데이터 폴더라 다른 프로세스·도구가 바꿀 수 있습니다. 열 때마다 확인하지
  않으면 변조된 bytes를 읽습니다. 비용은 Performance 절에 적었습니다.
- **cache entry에도 `probe_project`**: entry에는 `.git`이 없어 모두 일반 폴더로 거부됩니다. entry에 가짜 `.git`을 만들면 content address가
  바뀝니다.
- **`UnavailableReason`에 새 variant**: 진단 index 코드 표(`stellar-integrity`, 소유 밖)의 exhaustive match가 깨집니다. 의미가 같은 기존
  값으로 옮깁니다.
- **문서에 외부 참조 표를 따로 저장**: 새 파일 형식과 텍스트 원본 codec 변경이 필요합니다(document_io·document 소유). 추론 규칙으로 시작하고
  저장 형식이 생기면 바꿉니다.
- **update 전 검증을 resource diff(XV.7.2)로**: diff는 resource 변화만 알고 사용 프로젝트 override의 타입·공개 여부 판정은 사전 검사와 다른
  두 번째 판정이 됩니다.

## Consequences

- 공용 cache의 entry는 열 때마다 bytes·매니페스트·저장 데이터 검증을 통과해야 snapshot이 됩니다. 변조·일반 Git 저장소·손상 library는 그 node만
  unavailable이고 다른 commit을 고르지 않습니다.
- 사용 프로젝트에서 library 정의를 바꾸는 document command는 적용 전에 거부됩니다. 문서가 외부 참조를 저장하기 전까지 같은 StableId를 두 직접
  의존성이 export하면 편집을 거부합니다.
- update 후보를 적용하기 전에 끊기는 override·instance가 orphan 진단으로 나옵니다. lock·in-memory snapshot의 원자 갱신(XV.7.4)은 남았습니다.
- IX.9.1의 validator 0 오류는 document validator 수정 전까지 확인되지 않습니다.

## Compatibility

파일 포맷·lock 형식·generation manifest·C ABI·FFI 헤더를 바꾸지 않습니다. 보호 경로를 바꾸지 않습니다. 공개 API 변경은
`load_cached_snapshot`·`load_locked_libraries`에 검증기 인자가 더해진 것(workspace 안 호출자는 의존성 crate와 이 lane의 연결뿐),
`SnapshotLoadError::{Corrupted, Rejected}`, `ExportReadFailure::InvalidLibrary`, `StorageValidationFailure::TextSource`입니다.

## Performance

열 때 entry마다 모든 파일을 한 번 읽어 SHA-256을 계산하고 모든 chunk를 한 번 더 읽습니다. 비용은 entry 크기에 비례합니다(XV.3.3 상한 4 GiB,
일반 library는 수 MB). 열기 경로는 문서 열기와 독립이라 인터랙션 예산(§6)에 들지 않습니다. 큰 library에서 문제가 되면 entry별 검증 기록(read
lock 아래 mtime·inode 지문)을 두어 생략하는 최적화를 별도 ADR로 정합니다.

## Security

cache entry probe는 `stellar.yaml` symlink를 따르지 않고 loader 상한까지만 읽습니다. 텍스트 컴포넌트 원본은 외부 symlink 내용을 읽지 않습니다
(`ExternalAccess::MetadataOnly`). 진단·오류에는 repository URL·경로 원문을 넣지 않습니다(XV.1.4).

## Migration

없음. 기존 cache entry는 다음 열기에서 검증됩니다.
