# ADR-0131: 옛 chunk 프로젝트를 텍스트 원본으로 바꾸고, 원본이 컴포넌트 interface·계산 token·추가 root 문서를 담는다

## Status

Accepted (2026-11-24). engine lane `legacy-convert`의 결정입니다. ADR-0122 개정(2026-11-24, 원본 없는 옛 배치 보호)의
후속이고, ADR-0077 결정 3("Property·Slot·상태 머신과 리터럴이 아닌 token은 표현식 문법이 정해지기 전이라 쓰지 않음")과
ADR-0122 결정 3("옛 pointer는 새 pointer를 바꿈")의 작업 트리 부분을 바꿉니다. 보호 경로 `crates/storage`의 옛 배치 이동
함수(`stellar_storage::location`)에 선택지를 더합니다.

## Context

- ADR-0122 개정으로 작업 트리에 텍스트 원본 없이 옛 배치(`design/current`·`design/generations/`·`design/objects/`)만 있으면
  cache 이동과 열기를 거부합니다(`CacheError::LegacyWithoutSource`). 옛 배치가 그 프로젝트의 유일한 Git 원본이라 옮기면 Git
  원본이 사라질 수 있기 때문입니다. 이런 프로젝트를 쓰려면 원본으로 바꾸는 경로가 필요합니다.
- 원본으로 바꿀 수 없던 이유는 둘이었습니다. (1) 텍스트 원본(ADR-0077)이 component library의 Property·Slot·상태 머신을
  쓰지 못했습니다(`ComponentSource::from_entry`가 거부). (2) 원본은 root 문서 하나만 담았습니다(`from_saved`가 거부). 같은
  이유로 리터럴이 아닌 기본 표현식의 token도 원본에 쓸 수 없었습니다(V.10.4 남은 범위).
- 옛 배치 이동은 저장 잠금 밖의 열기 관찰(`document_io::session::observe`)에서 일어나고, 옛 pointer가 cache pointer를 바꾼
  뒤 stamp를 지웠습니다. Git이 `design/current`를 되살리면(`git restore`, 옛 commit checkout) 원본 이후 cache에만 쓴
  편집(Forward)이 current에서 빠졌습니다(ADR-0122 통합 검토의 남은 위험).
- 저장 잠금(`stellar_ops::lock::SaveLock`)은 ops crate에 있고, 관찰은 그 아래 crate(project)에 있습니다(crate graph).
- 사용자 방향(2026-11-23): 엔진만 만들고 Swift UI·셸 FFI는 더하지 않습니다. 검증은 Rust 시험·CLI·MCP입니다.

## Decision

### 1. 컴포넌트 Property·Slot·상태 머신은 interface 원본 `design/interfaces/<이름>-<hash12>.stellar.yaml`에 쓴다

- 컴포넌트 정의 파일(`design/components/`)은 그대로 이름·root·local token·정의 tree를 담고, 같은 컴포넌트의 공개 interface는
  같은 파일 이름의 interface 파일 하나에 씁니다(`source_v4::interface`, 형식 `stellar-component-interface`, schema 4).
  interface가 비었으면 쓰지 않습니다. 정의 파일 형식은 바뀌지 않습니다(정의 파일만 읽는 기존 경로·library 의존성 cache
  entry가 그대로 동작).
- 담는 것: Property(정의의 선언 순서, id·이름·`public|private`·값 타입·기본 식), Slot(선언 순서, id·host·허용 node 종류·
  `min`·`max`(`none` 또는 정수)·default content), semantic action(id 순), 상태 변수(id 순, 값 타입·초기 식), transition
  (우선순위 순, trigger `event <종류> <node>` 또는 `action <nested instance> <action>`, 조건 식(없으면 항상 참), action
  `set`(변수와 `to` 식)·`emit`).
- 식은 Component Context 원본(ADR-0099)과 같은 문법입니다: 리터럴 하나면 `<접두>_expression`(root id)·`<접두>_type`·
  `<접두>`, 그 밖이면 `<접두>_ast`(canonical AST bytes hex). 새 표현식 텍스트 문법을 따로 만들지 않습니다.
