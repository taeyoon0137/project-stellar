# ADR-0138: snapshot cache를 원본 digest로 다시 쓰고, 앱 유휴 GC를 연결하고, 프로젝트 범위 Context 목록을 되돌린다

## Status

Accepted (2026-11-24). engine lane `cache-perf-gc`의 결정입니다. ADR-0122 Performance("semantic Git은 Git tree snapshot마다
원본을 해석해 cache를 만듭니다")와 V.10.7 보고서의 남은 일("앱 runtime의 유휴 cache GC 일정은 연결하지 않았습니다"),
ADR-0123 Consequences("Git stage의 requirement·Slot 목록 전용 시험이 없음", "프로젝트 범위 snapshot에서 문서별 목록을
되돌리는 역투영은 없음")의 후속입니다. ADR-0122·ADR-0123 본문은 고치지 않습니다. 보호 경로(`crates/storage`·
`crates/core-types`·`crates/ffi`·`schemas`·`config/performance-budgets.yaml`)는 바꾸지 않습니다.

## Context

- ADR-0122 이후 Git이 담는 것은 텍스트 원본뿐이라 materialize한 Git tree(HEAD·index·후보·merge stage·HEAD 비교·Merge
  Composer side)에는 chunk cache가 없습니다. `stellar_project::cache::prepare_snapshot`이 snapshot마다 원본을 해석하고 chunk로
  직렬화해 cache를 쓴 뒤 저장소 loader로 엽니다. stage 한 번에도 index와 작업 트리 사본을 따로 엽니다. 2만 node 문서에서 이
  준비만 약 260 ms였습니다(아래 Performance).
- 그 준비 결과는 입력에만 달린 값입니다. cache가 없는 snapshot에서 원본으로 만든 generation은 1세대이고, chunk·manifest
  bytes는 프로젝트 id(`stellar.yaml`), 원본 파일 집합(`SourceBase::digest`), 제품 schema 번호, 직렬화 code가 같으면 같습니다.
- 저장소의 GC 시작 조건(V.7.4 `GcScheduler`: idle + 저장 없음)과 정리 함수(`cache::collect_garbage`, `stellar cache gc`)는
  있었지만 앱 runtime이 부르지 않았습니다. 교차 프로세스 저장 잠금(`stellar_ops::lock::SaveLock`)은 기다리지 않으므로
  정리가 잠금을 쥔 동안 앱의 저장이 시작되면 `Busy`로 실패합니다.
- 텍스트 원본 프로젝트의 Git semantic stage는 후보 문서를 원본으로 다시 쓸 때 index 원본의 Context를 그대로 썼습니다
  (`stellar_ops::git::text_candidate_files`). 그래서 requirement·Slot 공개 목록·key 같은 Context 변경을 고르면 다시 연 index
  의미가 후보와 달라 `ReopenMismatch`로 거부됐습니다. ADR-0123 결정 1의 "목록 변경은 semantic stage로 고를 수 있다"가 Git
  경로에서는 지켜지지 않았고, 이를 잡는 시험이 없었습니다.
- 프로젝트 범위 투영(`stellar_ops::diff::add_contexts`에 library)은 library 컴포넌트·Slot의 목록을 여러 문서의 합집합 하나로
  붙입니다. 어느 문서의 항목인지가 snapshot에 없습니다.
- 사용자 방향(2026-11-23): 엔진만 만들고 Swift UI·셸 FFI는 더하지 않습니다. 검증은 Rust 시험·CLI·MCP입니다.

## Decision

### 1. 같은 원본 digest의 snapshot generation을 보관하고 다시 쓴다

- `stellar_project::cache::SnapshotStore`: 사용자 임시 폴더 아래 `stellar-snapshot-cache/`. key(`SnapshotKey`)는
  `stellar-snapshot-cache/1`, build 식별자, `PRODUCT_SCHEMA_VERSION`, 프로젝트 id, 원본 digest를 이은 bytes의 hash입니다.
  build 식별자는 실행 파일 경로·크기·수정 시각의 hash입니다(crate version은 개발 중 바뀌지 않아 직렬화 code 변경을 가르지
  못함). 얻지 못하면 재사용하지 않습니다.
- 항목 `<key hex>/`은 작은 프로젝트 root처럼 cache 배치(`.stellar/cache/chunks/`)를 가집니다: current pointer, manifest,
  manifest가 가리키는 chunk object만. 비공개 임시 폴더에 다 쓴 뒤 rename으로 공개하므로 반쯤 쓴 항목은 보이지 않고, 같은
  key를 동시에 공개하면 먼저 끝난 쪽이 남습니다(내용이 같음).
- `prepare_snapshot`(그리고 `prepare_snapshot_with`)은 옛 배치를 옮긴 뒤 열기 계획이 `FromText`이고 snapshot에 cache가 전혀
  없을 때만 보관소를 씁니다. 꺼내기는 항목 manifest를 검증하고(`verify_generation`), chunk마다 envelope·hash를 다시
  확인하고(`ChunkStore::read_chunk`), snapshot에 hard link(다른 volume이면 임시 이름 복사 뒤 rename)로 놓은 뒤 current
  pointer와 stamp(원본 digest ↔ manifest)를 씁니다. 그래서 그 snapshot은 `Cache` 계획으로 평소처럼 열립니다. 하나라도
  실패하면 원본에서 만들고(current pointer를 쓰기 전이라 의미가 남지 않음), 원본에서 만든 결과를 보관합니다(실패는 무시).
  snapshot에 이미 cache가 있으면(옛 배치를 옮겨 그 다음 번호로 이음) 결과가 입력에만 달린 값이 아니므로 쓰지 않습니다.
- 작업 트리 사본(`cache::text_snapshot`, stage 원천)도 같은 경로입니다. 그래서 stage는 index·작업 트리를, unstage는 HEAD를,
  HEAD 비교(`saved_project::head`)·conflict 세 side(`conflict_open`)·Merge Composer·staged 재검증은 각 tree를 같은 원본이면
  원본 해석 없이 엽니다. 호출자는 바꾸지 않았습니다.
- 사용자 작업 트리의 cache는 보관소에 넣지도, 보관소 대신 쓰지도 않습니다(Alternatives).
- 폴더는 `0700`으로 만들고, symlink가 아닌 실제 폴더이며 group·other 비트가 없고, 이 사용자가 그 안에 만든 비공개 임시
  폴더와 소유자가 같을 때만 씁니다. 아니면 재사용하지 않습니다.
- 수명: 마지막 사용 시각(항목 current pointer의 수정 시각, 꺼낼 때 갱신) 기준 최근 16개(`MAX_ENTRIES`)만 남기고, 보존 정책의
  유예 시간(`config/storage-gc.yaml` `grace_period_seconds`)보다 오래 쓰지 않은 항목과 비정상 종료로 남은 공개 중 폴더를
  지웁니다(`SnapshotStore::prune`). 공개할 때마다와 앱 유휴 GC(결정 2)가 부릅니다. 16은 한 작업에서 함께 쓰는 tree 수
  (HEAD·index·작업 트리·병합 세 side)에 편집마다 바뀌는 작업 트리 사본을 넉넉히 더한 값입니다.

### 2. 앱 runtime의 유휴 GC는 document thread job 안에서 저장 잠금을 잡고 정리한다

- `AppRuntime::start_idle_gc(_with)`가 일정 thread(`stellar-idle-gc`)를 켜고 `IdleGc` handle을 돌려줍니다(drop하면 멈춤).
  thread는 관찰 간격(기본 15초)마다 background lane에 판정 job 하나를 같은 병합 key로 보냅니다.
- 판정 job(document thread)은 열린 프로젝트마다 V.7.4 `GcScheduler`를 둡니다. 그 프로젝트 창에 저장하지 않은 편집이나 진행
  중인 제스처가 있거나 자동 저장 쓰기 표시(`AppShared::autosave_writing`)나 이미지 가져오기 표시(`importing_image`)가 서
  있으면, 또는 cache current pointer가 바뀌었으면
  (이 앱이나 CLI·MCP가 저장함) 활동으로 기록합니다. 마지막 활동 뒤 idle(기본 60초)이 지나고 지금 바쁘지 않으며 지난 정리
  뒤 current가 바뀌었을 때만 정리합니다. scheduler를 처음 관찰할 때 만들므로 프로젝트를 연 직후는 정리하지 않습니다.
- 정리는 `SaveLock::for_identity`를 기다리지 않고 잡은 뒤 `collect_garbage`(보존 정책 `config/storage-gc.yaml`)입니다. 다른
  프로세스가 잠금을 쥐고 있으면 건너뛰고 다음 관찰에서 다시 시도합니다. 한 번이라도 정리했으면 snapshot 보관소도 같은 유예
  시간으로 정리합니다.
- 같은 프로세스의 저장과 겹치지 않는 근거: Save Now·자동 저장 캡처·stale 해결·에이전트 편집은 모두 document thread job에서
  시작하고, 자동 저장 writer는 캡처 job이 쓰기 표시를 세운 뒤에만 씁니다(표시는 결과 반영 job이 내림). 판정 job은 쓰기
  표시가 내려간 것을 본 뒤 같은 job 안에서 정리를 끝내므로, 그동안 이 프로세스의 저장은 시작되지 않습니다. 그래서 정리가
  잠금을 쥐어도 앱 저장이 `Busy`로 실패하지 않습니다. 저장 경로에 hook을 더하지 않았습니다.
- 60초·15초는 코드 상수(`app::gc::IDLE_THRESHOLD`·`POLL_INTERVAL`)입니다. 보존 수치(`config/storage-gc.yaml`)와 달리 저장
  의미가 아니라 앱 일정이고, 그 config 형식은 storage(보호 경로)가 엄격하게 읽어 key를 더하려면 storage 변경이 필요합니다.

### 3. Git semantic stage는 후보 Context를 index 원본에 쓴다

- `text_candidate_files`는 후보 문서의 Context를 index 원본 대신 후보 snapshot에서 레코드와 함께 되돌린 값
  (`diff::document_from_snapshot`, 영속 출력 `ContextSet::live`)으로 씁니다. token·이미지 record·컴포넌트 원본은 지금처럼 index
  원본 것입니다. Context는 문서 의미이고(ADR-0114 결정 2) 고른 변경만 후보에 들어 있으므로, 고르지 않은 Context 변경은 index
  값 그대로입니다.
- 시험: `git::tests::staging_a_context_list_of_a_text_source_writes_only_that_list_to_the_index_source`(시스템 Git, 텍스트 원본,
  목록 셋이 따로 status에 나오고 하나만 stage), `candidate::tests::staging_one_requirement_list_keeps_the_other_context_lists_as_they_were`
  (generation 직렬화 경로).

### 4. 프로젝트 범위 snapshot의 목록은 투영 전 문서별 Context를 기준으로 나눠 되돌린다

`stellar_semantic_diff::build::context::project_contexts_from_snapshot(snapshot, base)`:

- key·provision·Document 객체 목록은 그 문서 것(`context_from_snapshot`).
- library 대상(출처 `Origin::ProjectComponent`인 Component·Slot 객체) `T`의 목록 `M`: `base`에서 `T`에 목록을 둔 문서는 각자
  `기존 ∩ M`, `M`에서 어느 문서에도 없던 항목은 그 문서 모두에 더하고, `T`에 목록을 둔 문서가 없으면 `base` 첫 문서(투영
  기준 문서)에 둡니다.
- snapshot library에 없는 대상의 `base` 목록(투영이 넣지 않는 orphan)은 그대로 둡니다.
- 성질: 바꾸지 않은 snapshot은 `base` 그대로이고, 결과를 다시 투영하면 같은 snapshot입니다(대상마다 합집합이 `M`). 지금 병합
  결과를 쓰는 경로는 모두 문서 범위라 제품 호출자는 없습니다. 프로젝트 범위 결과를 쓰는 경로가 생기면 이 함수를 씁니다.

## Alternatives

- **원본만으로 semantic snapshot을 만들어 stage 경로에서 cache를 없앰**(ADR-0122 Performance가 적은 후속): stage·병합 해결
  저장은 저장소 loader가 연 `SavedProject`(generation manifest·chunk pointer 검사, 후보 직렬화)를 전제로 합니다. 그 경로를 모두
  원본 기준으로 다시 쓰는 일은 이 lane 소유 밖 파일(head·conflict_open·composer·candidate session)을 넓게 바꿉니다. 같은
  입력의 결과 재사용은 호출자를 바꾸지 않고 같은 효과(원본 해석 생략)를 냅니다. 보류.
- **프로젝트 안 `.stellar/cache/` 아래 보관소**: materialize한 snapshot은 작업 트리 밖 임시 폴더에 있어 HEAD 비교·conflict
  열기·Composer 같은 호출자가 작업 트리 경로를 넘겨야 합니다(소유 밖 호출자 변경). key가 내용 hash라 프로젝트별로 나눌 필요도
  없습니다. 사용자 임시 폴더는 snapshot과 같은 volume이라 hard link로 옮길 수 있습니다. 탈락.
- **같은 프로세스 메모리 memo만**: CLI·MCP는 명령마다 새 프로세스라 stage → status → commit 사이에 재사용이 없습니다. 탈락.
- **작업 트리 cache를 donor로 씀**(stamp가 짝이면 그 generation을 복사): 앱·CLI가 이어 쓴 generation은 원본만으로 만든 결과와
  chunk 구성이 같다는 보장이 없고(이미지 metadata 등, `saved_semantic_snapshot`의 재직렬화 비교가 거부할 수 있음), 다른 writer·GC와
  동시에 바뀝니다. 탈락.
- **key에 crate version**: 모든 crate가 `0.0.0`이라 직렬화 code가 바뀐 개발 build가 옛 항목을 씁니다. 실행 파일 식별자를
  씁니다.
- **꺼낼 때 chunk 재검증 생략**: loader가 다시 검증하지만, 그때 실패하면 사용자 명령이 실패합니다. 꺼내기 단계에서 확인해
  원본으로 돌아갑니다(2만 node에서 수십 ms).
- **유휴 GC를 별도 thread에서 실행하고 저장 경로에 대기 hook을 더함**: Save Now·자동 저장 writer·에이전트 편집 등 잠금을 잡는
  곳마다 hook이 필요하고(소유 밖), 정리를 취소할 수단이 storage sweep에 없습니다. document thread job 안 실행은 hook 없이
  상호 배제합니다. 대신 정리 시간만큼 document thread를 막습니다(2만 node, 6세대에서 약 2 ms).
- **GC가 잠금을 기다림**: 다른 프로세스의 긴 저장 동안 document thread가 멈춥니다. 건너뛰고 다시 시도합니다.
- **프로젝트 범위 투영을 문서별 속성(Component 객체 + 문서 id map key)으로 바꿔 역투영을 정확히 함**: `stellar diff`·compare의
  프로젝트 범위 출력과 병합 단위가 바뀌고 ops 투영(소유 밖)을 고쳐야 합니다. 합집합 투영을 유지하고 기준 문서별 Context로
  나눕니다.

## Consequences

- 긍정: 같은 원본의 Git tree를 다시 열 때 원본 해석·직렬화·chunk 쓰기를 하지 않습니다. CLI·MCP·앱 Git 경로가 프로세스를 넘어
  재사용합니다.
- 긍정: 앱을 오래 켜 두어도 cache가 보존 정책 밖 세대로 쌓이지 않고, 정리가 앱 저장을 실패시키지 않습니다.
- 긍정: 텍스트 원본 프로젝트에서 Context 변경(requirement·Slot 공개 목록·key·provision)을 Git semantic stage로 고를 수 있습니다.
- 부정: 사용자 임시 폴더에 항목이 최대 16개 남습니다(2만 node 문서 항목 하나가 chunk 수 MB, snapshot과 hard link로 공유).
  OS 임시 폴더 정리로 지워져도 다음 snapshot이 원본에서 다시 만듭니다.
- 부정: 유휴 GC 정리 동안 document thread가 막힙니다. 입력은 background lane job보다 먼저 처리되지만 정리 중에 온 입력은 정리가
  끝난 뒤 처리됩니다. 정리가 잠금을 쥔 동안 다른 프로세스(CLI·MCP)의 쓰기는 `Busy`입니다(지금 저장 경합과 같은 처리).
- 부정: 유휴 GC는 앱 runtime이 `start_idle_gc`를 불러야 켜집니다. 셸 UI 재설계 때 자동 저장과 함께 켜는 연결이 남습니다(지금
  셸은 빈 창, 사용자 방향).
- 부정: 프로젝트 범위 역투영은 합집합에 새로 생긴 항목을 그 대상에 목록을 둔 문서 모두에 더합니다. 한 문서에만 더한 변경도
  다른 문서에 퍼집니다(합집합에서는 구별할 수 없음). 제품 호출자는 아직 없습니다.

## Compatibility

- 저장 bytes·텍스트 원본·chunk·manifest·stamp·journal 형식은 같습니다. 보관 항목은 같은 형식의 사본입니다.
- 공개 Rust API 추가: `stellar_project::cache::{SnapshotStore, SnapshotKey, PruneReport, MAX_ENTRIES, STORE_DIR_NAME,
  SnapshotCache, prepare_snapshot_with}`, `stellar_runtime::app::gc::{IdleGc, IdleGcConfig, IdleGcOutcome, IdleGcRun,
  IDLE_THRESHOLD, POLL_INTERVAL, RUN_LIMIT}`, `AppRuntime::{start_idle_gc, start_idle_gc_with}`,
  `stellar_semantic_diff::build::context::project_contexts_from_snapshot`. `prepare_snapshot`의 반환형은 그대로입니다(재사용도
  `Rebuild::Rebuilt`).
- FFI·C 헤더·MCP catalog·CLI 명령은 바뀌지 않습니다.

## Performance

`cargo test -p stellar-ops --release --lib git_cache::snapshot_tests::measure -- --ignored --nocapture`, 개발 장비(다른 lane 빌드와
시험이 함께 도는 부하 상태), 2만 node 문서, 기준 장비(REF-MAC) 실측 아님:

| 측정 | 1회차 | 2회차(부하 큼) |
|---|---|---|
| snapshot 준비, 원본에서 만들기 | 254\~267 ms | 274\~547 ms |
| snapshot 준비, 보관 generation 재사용 | 56\~108 ms | 70\~101 ms |
| 준비 뒤 열기 + semantic snapshot | 62\~68 ms | 69\~170 ms |
| 처음 만들고 보관(공개 포함) | 279 ms | 346 ms |
| cache GC(6세대, 지울 파일 10개) | — | 2.1 ms |
| cache GC(지울 것 없음) | — | 0.6 ms |

재사용 시간의 대부분은 원본 파일 읽기·digest(key 계산)와 chunk 재검증입니다. 성능 예산 값은 바꾸지 않았습니다.

## Security

- 보관소는 이 사용자만 접근하는 폴더(`0700`, symlink 아님, 소유자 확인)일 때만 씁니다. 항목 manifest·chunk는 꺼낼 때 hash로
  다시 검증하므로 손상된 항목은 쓰지 않습니다. 같은 사용자 권한을 가진 프로세스의 변조는 막지 않습니다(그 프로세스는 작업
  트리도 바꿀 수 있음).
- 보관 항목에는 프로젝트 내용(비공개 저장소일 수 있음)이 chunk로 남습니다. 위치는 materialize한 snapshot과 같은 사용자 임시
  폴더이고, 유예 시간과 개수 상한으로 지웁니다.
- 유휴 GC는 cache 폴더만 지우고(기존 `collect_garbage`의 symlink 규칙), 신뢰 여부와 무관하게 Git을 실행하지 않습니다.

## Migration

없습니다. 보관소는 처음 쓸 때 만들어지고, 지워도 다음 snapshot이 원본에서 다시 만듭니다. 유휴 GC는 켜는 runtime에서만 동작합니다.
