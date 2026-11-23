# ADR-0122: chunk cache를 Git 밖 `.stellar/cache/chunks/`로 옮긴다

## Status

Accepted (2026-11-24). engine lane `git-cache-out`의 결정입니다. ADR-0059 결정 3("chunk 엔진은 Git에서 제외한 로컬
캐시, 위치 `.stellar/cache/`")을 구현하고, ADR-0088 결정 8과 ADR-0113 결정 7의 "chunk의 물리 위치와 cache GC"를
이어받습니다. 보호 경로 `crates/storage`의 저장소 경로 함수와 journal 대상 경로 규칙을 바꿉니다.

## Context

- 사용자 결정(2026-11-14): 의미 단위 텍스트 파일이 Git 원본이고 chunk는 로컬 cache입니다. ADR-0088·ADR-0113으로 앱
  창·CLI·MCP·semantic Git이 모두 텍스트 원본을 원본으로 읽고 쓰지만, generation 파일은 여전히
  `design/current`·`design/generations/`·`design/objects/`에 있어 Git이 추적했습니다.
- 그래서 두 branch가 모두 저장하면 `design/current`가 항상 conflict였고(ADR-0059 Context), 추적되는 옛 generation을
  지우면 Git 추적 파일 삭제라 cache GC를 연결할 수 없었습니다.
- 저장소의 경로는 storage crate 안의 고정 상수 몇 개(`current.rs`·`loader.rs`·`chunk/path.rs`·scanner·GC sweep·
  chunk store)에서 나오고, project loader·watcher·ops는 그 함수를 씁니다. 다만 ops·project·runtime·시험 곳곳에
  `design/current` 같은 literal이 남아 있었습니다.
- 사용자 방향(2026-11-23): 엔진만 만들고 Swift UI·셸 FFI는 더하지 않습니다. 검증은 Rust 시험·CLI·MCP입니다.

## Decision

### 1. cache 위치는 `.stellar/cache/chunks/` 하나이고, 위치 함수는 `stellar_storage::location` 하나다

```text
<프로젝트>/.stellar/cache/
├── .gitignore            # `*` (처음 cache·stamp를 쓸 때 만듦, 이미 있으면 그대로)
├── text-source.stamp     # 원본·cache 짝(ADR-0088)
└── chunks/
    ├── current
    ├── generations/<algo>-<hex>.starmanifest
    └── objects/<prefix>/<algo>-<hex>.starchunk
```

- `stellar_storage::location`이 cache 폴더(`cache_dir`, `CACHE_DIR`, `CACHE_RELATIVE`), 경로 판정(`is_cache_path`,
  `is_legacy_cache_path`, `is_local_cache_path`), 옛 배치 대응(`LEGACY_PATHS`, `legacy_relative_path`), Git 제외 규칙
  (`ensure_cache_ignore`)을 둡니다. current pointer·manifest·chunk object·scanner·GC sweep·chunk store가 모두 이 모듈의
  상수로 경로를 만들고, 그래서 project loader·watcher(`current_pointer_watch_path`)·앱 창 session·CLI·MCP·merge
  driver·semantic Git이 같은 위치를 씁니다. 다른 곳에서 경로를 따로 조립하지 않습니다(ops의 `design/current` literal을
  `current_pointer_path`·위치 판정으로 바꿈).
- 프로젝트 밖 사용자 cache(`~/Library/Caches`)는 고르지 않았습니다(Alternatives). `.stellar/cache/`는 ADR-0059가 정한
  위치이고 stamp가 이미 그곳에 있어, 원본·cache·stamp가 한 프로젝트 폴더 안에서 같은 수명을 가집니다.
- cache·manifest·pointer를 처음 쓸 때 `.stellar/cache/.gitignore`(`*`)를 만듭니다(stamp writer와 같은 함수). 그래서
  `git add -A`도 cache를 담지 않습니다.

### 2. journal은 cache 안 파일만 `.stellar/` 아래 대상으로 받는다

저장 journal(ADR-0078)은 `.stellar/`를 대상에서 거부했습니다. 병합 결과처럼 원본과 cache를 한 거래로 쓰는 경로가 있으므로
`.stellar/cache/chunks/` 아래 파일만 대상으로 받고, journal 자신(`.stellar/journal/`)·stamp 등 나머지 로컬 상태는 그대로
거부합니다(`journal::path::components`).

### 3. 옛 배치는 열기 관찰에서 한 번 옮기고, Git index 정리는 Git을 아는 호출자가 한다

- `stellar_storage::location::move_legacy_layout`: 작업 트리의 `design/current`·`design/generations/`·`design/objects/`를
  cache 위치로 옮깁니다. 내용 주소 파일은 새 위치에 없으면 rename, 같은 bytes가 있으면 옛 파일을 지우고, 다르면 둘 다
  남깁니다. 옛 pointer는 새 pointer를 바꿉니다(작업 트리에 옛 배치가 있다는 것은 아직 옮기지 않았거나 Git이 옛 branch를
  checkout해 다시 놓은 것이라 그 Git 상태가 새 정보). symlink는 따라가지 않습니다. 저장 순서(chunk → manifest →
  pointer)로 옮겨 중간에 멈춰도 pointer가 없는 세대를 가리키지 않습니다.
- `stellar_project::cache::migrate_layout`이 이를 부르고, 옛 pointer가 cache pointer를 바꿨으면 stamp를 지웁니다(짝을 모르는
  cache는 원본이 있으면 원본에서 읽음, ADR-0088 결정 1).
- `document_io::session::observe`(앱 창 열기·CLI·MCP 열기·semantic Git이 모두 쓰는 관찰)가 관찰 전에 `migrate_layout`을
  부릅니다. 옮기지 못하면 원본이 있을 때는 cache 없이 관찰하고, 원본이 없으면 pointer 읽기 오류입니다. 읽기 명령도 이
  이동을 합니다. 옮기는 것은 의미가 아닌 로컬 cache 배치이고 한 번뿐이기 때문입니다(아래 Alternatives).
- `stellar_project::cache::untrack_legacy`: 작업 트리에 텍스트 원본이 있으면 `git rm -r --cached`로 옛 배치 경로를 index에서
  뺍니다(작업 트리·HEAD는 그대로, 다음 commit이 옛 cache를 Git에서 지움). 원본이 없는 옛 chunk 프로젝트는 Git에 남은 옛
  cache가 유일한 Git 원본이라 건드리지 않습니다. `GitSession::migrate_cache`가 이를 감싸고 stage·unstage·commit이 먼저
  부릅니다. `stellar cache migrate`도 같은 함수를 씁니다.

### 4. cache를 원본에서 다시 만드는 경로와 GC를 연결한다

- `stellar_project::cache::rebuild_from_text`: 열기 계획이 `FromText`(원본은 있고 cache가 없거나 짝이 아님: clone 직후, Git
  pull·checkout 뒤)일 때만 원본 문서를 chunk로 직렬화하고 문서가 참조하는 이미지 record를 asset metadata chunk로 더해 새
  generation과 stamp를 씁니다. 원본이 Git 원본이라 옛 cache chunk는 이어 쓰지 않습니다. `Cache`·`Forward`면 쓰지 않습니다.
  원본 파일은 쓰지 않습니다. 앱 창 열기의 `FromText` 맞춤 쓰기(ADR-0088 결정 4)와 CLI·MCP 첫 쓰기(ADR-0113 결정 3)도 cache를
  다시 만들고, 명시적으로는 `stellar cache rebuild`입니다.
- `stellar_project::cache::collect_garbage`: 보존 정책(`config/storage-gc.yaml`, V.7.1, 코드에 수치를 두지 않고 config를
  읽음)으로 current·최근 정상 세대·stamp가 가리키는 세대를 보존하고, mark(V.7.2)에 없는 cache 파일을 유예 시간 뒤 지웁니다
  (V.7.3). cache가 Git 밖이라 Git 추적 파일 삭제가 아닙니다. `stellar cache gc`와 `stellar cache migrate`가 저장 잠금 안에서
  부릅니다.

### 5. semantic Git은 Git tree의 원본으로 cache를 만들어 연다

- Git tree(HEAD·index·후보·merge stage·HEAD 비교·Merge Composer source)를 materialize한 snapshot에는 cache가 없습니다.
  저장소 loader로 열기 전에 `stellar_project::cache::prepare_snapshot`이 snapshot 안에서 옛 배치를 옮기고 원본으로 cache를
  만듭니다(비공개 임시 폴더라 잠금 없음). ops Git 엔진(`open_snapshot_tree`), conflict 세 side(`conflict_open`), HEAD 비교
  (`saved_project::head`, 원본 파일만 materialize), Merge Composer(`side_semantic`), staged 재검증이 이 함수를 씁니다.
- stage 원천(작업 트리)은 작업 트리 cache가 원본과 짝인지와 관계없이 원본만 비공개 사본으로 복사해 엽니다
  (`cache::text_snapshot`). 원본이 Git 원본이기 때문입니다.
- 후보 index에는 원본 파일만 넣고 cache 경로는 넣지 않습니다. 원본 파일의 index mode(실행 bit)는 유지합니다. 원본이 없는
  index(옛 배치만 추적)는 변경 ID 단위 stage를 거부합니다(cache를 Git에 넣지 않으므로 표현할 수 없음, `text-source adopt`
  뒤 다시).
- generation 묶음(`SourceUnit::Generation`, 문서 전체)은 원본 프로젝트에서 문서를 이루는 원본 파일(컴포넌트 제외)도
  덮습니다. 원본 파일 하나하나도 원본 단위이지만, 문서 전체를 고르는 선택은 옛 배치와 같은 뜻이어야 합니다. 원본 단위를
  고른 stage의 기대 의미는 후보 tree의 원본이 읽히는 의미입니다(변경 ID만 고른 stage는 지금처럼 선택 결과).
- conflict 세션: 옛 배치 cache pointer(`design/current`) conflict는 텍스트 원본 프로젝트면 원본 단위가 그 경로를 지워
  해결하고(cache는 병합 뒤 열기가 원본에서 다시 만듦), 원본이 없는 옛 chunk 프로젝트면 의미 병합하지 않고 이유와 함께
  보고합니다(`UnitKind::Generation` 단위는 더 만들지 않음). Merge Composer target 쓰기는 cache 없이 원본에서 읽은 target도
  받습니다.
- 그래서 두 branch가 각각 저장한 뒤 병합하면 conflict 후보는 텍스트 원본 파일뿐입니다(`tests/integration/tests/
  git_cache_out.rs`).

### 6. 옛 배치로 commit된 library는 읽기 전용 사본에서 읽는다

library dependency cache entry는 library commit 하나를 읽기 전용으로 materialize한 폴더라 옮길 수 없습니다. ADR-0122 이전에
commit된 library(텍스트 원본이 아직 담지 못하는 Property·Slot 정의의 component library chunk를 옛 배치로 담음)는
`stellar_project::cache::legacy_view`가 옛 배치 파일만 비공개 임시 폴더로 복사해 그 안에서 cache 위치로 옮긴 뒤 읽습니다
(`library_entry`). 원래 entry는 바꾸지 않습니다. 새로 저장한 library의 chunk는 Git 밖이므로, Property·Slot 정의를 담은
library를 새로 배포하려면 그 정의를 텍스트 원본이 담을 수 있어야 합니다(후속).

### 7. `stellar cache` 명령

`stellar cache status|rebuild|gc|migrate <프로젝트>`(`stellar_ops::git_cache`). status는 위치·열기 계획·current·세대 수를,
rebuild·gc는 저장 잠금 안에서 4번 함수를, migrate는 옛 배치 이동 → (신뢰한 Git 저장소이고 원본이 있으면) index 정리 →
다시 만들기 → GC를 합니다. 원본 파일은 어느 명령도 쓰지 않습니다.

## Alternatives

- **프로젝트 밖 사용자 cache(`~/Library/Caches/<key>`)**: Git·편집기가 전혀 보지 않지만, key를 프로젝트 경로나 id로 정해야
  해 폴더 이동·복사·linked worktree마다 cache가 끊기거나 섞이고, stamp(`.stellar/cache/`)와 수명이 갈라집니다. ADR-0059가
  정한 위치와도 다릅니다. 탈락.
- **cache 위치를 프로젝트마다 고르는 이중 위치(옛 배치를 계속 읽음)**: 모든 경로 함수가 두 위치를 판정해야 하고 같은 저장
  의미가 두 곳에서 갈라집니다. 옛 배치는 한 번 옮기는 migration으로만 다룹니다. 탈락.
- **읽기 명령은 옛 배치를 옮기지 않음**: 옛 배치 chunk 프로젝트를 읽으려면 저장소가 옛 위치도 읽어야 합니다(위 이중 위치).
  옮기는 것은 Git이 추적하던 로컬 cache 파일을 작업 트리에서 치우는 일이고 의미는 바뀌지 않습니다. 원본이 있는 프로젝트는
  옮기지 못해도 원본으로 읽습니다.
- **migration이 자동으로 index를 정리하지 않고 사용자에게 맡김**: `design/current` 삭제가 unstaged로 남아 일부 commit에
  옛 cache가 남습니다. stage·commit 전에 원본이 있을 때만 정리합니다.
- **원본이 없는 옛 chunk 프로젝트도 index에서 뺌**: 그 프로젝트의 Git 원본이 사라집니다. 탈락.
- **generation 묶음을 원본 프로젝트에서 없앰**: 문서 전체 선택·컴포넌트 정의 제거의 동반 단위(ADR-0080 결정 4)가 표현되지
  않습니다. 묶음이 문서 원본 파일을 덮게 했습니다.

## Consequences

- 두 branch 저장 뒤 병합의 conflict 후보가 텍스트 원본뿐입니다. `design/current` conflict가 사라집니다.
- clone·checkout 직후 cache가 없어도 원본에서 읽고(FromText), 첫 쓰기·앱 열기·`stellar cache rebuild`가 cache를 만듭니다.
- cache GC가 Git과 무관하게 동작합니다.
- 기존 프로젝트를 처음 관찰하면 작업 트리의 `design/current`·`generations/`·`objects/`가 `.stellar/cache/chunks/`로 옮겨지고,
  stage·commit·`stellar cache migrate`가 원본이 있을 때 index에서 뺍니다. 다음 commit은 옛 cache 삭제를 담습니다.
- Property·Slot 정의를 component library chunk로만 담을 수 있는 library는 새 저장이 Git으로 배포되지 않습니다(결정 6).
  텍스트 원본이 그 정의를 담게 하는 일이 후속입니다.
- 원본이 없는 옛 chunk 프로젝트(root 문서 둘 이상, component library chunk)는 Git에 원본이 없어 변경 ID stage·의미 병합을
  하지 않습니다(이유와 함께 거부). `text-source adopt`(ADR-0113)로 옮기면 됩니다.
- 시험 fixture 가운데 chunk만 저장해 Git에 commit하던 것은 원본을 함께 쓰는 helper(`test_support::save_text_document`,
  project 시험 `save_text_document`)로 바꾸었고, generation 파일이 Git에 있음을 전제한 기대값(`design/current` conflict·
  commit 파일 목록·index chunk bytes 보존)은 원본 기준으로 고쳤습니다. 의미가 없어진 시험(index의 미지원 chunk bytes 보존)은
  지웠습니다.
- 커밋된 옛 배치 fixture(`crates/runtime/tests/fixtures/window-session-project`)는 옛 배치 migration fixture로 그대로 둡니다.

## Compatibility

- generation manifest·chunk envelope·pointer·journal manifest·stamp의 bytes 형식은 같습니다. 바뀌는 것은 위치뿐입니다.
- 보호 경로: `crates/storage`의 경로 상수와 `ChunkObjectPath::to_relative_path`·`Display`(이제
  `.stellar/cache/chunks/objects/...`), journal 대상 규칙(cache 안 파일 허용), 공개 모듈 `stellar_storage::location` 추가.
  C ABI·FFI 헤더·schemas·성능 예산 값은 바꾸지 않습니다.
- 공개 Rust API 추가: `stellar_project::cache::{migrate_layout, rebuild_from_text, prepare_snapshot, text_snapshot,
  collect_garbage, untrack_legacy, default_retention, project_id, LayoutMigration, Rebuild, GcReport, CacheError}`,
  `stellar_ops::git_cache::*`, `GitSession::migrate_cache`, `GitError::Cache`. 제거: `SessionUnit::ours_generation`(옛 배치
  generation 단위를 더 만들지 않음).
- CLI: `stellar cache status|rebuild|gc|migrate <프로젝트>`.
- 옛 commit을 읽는 Git 경로(HEAD·index·merge stage)는 snapshot 안에서 옛 배치를 옮겨 그대로 엽니다.

## Performance

- 열기 관찰은 옛 배치 확인(`design` 아래 세 항목 lstat) 하나가 늘어납니다. 옮기는 일은 한 번뿐이고 같은 볼륨 rename입니다.
- semantic Git은 Git tree snapshot마다 원본을 해석해 cache를 만듭니다(ADR-0088 Performance의 decode·직렬화 비용: 5만 node
  decode 약 1.2초). 지금까지는 tree의 generation을 그대로 열었으므로 대형 문서의 status·stage·병합이 그만큼 느려집니다. 텍스트
  원본만으로 semantic snapshot을 만드는 경로(status는 이미 그렇게 함)로 stage 경로를 옮기는 개선이 후속입니다.
- GC는 cache 폴더 목록과 보존 manifest만 읽습니다. 성능 예산 값은 바꾸지 않았고 기준 장비 실측은 하지 않았습니다.

## Security

- cache 폴더 구성 요소(`.stellar`·`cache`·`chunks`)가 symlink면 쓰기·GC가 따라가지 않습니다(기존 `fs_guard` 규칙을 새
  구성 요소에 적용). 옛 배치 이동도 symlink를 따라가지 않고 정규 파일만 옮깁니다.
- journal은 `.stellar/cache/chunks/` 아래만 새로 받고 journal 자신과 다른 로컬 상태는 계속 거부합니다.
- Git index 정리는 신뢰한 프로젝트의 `GatedGitRunner`로만 합니다(비신뢰면 프로세스 0).

## Migration

- 기존 프로젝트: 앱·CLI·MCP가 처음 관찰할 때 작업 트리의 옛 배치를 옮깁니다. Git index는 stage·unstage·commit 또는
  `stellar cache migrate <프로젝트>`가 원본이 있을 때 정리합니다. 그 뒤 commit하면 Git에서 옛 cache가 지워집니다.
- 옛 branch를 checkout하면 Git이 옛 배치를 다시 놓고, 다음 관찰이 다시 옮깁니다(같은 chunk는 지우고 pointer는 바꿈, stamp를
  지워 원본 기준으로 읽음).
- 되돌리기: `.stellar/cache/`를 지우면 다음 열기가 원본에서 cache를 다시 만듭니다.

## 개정(2026-11-24, 통합 판정: 원본 없는 옛 배치 보호)

- 작업 트리에 텍스트 원본 없이 옛 배치(`design/current` 등)만 있으면 옮기지 않습니다(`stellar_project::cache::migrate_working_tree`,
  `CacheError::LegacyWithoutSource`). 옛 배치는 그 프로젝트의 유일한 Git 원본일 수 있어, 옮긴 뒤 `git commit -a` 한 번으로
  원본이 Git에서 사라질 수 있기 때문입니다. 열기 관찰, `stellar cache status·migrate`, semantic Git의 cache 이동이 이 함수를
  씁니다. Git snapshot을 푼 임시 폴더는 작업 트리가 아니므로 `migrate_layout`을 그대로 씁니다.
- 이런 프로젝트는 지금 열기를 거부하고 이유를 알립니다. 옛 배치를 읽기 전용 사본(`cache::legacy_view`)에서 텍스트 원본으로
  바꾼 뒤 옮기는 변환 경로가 후속입니다.
- 저장소의 창 session 시험 fixture(`crates/runtime/tests/fixtures/window-session-project`)는 텍스트 원본만 담도록 다시
  만들었습니다. 열면 chunk cache를 원본에서 다시 만듭니다.