- 읽기는 컴포넌트 정의를 모두 등록한 뒤 interface를 적용하고, library 저장 규칙(`ComponentLibrary::decode`의 선언 record·
  slot 투영·override·상태 머신 검증)을 그대로 다시 통과해야 합니다. 같은 검증을 두 곳에 두지 않으려고 canonical bytes를
  다시 decode합니다. interface 파일 이름이 정의 파일과 다르거나, 같은 컴포넌트의 interface가 둘이거나, 빈 interface면
  거부합니다. 쓰기는 다시 읽은 library bytes가 같을 때만 합니다(다중 파일 원본의 의미 비교).

### 2. 리터럴 하나로 쓸 수 없는 기본 식의 token은 계산 token으로 쓴다

- `TokenSetSource`에 `computed: Vec<ComputedTokenSource>`(id·이름·선언 값 타입·기본 식 AST)를 더합니다. 파일에는 같은
  `tokens` 목록에 id 순으로 섞어 `value_type`과 `ast`로 씁니다. 리터럴 하나로 쓸 수 있는 식은 리터럴 token으로만 씁니다
  (같은 의미의 표기는 하나). id·이름은 두 종류를 합쳐 유일합니다. 표현식 type 검사와 bindings `TokenSet` 변환은 참조 타입을
  아는 bindings 계층의 몫입니다(project는 bindings에 의존할 수 없음, ADR-0077).
- 리터럴 token 목록(`tokens`)의 뜻은 그대로라 리터럴만 읽는 기존 호출자(library export 서명 등)는 계산 token을 리터럴로
  잘못 읽지 않습니다. 모든 token id는 `TokenSetSource::token_ids`입니다.

### 3. 둘째 root 문서부터는 `design/documents/<slug>/`에 같은 배치로 쓰고, 옛 배치를 그 원본으로 바꾼다

- 추가 root 문서 목록 `design/documents.stellar.yaml`(형식 `stellar-documents`)이 generation root 순서대로
  `documents/<slug>/document.stellar.yaml`을 가리킵니다. 문서 폴더 하나는 첫 문서와 같은 codec(`source_v4::project`)으로
  쓰고 읽으며 그 문서의 View·Frame 파일·Text style catalog·Context를 담습니다. 컴포넌트·token·이미지 record는 프로젝트 전체
  의미라 첫 문서 쪽에만 씁니다(`source_v4::documents`).
- 첫 문서만 다루는 경로(앱 창 session·CLI·MCP·semantic Git)는 추가 문서를 바꾸지 않습니다. 첫 문서 codec은 이 경로들을
  문서 의미 밖으로 건너뛰고, 원본 저장 계획(`session::plan_changes`)은 인코딩 결과에 없는 추가 문서 파일을 지우지 않습니다.
  읽기 검증은 `session::decode_source`가 함께 합니다. cache 다시 만들기(`cache::rebuild_from_text`)는 모든 root 문서를 한
  generation에 담습니다(root 순서 유지).
- generation 변환은 `source_v4::project::from_saved_documents` 하나입니다. root 문서 전부·component library chunk·asset
  metadata chunk를 소비해야 하고(남는 chunk가 있으면 거부), 원본의 컴포넌트 정의와 generation library chunk가 둘 다 있으면
  같아야 합니다. `from_saved`는 문서 하나의 형태이고, 앱 창·CLI·MCP의 Forward·Migrate 변환(`session::convert_saved`)도 이
  함수를 씁니다. Forward에서 다른 root 문서는 원본의 추가 문서와 같아야 하고, 원본이 없는 Migrate는 root 문서 하나만 받습니다.
- `cache::convert_legacy`: 작업 트리에 원본 없이 옛 배치만 있으면 옛 파일을 읽기 전용 비공개 사본(`cache::legacy_view`와
  같은 복사 함수)으로 옮겨 연 뒤, generation 전체를 원본 의미로 옮기고, 인코딩한 파일 집합을 열기와 같은 함수로 다시 읽어
  의미가 같을 때만 저장 journal 한 거래로 씁니다(기대 hash = 기존 컴포넌트 원본, 다른 writer가 먼저 쓰면 거부). 그다음 옛 배치를
  cache에 **복사**하고(옛 파일은 작업 트리에 둠) stamp(원본 digest·옛 manifest)와 변환 표시(`.stellar/cache/legacy-converted`,
  옛 pointer bytes hash)를 씁니다. 바꿀 수 없으면(모르는 chunk, schema가 제품 schema가 아님, 의미 차이) 원본 쓰기 전에 멈추고
  `CacheError::LegacyUnconvertible`입니다.
