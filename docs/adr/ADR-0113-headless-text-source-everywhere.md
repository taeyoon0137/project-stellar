# ADR-0113: CLI·MCP·semantic Git이 텍스트 원본을 원본으로 읽고 쓴다

## Status

Accepted (2026-11-24). engine lane `text-source-everywhere`의 결정입니다. ADR-0088(앱 기본 원본을 텍스트로, chunk를
로컬 cache로)의 결정 8 "이번에 연결하지 않은 것"의 CLI·MCP·semantic Git 부분과 새 프로젝트 첫 저장을 이 ADR이
이어받습니다. ADR-0081(semantic merge 해결과 Git driver)의 driver·conflict 세션 dispatch에 텍스트 원본 단위를 더합니다.

## Context

- 사용자 결정(2026-11-14): 의미 단위 텍스트 파일이 Git 원본이고 chunk generation은 로컬 cache입니다. 핵심 가치는 Git
  협업과 AI 친화(CLI·MCP로 앱과 같은 파일 편집)입니다.
- ADR-0088로 앱 창 session은 텍스트 원본을 기본으로 열고 저장합니다. 그러나 `stellar_ops`(CLI·MCP의 공용 library)의
  snapshot·apply·Merge Composer·Git 엔진, `stellar merge-driver`, conflict 세션은 generation만 읽고 썼습니다. 그래서
  - Git pull·편집기로 원본만 바뀌면 CLI·MCP는 옛 cache를 읽었고, 그 위에 쓴 편집은 원본에 들어가지 않았습니다.
  - 앱이 만든 프로젝트에서 두 branch를 병합하면 View 원본 파일이 conflict로 남고 conflict 세션이 그 경로를 "의미 병합하지
    않는 Stellar 경로"로 제외했습니다(XVI.11.3 실측).
  - 새 프로젝트의 최초 commit에는 generation만 있었습니다.
- 사용자 방향(2026-11-23): 엔진만 만들고 Swift UI·셸 FFI를 더하지 않습니다. 검증은 Rust 시험·CLI·MCP입니다.
- 같은 판정을 두 곳에 만들지 않습니다: 열기 계획·원본 변환·원본 저장은 `stellar_project::document_io::session` 하나를
  앱 창 session과 CLI·MCP가 함께 씁니다.

## Decision

### 1. CLI·MCP 열기는 앱 창과 같은 열기 계획을 쓴다

- `document_io::session::observe(root)`가 원본·current pointer·stamp를 읽어 ADR-0088 결정 1의 `OpenPlan`을 정합니다. 앱 창
  (`window_session::source::read_disk_state`)과 `stellar_ops::snapshot::open_snapshot`이 같은 판정을 씁니다.
- generation을 원본으로 옮기는 검증된 변환은 `document_io::session::convert_saved` 하나입니다. 앱 창 session의 `convert`는
  이 함수를 부르는 얇은 wrapper가 되었습니다.
- `open_snapshot`의 문서 읽기:

| 계획 | 문서를 읽는 곳 | revision |
|---|---|---|
| `Cache`·`Forward`·`Migrate` | cache generation(부분 로딩, 지금과 같음) | `g<번호>:<manifest hash>` |
| `FromText` | 텍스트 원본을 열 때 해석한 문서(`source_v4::project::decode`) | 결정 2 |
| `FromText`인데 원본이 깨짐 | 열기는 하되 문서 읽기를 원본 진단(파일·줄)으로 거부 | 결정 2(빈 원본으로 계산) |
| `Empty` | 없음 | `g0` |

- 읽기 경로는 여전히 파일을 쓰지 않습니다. cache가 원본보다 뒤처져도 읽기는 cache를 다시 만들지 않습니다(다음 쓰기가
  맞춤). `ProjectSnapshot::generation()`은 cache이고, FromText snapshot에서는 뒤처진 cache입니다.

### 2. revision은 원본과 cache를 함께 반영한다

- 원본에서 읽은 snapshot(FromText)의 revision은 `g<current 번호 + 1>:<원본 revision hash>`입니다. 원본 revision hash는
  `session::source_revision`: `stellar-text-source-revision/1`, current manifest hash(없으면 빈 값), 원본 파일 집합 digest를
  길이와 함께 이은 bytes의 hash입니다. 번호는 다음 cache generation 번호입니다.
- 그래서 Git pull 전에 받은 revision으로 `stellar apply`·MCP 쓰기를 하면 base revision 불일치로 거부되고, 원본을 다시 읽은
  revision으로만 씁니다. 원본과 cache가 짝(Cache)이면 revision은 지금과 같은 generation revision입니다.