- 앱 창 열기·CLI·MCP 열기(`observe`)와 semantic Git(`GitSession::migrate_cache`)이 `cache::migrate_working_tree`(=
  `LegacyLayout::Convert`)로 이 변환을 부릅니다. `stellar cache migrate --convert <프로젝트>`도 같은 함수입니다
  (`stellar_ops::git_cache::migrate_with`). `stellar cache status`와 `--convert` 없는 `migrate`는 원본을 쓰지 않으므로 변환하지
  않고 지금처럼 거부합니다(`LegacyLayout::Refuse`, 메시지가 `--convert`를 알림).

### 4. 옛 배치는 바꾼 원본이 Git index에 들어갈 때 옮기고, 원본이 있는 작업 트리에서 옛 pointer는 cache pointer를 바꾸지 않는다

- 변환 표시가 있으면(바꾼 원본이 아직 Git에 없음) 관찰은 옛 배치를 그대로 둡니다. 그래서 그사이 `git commit -a`를 해도
  추적하지 않는 새 원본 대신 옛 배치가 HEAD에 남아 Git 원본이 끊기지 않습니다. 작업 트리의 옛 pointer가 변환 때와 다르면
  (다른 commit checkout) 바꾼 원본과 옛 배치가 다른 상태이므로 옮기지 않고 `CacheError::LegacyChanged`입니다(열기는 원본으로
  계속).
- `cache::untrack_legacy`(Git을 아는 호출자: semantic Git stage·unstage·commit, `stellar cache migrate`)는 변환 표시가 있으면
  같은 index 갱신에서 원본 파일을 먼저 `git add`(literal pathspec)하고 옛 경로를 `git rm --cached`한 뒤, 표시를 지우고 작업
  트리의 옛 배치를 cache로 옮깁니다. 다음 commit 하나가 옛 배치 삭제와 새 원본을 함께 담습니다.
- 원본이 있는 작업 트리에서 옛 배치를 옮길 때 옛 pointer는 cache pointer로 쓰지 않고 지웁니다
  (`stellar_storage::location::LegacyPointer::Drop`). 원본이 Git 원본이라 옛 pointer는 새 정보가 아니고(원본과 짝인지 모르므로
  ADR-0122도 stamp를 지워 원본에서 읽었음), cache pointer를 바꾸면 Forward 편집이 빠지기 때문입니다. 그 세대의 manifest·chunk는
  다른 파일처럼 cache로 옮기고 GC가 정리합니다. Git snapshot을 푼 비공개 폴더(`cache::migrate_layout`)는 지금처럼 옛 pointer가
  cache pointer를 바꿉니다(`LegacyPointer::Replace`).
- 저장 잠금: 이 규칙은 cache pointer 상태와 관계없이 같은 결과이고 내용 주소 파일 이동은 경쟁해도 안전하므로, 잠금 없이 도는
  관찰에서도 다른 writer의 pointer 교체와 경쟁하지 않습니다. 잠금은 ops crate에 있어 project crate의 관찰이 잡을 수 없으므로,
  "관찰의 이동을 잠금 안으로" 옮기는 대신 이동이 cache pointer를 쓰지 않게 했습니다. 원본을 쓰는 변환은 저장 journal 잠금과
  기대 hash가 직렬화하고, `stellar cache migrate`는 지금처럼 저장 잠금 안에서 돕니다.
- `stellar_storage::location::transfer_legacy_layout(root, LegacyTransfer::{Move, Copy}, LegacyPointer::{Replace, Drop})`가
  일반형이고 `move_legacy_layout`은 `(Move, Replace)`입니다. `LegacyMove::current_dropped`를 더합니다.

### 5. 공개 포맷 동결 전 검증

- `source_v4::freeze_tests`: 모든 파일 종류(문서·View·Frame·catalog·컴포넌트 정의·interface·리터럴·계산 token·추가 root
  문서)를 담은 프로젝트로 (1) 새 파일 종류의 golden bytes(`source_v4/golden/{interface,tokens,documents}-v4.stellar.yaml`,
  `STELLAR_TEXT_SOURCE_BLESS=1`로 다시 씀), (2) 같은 프로젝트 = 같은 파일 집합·bytes, 다시 읽어 다시 쓰면 같은 bytes, (3) 어느
  파일의 어느 한 줄을 지워도 거부·의미 변경이거나, 생략값을 기본값으로 읽는 줄(ADR-0082·ADR-0086)이면 다시 쓴 결과가 원래
  canonical bytes임을 확인합니다(의미 하나 = 정규형 하나).

## Alternatives

- **Property·Slot·상태 머신을 정의 파일(`components/`) 안에 씀**: 한 파일에 모이지만 정의 파일 형식(ADR-0077 결정 2, 정의 파일만
  읽는 library 의존성·component-source 명령)이 바뀌고, 정의 tree만 편집하는 CLI 경로가 interface 줄을 다시 써야 합니다. 같은
  이름의 별도 파일로 두면 기존 경로가 그대로 동작합니다. 탈락.
- **interface·token 식에 새 텍스트 표현식 문법(`a + b` 등)**: 사람이 읽기 좋지만 parser·formatter를 하나 더 만들어 expressions
  crate의 canonical AST와 두 경로가 됩니다. Context 원본이 이미 쓰는 "리터럴 또는 canonical AST hex" 문법을 그대로 씁니다. 텍스트
  식 문법은 expressions crate가 정하면 그 문법으로 바꿉니다(그때 schema를 올림). 탈락.
- **`TextProject`에 추가 root 문서 필드를 더함**: 첫 문서만 다루는 모든 호출자(앱 창·CLI·MCP·semantic merge)와 시험의 생성
  지점이 바뀌고, 추가 문서를 모르는 경로가 저장하면 지울 위험이 있습니다. 별도 파일 묶음과 "지우지 않음" 규칙으로 두었습니다.
  탈락.
- **관찰이 저장 잠금을 잡음**: 잠금 primitive가 ops crate에 있어 project crate로 옮기거나 같은 key를 따로 계산해야 합니다(이중
  경로). 잠금을 잡아도 "옛 pointer가 cache pointer를 바꿈" 규칙이 남으면 Forward 편집은 여전히 빠집니다. pointer를 쓰지 않는
  규칙으로 경쟁 자체를 없앴습니다. 탈락.
- **변환하자마자 옛 배치를 작업 트리에서 옮김**: 새 원본이 untracked라 `git commit -a`가 옛 배치 삭제만 담아 Git 원본이 끊깁니다
  (ADR-0122 개정이 막은 위험). 탈락.
- **변환할 때 Git index도 고침(관찰 안에서 `git add`)**: 관찰은 신뢰·Git 실행을 모르는 경로이고(비신뢰면 프로세스 0) 읽기
  명령도 씁니다. index는 Git을 아는 호출자(`untrack_legacy`)만 고칩니다. 탈락.

## Consequences

- 원본 없는 옛 chunk 프로젝트(root 문서 둘 이상, Property·Slot·상태 머신을 담은 component library, 이미지 record)를 열면 텍스트 원본으로 바뀌고, 다음 semantic Git stage·commit이나 `stellar cache
  migrate --convert`가 원본을 index에 넣고 옛 배치를 뺍니다. 그 commit 뒤로는 ADR-0122 프로젝트와 같습니다.
- Property·Slot·상태 머신을 가진 library도 텍스트 원본으로 저장·배포됩니다(ADR-0122 결정 6의 후속). 옛 배치 library는 지금처럼
  읽기 전용 사본에서 읽습니다.
- Git이 `design/current`를 되살려도 Forward 편집이 cache current에 남습니다.
- 바꿀 수 없는 옛 배치(제품 schema가 아닌 generation, 텍스트 원본이 소비하지 않는 chunk)는 지금처럼 거부하고 작업 트리를
  바꾸지 않습니다.
- 첫 문서만 다루는 앱 창 session은 추가 root 문서를 편집하지 않습니다. 추가 문서 편집 경로(창 하나에 문서 고르기)는 UI 재설계
  몫입니다. 앱 창의 FromText 맞춤 쓰기(`runtime::window_session`)가 cache가 없는 상태에서 쓰면 그 generation에는 첫 문서만
  들어갑니다(원본에는 남고, `stellar cache rebuild`·다음 원본 변경이 모든 문서로 다시 만듦).
- semantic Git의 원본 단위 분류(`stellar_ops::git::SourceUnit`)는 `interfaces/`·`documents/` 경로를 아직 모르므로 문서 전체
  generation 묶음으로 stage합니다. merge driver 대상(`.gitattributes` pattern)에도 없어 Git 줄 병합입니다. 파일 단위 stage·의미
  병합은 후속입니다.