- MCP headless의 snapshot 재사용 판정(`TrustedProject::snapshot_is_current`)은 current pointer만이 아니라 원본 파일 bytes
  hash까지 비교합니다(`ProjectSnapshot::disk_unchanged`, `observe` 재사용). 원본만 바뀌어도 snapshot을 다시 엽니다.

### 3. CLI·MCP 쓰기는 앱 창 저장과 같은 순서로 원본과 cache를 함께 쓴다

`stellar_ops::snapshot::save::save_documents`가 `stellar apply`·`stellar css paste`(apply 경유)·Merge Composer 결과 적용의
유일한 쓰기 경로입니다. 저장 잠금 안에서:

1. 원본 대상: snapshot이 원본을 봤으면(Cache·Forward·FromText) 열 때의 원본 base 위에 씁니다. 문서 밖 의미(컴포넌트·
   token·이미지 record)는 원본 것입니다(FromText는 열 때 해석한 것, 아니면 열 때 본 원본 bytes를 해석).
2. 결과 문서를 원본으로 인코딩합니다. 원본이 담을 수 없는 결과는 아무것도 쓰지 않고 거부합니다(원본과 cache를 가르지
   않음).
3. current pointer가 열 때와 같은지 확인합니다.
4. 바뀐 원본 파일만 저장 journal(ADR-0078) 한 거래로 씁니다(기대 hash = 열 때 base). 그사이 바뀐 원본 파일이 있으면
   `ConcurrentWrite`이고 아무것도 쓰지 않습니다.
5. cache generation을 세대 전환 저장으로 씁니다.
6. stamp(원본 digest, 새 manifest hash)를 씁니다.

원본을 쓴 뒤 cache·stamp 쓰기가 실패하면 저장은 성공이고 `cache_stale`이며, 돌려주는 revision은 그 디스크를 다시 열었을
때의 원본 revision입니다. 텍스트 원본이 없는 chunk 프로젝트(`Migrate`)와 root 문서가 둘 이상인 프로젝트는 generation만
씁니다(`SourceWrite::ChunkOnly`). CLI 쓰기가 기존 chunk 프로젝트를 몰래 원본으로 바꾸지 않습니다. 앱 없이 원본으로 옮기려면
명시적인 `stellar text-source adopt <프로젝트>`를 씁니다: 같은 `convert_saved` 검증을 통과할 때만 원본과 stamp를 쓰고
generation은 그대로라 revision도 그대로입니다.

### 4. 새 프로젝트의 첫 저장은 원본·cache·stamp를 함께 쓴다

`create_editable_project`의 첫 저장(`create/initial_document`)은 앱 창 저장과 같은 원본 → cache → stamp 순서로 씁니다.
최초 commit은 텍스트 원본 파일을 담고, `.stellar/` 아래 로컬 파일(stamp·journal)은 그 폴더의 `.gitignore`로 Git 밖에
둡니다. 앱이 처음 열 때의 `Migrate`는 더 이상 필요 없습니다(바로 `Cache`).

### 5. 텍스트 원본을 merge driver와 conflict 세션이 의미 병합한다

- `stellar_ops::merge_text`: 텍스트 원본 파일 하나는 프로젝트 전체 의미의 일부라 세 쪽의 **원본 파일 집합 전체**를 같은
  codec으로 읽고, 문서는 레코드 snapshot을 엔진으로 병합(XIV.10, 무손실 왕복 조건은 `merge_apply::document`와 같음)하고,
  문서 밖 의미(이미지 record·token set은 id별, Component Context는 통째)는 3-way로 한쪽만 바뀐 값을 받습니다. 두 쪽이
  다르게 바꾸면 병합하지 않습니다. 컴포넌트 정의 파일은 지금처럼 파일마다 컴포넌트 원본 병합(XXIV.2.1)이 다루고, 텍스트
  원본 병합의 쓰기 결과에서 뺍니다. 결과는 원본으로 인코딩해 다시 읽은 의미가 같을 때만 받습니다.
- driver(`stellar merge-driver %O %A %B %P`): 경로가 텍스트 원본(`design/{document,pages/**,assets/catalog,context,
  tokens/*}.stellar.yaml`)이면 merge 중 작업 트리(= ours)의 나머지 원본 파일을 세 쪽 공통 문맥으로 씁니다. 결과가 그
  경로 밖의 원본도 바꿔야 하면 driver로 표현할 수 없어 nonzero로 끝납니다(Git이 conflict로 남기고 conflict 세션이 세 stage
  전체로 다시 병합). `merge-driver install`은 이 다섯 pattern 줄을 컴포넌트 줄 앞에 더합니다.