- 변환 뒤 Git에 넣기 전에 `.stellar/cache/`를 지우면 변환 표시도 지워져, 다음 열기가 옛 배치를 옮깁니다(ADR-0122 동작). 그때는
  `git add design`이 필요합니다.

## Compatibility

- 새 공개 파일 형식: `stellar-component-interface`(schema 4, `design/interfaces/`), `stellar-documents`(schema 4,
  `design/documents.stellar.yaml`·`design/documents/`), token set 파일의 계산 token 항목(`value_type`·`ast`). 기존 파일의
  bytes는 같습니다. 이 형식을 모르는 이전 도구는 모르는 파일로 거부합니다(손실 저장 없음).
- 공개 Rust API: `source_v4::{interface, documents}`, `source_v4::project::{from_saved_documents, definition_records}`
  (`definition_records`는 crate 안), `token::{ComputedTokenSource, TokenSetSource::computed, TokenSetSource::token_ids}`,
  `session::decode_extra_documents`, `cache::{migrate_working_tree_with, LegacyLayout, has_legacy_layout, convert_legacy,
  LegacyConversion, converted_marker, clear_converted_marker, legacy_pointer_digest}`, `CacheError::{LegacyUnconvertible,
  LegacyChanged}`, `LayoutMigration::{converted, held}`, `stellar_ops::git_cache::{migrate_with, LegacyConversion}`.
  `TokenSetSource` 구조체 literal에 `computed` 필드가 늘었습니다(시험 fixture 다섯 곳에 빈 목록을 더함).
- 보호 경로 `crates/storage`: `location::{transfer_legacy_layout, LegacyPointer, LegacyTransfer}`와 `LegacyMove::current_dropped`
  추가. 기존 `move_legacy_layout`의 동작은 같습니다. 저장 bytes·journal·C ABI·FFI 헤더·schemas·성능 예산 값은 바꾸지 않습니다.
- CLI: `stellar cache migrate --convert <프로젝트>`. 출력 JSON에 `converted`(manifest·documents·components·written 또는
  null)를 더합니다.
- 동작 변경: 원본이 있는 작업 트리에서 옛 pointer는 cache pointer를 바꾸지 않습니다(ADR-0122 결정 3 개정). 그 시험 기대값
  (`tests/integration/tests/git_cache_out.rs`)을 고쳤습니다. 선언만 하고 record가 없는 Property의 쓰기 거부는 이제
  `Unsupported`가 아니라 `Invalid`입니다(`crates/project/tests/text_project.rs` 기대값을 고침).

## Performance

- 관찰은 옛 배치가 있을 때만 원본 읽기·변환 표시 확인이 늘고, 변환은 프로젝트마다 한 번입니다(generation 전체 decode·인코딩·
  다시 읽기, ADR-0088 Performance의 비용과 같은 크기). 추가 root 문서가 없으면 읽기 비용은 목록 파일 확인 하나입니다.
- interface가 있으면 읽기가 library canonical bytes를 한 번 더 decode합니다(검증 공유).
- 성능 예산 값은 바꾸지 않았고 기준 장비 실측은 하지 않았습니다.

## Security

- 변환은 옛 파일을 비공개 임시 폴더로 복사해 읽고(symlink를 따르지 않음) 작업 트리의 옛 배치는 바꾸지 않습니다. 원본 쓰기는
  저장 journal(기대 hash)입니다.
- Git index 정리는 신뢰한 프로젝트의 `GatedGitRunner`로만 하고, `git add`는 원본 파일 경로를 literal pathspec으로 넘깁니다.
- 변환 표시는 Git 밖 `.stellar/cache/`(`.gitignore` `*`)에 둡니다.

## Migration

- 원본 없는 옛 chunk 프로젝트: 앱·CLI·MCP로 열거나 `stellar cache migrate --convert <프로젝트>`를 실행합니다. 그 뒤 semantic Git
  stage·commit 또는 `stellar cache migrate`가 원본을 index에 넣고 옛 배치를 뺍니다. commit하면 전환이 끝납니다.
- 되돌리기: 그 commit을 되돌리면 옛 배치가 돌아옵니다. 변환만 하고 commit하지 않았다면 `design/` 아래 새 원본 파일과
  `.stellar/cache/`를 지우면 변환 전 상태입니다(옛 배치는 그대로 있음).