- conflict 세션: unmerged 텍스트 원본 경로 전체가 한 단위(`UnitKind::TextSource`)입니다. 세 쪽 파일 집합은 작업 트리의
  나머지 원본(unmerged 경로 제외)에 unmerged 텍스트 원본·컴포넌트 원본의 stage blob을 더해 만듭니다. 텍스트 원본이 있는
  프로젝트에서 `design/current`(cache pointer)가 conflict이면 cache는 따로 병합하지 않고 이 단위의 결과로 다시 씁니다
  (ours 세대 위의 새 세대, `git add`). 원본은 깨끗이 병합되고 cache pointer만 conflict여도 같은 단위가 cache를 다시 씁니다.
  해결은 기존 `Resolution`·`apply_session`(journal 한 거래, `git add`, continue·commit 없음)을 그대로 씁니다.

### 6. semantic Git은 원본을 Git 원본으로 본다

- `stellar git status`의 semantic 변경은 HEAD·index·작업 트리 각각에서 텍스트 원본이 있으면 원본을, 없으면 generation을
  읽습니다(`snapshot::source_semantic`). Merge Composer의 두 source도 같습니다.
- 변경 ID 단위 stage·unstage는 index에 텍스트 원본이 있으면 후보 문서를 원본으로도 인코딩해 바뀐 원본 파일을 index
  후보에 넣고, 후보 tree의 원본 의미가 기대 의미와 같은지 다시 확인합니다. cache generation만 stage해 commit의 원본과
  cache가 갈라지는 일을 막습니다.
- staged 재검증(`validate_staged`)은 staged 원본이 읽히는지 확인합니다(읽히지 않으면 오류). staged 원본과 staged cache의
  의미가 다르면 경고입니다(원본이 이기므로 commit은 막지 않음).

### 7. 이번에 하지 않은 것

- **chunk의 물리 위치와 cache GC**: generation은 아직 `design/current`·`design/generations`·`design/objects`에 있고 Git이
  추적합니다. Git 밖(`.stellar/cache/`)으로 옮기려면 storage 경로(보호 경로), git-porcelain의 snapshot·분류, 앱 창 session,
  감시, 시험 fixture 전체가 바뀌고, 위 결정 6의 stage 경로도 cache 없이 원본만으로 후보를 만들어야 합니다. 이 lane 소유
  밖이 대부분이라 하지 않았습니다. 추적되는 옛 generation을 지우는 GC는 Git 추적 파일 삭제라 연결하지 않았습니다. 그
  대신 결정 2·5·6으로 cache가 commit에 들어가도 원본이 기준이 되게 했습니다(cache pointer conflict는 결정 5가 원본 결과로
  다시 씀).
- **driver의 텍스트 줄 merge fallback**: driver가 의미 병합하지 못한 텍스트 원본 파일은 Git 텍스트 merge 결과 대신
  conflict로 남습니다. conflict 세션이 세 stage로 다시 병합합니다.
- **원본 단위(파일) stage의 cache 동반**: 원본 파일만 stage하면 staged cache와 의미가 다를 수 있습니다(결정 6 경고).

## Alternatives

- **읽기에서 cache를 다시 만듦(FromText면 open_snapshot이 generation을 씀)**: 모든 generation 소비자가 그대로 맞지만, cache가
  Git 추적 경로라 `stellar query` 같은 읽기가 작업 트리를 바꿉니다. 읽기 전용 계약(ADR-0005 §1, `ops` 모듈 문서)과 어긋나
  탈락.
- **revision을 generation만으로 유지**: Git pull 뒤에도 같은 revision이라 pull 전 조회로 만든 편집이 pull한 원본 위에 조용히
  적용됩니다. 탈락.
- **CLI 쓰기가 chunk 프로젝트를 자동으로 원본으로 전환**: 사용자 동의 없이 `design/`에 새 파일을 만들고 기존 merge 시나리오
  (generation 단위)를 바꿉니다. 명시적 `text-source adopt`로 둡니다.
- **driver가 그 파일만 해석**: View 파일은 문서 파일·catalog 없이 해석할 수 없습니다(View 목록·Text style 표). 줄 단위 YAML
  병합은 의미 엔진과 다른 두 번째 병합기가 됩니다. 탈락.
- **conflict 세션에서 cache(`design/current`)도 generation 단위로 따로 병합**: 같은 문서를 두 번 병합해 두 결과가 다를 수
  있습니다. 원본 결과로 cache를 다시 씁니다.

## Consequences

- CLI·MCP·Git이 앱과 같은 파일(텍스트 원본)을 원본으로 읽고 씁니다. Git pull·편집기 변경을 CLI·MCP가 바로 봅니다.
- `stellar apply`가 바꾼 이름 하나는 View 원본 파일의 한 줄 diff입니다. 같은 View 파일의 이웃한 줄을 두 branch가 바꿔도
  driver가 의미 단위로 병합합니다.
- 앱이 만든 프로젝트의 두 branch conflict가 `merge-driver resolve --apply`로 unmerged 0이 됩니다(XVI.11.3).
- CLI 쓰기가 원본도 쓰므로, 열린 앱 창은 원본 파일 event와 cache event를 따로 받을 수 있습니다. 창 session은 원본 event로
  FromText 상태를 받은 뒤 cache가 그 원본에 맞춰 써지면 그 cache를 병합 base로 씁니다(`reconcile_external`의 따라잡은 cache
  판정).
- 기존 시험 기대값 가운데 "CLI는 generation만 씀"을 전제한 것은 cache-only writer helper(`test_fixtures::cache_only_rename`)로
  바꾸었고, "최초 commit에 원본이 없음"·"View 원본 conflict는 제외"를 전제한 E2E 기대값을 이 ADR에 맞게 고쳤습니다.

## Compatibility

- 파일 포맷·generation manifest·journal·stamp 형식·C ABI·FFI 헤더를 바꾸지 않습니다. 보호 경로를 바꾸지 않습니다.
- revision 표기 형식은 같습니다(`g<번호>:<hash>`). FromText snapshot의 hash는 manifest hash가 아니라 원본 revision hash입니다.
  revision을 current pointer와 직접 비교하던 MCP 재사용 판정은 `ProjectSnapshot::disk_unchanged`로 바꾸었습니다.
- 공개 Rust API 추가: `document_io::session::{observe, DiskSource, ObserveError, source_revision, convert_saved}`,
  `ProjectSnapshot::{open_plan, current_manifest, source_files, reads_from_text, source_diagnostic, decoded_library,
  disk_unchanged}`, `snapshot::source_semantic`, `snapshot::save::{save_documents, root_order, SaveOutcome, SaveError,
  SourceWrite, adopt, adopt_text_source, AdoptOutcome, AdoptError}`, `ApplyOutcome::{source, cache_stale}`,
  `merge_text::*`, `candidate::session::UnitKind::TextSource`, `candidate::install::attributes_lines`.
- CLI: `stellar text-source adopt <프로젝트>`(결과 `operation`·`revision`·`file_count`·`written`, 이미 원본이 있으면 종료 코드 1
  `text-source-present`). `merge-driver resolve`의 단위 `kind`에 `text-source`가 생깁니다. `merge-driver install`이
  `.gitattributes`에 텍스트 원본 pattern 다섯 줄을 더합니다.

## Performance

- 원본에서 읽는 snapshot은 원본 전체를 해석합니다(ADR-0088 Performance의 decode: 5만 node 약 1.2초). cache와 짝이면 지금과
  같은 부분 로딩입니다.
- CLI·MCP 쓰기는 원본 인코딩과 journal fsync만큼 늘어납니다(ADR-0088: 5만 node encode 약 5.8초). Cache·Forward snapshot의
  쓰기는 문서 밖 의미를 얻으려고 원본을 한 번 해석합니다.
- MCP 재사용 판정은 요청마다 원본 파일 bytes를 읽고 hash합니다. 대형 원본에서는 metadata 지문으로 먼저 거르는 개선이
  필요합니다(ADR-0088 남은 위험과 같음).
- 이번 변경에서 성능 예산 값을 바꾸지 않았고 기준 장비 실측은 하지 않았습니다.

## Security

- 원본 저장은 journal 경로 규칙(`..`·절대 경로·`.git`·`.stellar/` 거부)과 symlink 거부를 따릅니다. 원본 읽기는 `design`
  폴더 symlink와 그 안의 symlink를 거부합니다.
- conflict 세션의 stage blob은 비신뢰 입력이라 같은 codec 검증을 통과해야 하고, UTF-8이 아니면 병합하지 않습니다. driver는
  Git을 실행하지 않고 작업 트리 원본만 읽습니다.

## Migration

- 텍스트 원본이 있는 프로젝트: 바꿀 것이 없습니다. 다음 CLI·MCP 쓰기가 원본과 cache를 함께 씁니다.
- 텍스트 원본이 없는 chunk 프로젝트: 지금처럼 generation만 씁니다. 앱에서 처음 열거나 `stellar text-source adopt`로 옮깁니다.
- merge driver를 이미 등록한 저장소: `stellar merge-driver install <프로젝트>`를 다시 실행하면 텍스트 원본 pattern 줄만 더합니다.
