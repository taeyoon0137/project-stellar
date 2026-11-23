# ADR-0088: 앱 기본 원본을 텍스트로 전환하고 chunk generation을 로컬 cache로 둔다

## Status

Accepted (2026-11-23). engine lane `text-source-default`의 결정입니다. ADR-0077(텍스트 원본 v4)·ADR-0078(저장
journal)·ADR-0083(창 session stale·외부 변경)을 앱 runtime 기본 경로에 연결합니다. ADR-0083 결정 9의 "앱 기본
열기·저장은 아직 chunk generation"을 이 ADR이 대체합니다.

## Context

- 사용자 결정(2026-11-14): 의미 단위 텍스트 파일이 Git 원본이고 chunk는 로컬 cache입니다. 계획 V.10.7은 검증된
  변환 경로에서만 기본 원본을 텍스트로 바꾸고, 새 프로젝트의 첫 저장, source hash 기반 빠른 열기·부분 로딩, cache
  GC를 함께 요구합니다.
- 텍스트 원본 codec(`source_v4::project`, V.10.4·V.10.8·V.10.9), 저장 journal(V.10.5), 엔진 session
  (`external_change::source::TextSourceSession`, V.10.6 일부)은 있었지만, 앱 창 session
  (`stellar_runtime::window_session::ProjectWindowSession`)의 열기·저장·자동 저장·stale·외부 변경은 모두 chunk
  generation만 읽고 썼습니다.
- CLI·MCP(`stellar_ops`)는 아직 generation을 읽고 씁니다. ops·CLI·MCP·semantic Git은 이 lane의 소유 밖입니다. 그래서
  원본을 텍스트로 바꾼 뒤에도 generation은 같은 의미로 유지되어야 하고, CLI가 쓴 generation도 앱이 받아야 합니다.
- 사용자 방향(2026-11-23): 엔진만 만들고 Swift UI·셸 FFI는 더하지 않습니다.

## Decision

### 1. 원본과 cache의 짝을 stamp로 기록하고 열기 계획을 정한다

- 원본은 `design/` 아래 `.stellar.yaml` 파일 집합입니다. 기본값 세트 파일(`design/style-defaults/`, ADR-0082 결정 4)은
  `stellar.yaml`이 고르는 프로젝트 설정이라 원본 집합에서 뺍니다(`document_io::session::read_source_dir`, 엔진
  session도 같은 함수를 씀).
- 원본 파일 집합의 digest(경로와 파일별 SHA-256을 길이와 함께 이은 bytes의 hash)와 그때의 current manifest hash를
  `.stellar/cache/text-source.stamp`에 씁니다. `.stellar/cache/`는 Git 밖 로컬 데이터이고(기술 아키텍처 §19), 처음
  쓸 때 `.stellar/cache/.gitignore`(`*`)를 함께 만듭니다.
- 열기·다시 불러오기·외부 변경 반영·UseCurrentWindow의 디스크 읽기는 `window_session::source::read_disk_state` 하나입니다.
  계획(`document_io::session::OpenPlan`)은 다음과 같습니다.

| 원본 | stamp와 비교 | 계획 | 읽는 것 | 맞춤 쓰기 |
|---|---|---|---|---|
| 있음 | 원본·cache 모두 같음 | `Cache` | generation에서 작업 문서만(부분 로딩) | 없음 |
| 있음 | 원본만 같고 cache가 움직임 | `Forward` | generation(CLI·MCP가 쓴 새 cache) | 원본 |
| 있음 | 원본이 다름, stamp 없음, generation 없음 | `FromText` | 원본 전체 | cache |
| 없음 | generation 있음 | `Migrate` | generation | 원본(처음 전환) |
| 없음 | generation 없음 | `Empty` | 없음 | 첫 저장이 원본을 만듦 |

- 원본과 cache가 함께 움직였으면 원본이 이깁니다(Git 원본). 덮인 generation은 `design/generations`에 남습니다.

### 2. 기존 chunk 프로젝트는 검증된 변환이 있을 때만 원본으로 옮긴다

`Migrate`·`Forward`는 다음을 모두 만족할 때만 텍스트 원본 session이 됩니다.

- generation에 component library chunk가 없습니다(그 chunk와 `design/components` 원본이 함께 있으면 ops가 거부하므로
  옮기지 않습니다).
- `from_saved`가 generation의 모든 chunk를 소비합니다(root 문서 하나, 다시 직렬화한 chunk 집합이 저장된 것과 같음).
- 인코딩한 원본을 다시 읽은 `semantic_key`가 원본 의미와 같습니다.
- `Migrate`에서 `design/` 아래 기존 `.stellar.yaml`(컴포넌트 원본)은 모두 인코딩 결과의 경로여야 합니다. 그 파일의 기대
  hash는 지금 bytes이고, 같은 bytes면 쓰지 않습니다.

하나라도 실패하면 session은 chunk 전용(`SourceMode::ChunkOnly`, 이유 문장 `chunk_only_reason`)으로 남고 원본을 쓰지
않습니다. chunk 전용 session의 저장·외부 변경은 이전과 같은 generation 경로입니다. `Forward`의 token set은 원본 것을
씁니다(generation에는 token이 없음).

### 3. 저장은 같은 잠금 안에서 generation 확인 → 원본 → cache → stamp 순서다

- `SaveCapture`는 원본 base(파일별 hash)와 문서 밖 의미(`SourceLibrary`: 컴포넌트·token set·이미지 record)를 함께
  캡처합니다. cache로 연 session은 문서 밖 의미를 모르므로 writer가 base와 같은 디스크 원본에서 읽습니다. 다르면 쓰지
  않고 `SourceStale`입니다.
- writer는 문서 레코드와 문서 밖 의미로 원본 파일 집합을 인코딩하고(writer thread), 공용 저장 잠금(`SaveLock`) 안에서
  generation base를 확인한 뒤 바뀐 원본 파일만 저장 journal 한 거래로 씁니다(기대 hash = 원본 base). 그다음 cache
  generation, 마지막으로 stamp입니다.
- 원본이 외부에서 바뀌었으면 `WindowSessionSaveError::SourceStale`이고 아무것도 쓰지 않습니다. 원본과 generation이 함께
  움직였으면(다른 앱 창의 저장) generation `Stale`로 알립니다(두 선택지 판정이 generation writer 기록을 먼저 봄).
  `WindowSessionSaveError::is_stale`이 두 경우를 함께 다룹니다(자동 저장 멈춤·알림·Save Now 실패 종류).
- 원본을 쓴 뒤 cache 쓰기가 실패하면 저장은 성공으로 반영하고 `SaveOutcome::cache_stale`로 알립니다. stamp가 옛 짝이라
  다음 열기가 `FromText`로 cache를 다시 만듭니다.
- 원본에 새로 들어온 이미지 record와 원본 catalog의 이미지 가운데 문서가 참조하고 base generation에 없는 것은 cache에
  asset metadata chunk로 더합니다(원본에서 다시 만든 cache도 이미지를 그림).

### 4. 맞춤 쓰기는 열기에서만 동기로 하고, 외부 변경 뒤에는 writer가 한다

- 열기(`ProjectWindowSession::load`)는 원본을 읽기 전에 `.stellar/journal/`에 남은 저장 거래를 복구합니다
  (`stellar_storage::journal::recover`, ADR-0078). 중단된 원본 저장의 반쯤 바뀐 파일 집합을 원본으로 읽지 않기
  위해서입니다. journal 폴더가 없으면 아무것도 만들지 않고, 다른 저장이 journal을 잡고 있거나(`Busy`) 복구가 실패해도
  열기는 계속합니다(다음 저장 거래가 시작할 때 다시 복구함).
- 열기는 계획이 요구하면 연 직후 같은 저장 경로로 원본 또는 cache를 씁니다. 실패해도 session은 열리고(로그)
  `reconcile_pending`을 세워 다음 저장(자동 저장 writer·창 닫기 flush)이 다시 맞춥니다. 열기는 이미 document thread에서
  프로젝트를 읽는 일회성 작업입니다.
- 외부 변경 반영(`sync_external`)과 UseLatestDisk 다시 불러오기는 맞춤 쓰기를 document thread에서 하지 않고
  `reconcile_pending`만 세웁니다(기술 아키텍처 §6.4 "저장은 인터랙션 동기 경로에서 실행 금지"). 자동 저장 일정과 창 닫기
  flush는 `needs_write()`(미저장 편집 또는 맞춤 쓰기 남음)를 보고 writer thread에서 씁니다. 맞춤 쓰기는 미저장 편집이
  아니므로 `has_unsaved_changes`(셸 dirty 표시)에 들지 않습니다.

### 5. stale은 generation과 원본을 함께 판정한다

- `stale_judgement`는 current pointer와 원본 파일 집합을 각각 base와 비교합니다. 원본은 파일 hash를 비교하므로 판정마다
  원본 bytes를 읽습니다(감시 event·선택 실행 때만 부름).
- 앱 창 저장은 원본을 쓰기 전에 새 원본 digest를, generation을 쓰기 전에 manifest hash를 `record_app_window_save`로
  기록합니다. 판정은 두 관찰값 가운데 하나라도 기록에 없으면 외부 writer입니다(CLI·MCP는 cache만, Git·편집기는 원본만
  쓰고, 원본과 cache를 함께 쓰는 writer는 앱 창뿐). 외부 writer와 dirty가 겹치면 ADR-0083 결정 2와 같이 reconciliation
  대기입니다.
- UseCurrentWindow는 판정한 generation과 원본이 그대로일 때만 씁니다(최신 원본 hash를 기대 hash로). 판정 뒤 다른 writer가
  썼으면 다시 stale입니다.
- 문서 밖 의미 파일(`components/`·`tokens/`, `document_io::session::is_library_path`)만 바뀐 것은 문서 stale이 아닙니다.
  그 의미는 창 history 밖이고(ADR-0079 결정 2) 창 저장은 그 파일을 바꾸지 않으므로(인코딩 결과가 base와 같아 쓰지 않음),
  `sync_external`이 편집 여부와 관계없이 디스크 의미와 base를 받고 stamp 맞춤만 writer에 남깁니다. cache로 연 session의
  writer도 디스크 원본이 그 파일에서만 다르면 디스크 의미로 인코딩합니다. 이미지·Text style catalog는 문서 Text style 표를
  담으므로 문서 파일로 봅니다.

### 6. 감시는 원본 파일 event도 반영 job으로 보낸다

`stellar-external-watch`는 `FileWatchEvent::affects_text_source` event가 있으면 마지막 event 뒤 조용한 구간(30 ms)이
지난 poll에서 그 프로젝트 창들의 반영 job을 보냅니다. 자기 저장 event는 창의 hash 비교가 거릅니다. 감시기가 없으면
같은 주기로 `design/` 원본 metadata(경로·크기·수정 시각) 지문을 비교합니다(내용은 읽지 않음).

### 7. 활성 기본값 세트 거울을 열 때 넣고 바꾼 창만 되쓴다

열 때 `stellar.yaml`의 활성 세트 이름(`document_io::session::active_style_defaults`, 매니페스트 해석은
`manifest_style_defaults::active_name`)을 모델 거울(`DocumentModel::set_style_defaults`)에 넣습니다. 저장 때 거울이 열 때·
마지막 저장 때의 값과 다르면 같은 잠금 안에서 `manifest_style_defaults::set_active`로 되씁니다. 바꾸지 않은 창은 다른
쓰기가 바꾼 `stellar.yaml`을 되돌리지 않습니다. 외부에서 바뀐 활성 세트는 clean 창에서도 거울에 반영하지 않고 다시 열 때
읽습니다(거울은 레코드 밖이라 `ApplyExternalSnapshot` 변경 집합에 들지 않음).

### 8. 이번에 연결하지 않은 것

- **chunk의 물리 위치**: generation은 아직 `design/current`·`design/generations`에 있어 Git이 추적합니다. 역할은 파생
  cache이지만, `.stellar/cache/`로 옮기거나 Git에서 빼려면 ops·CLI·MCP·semantic Git의 generation 읽기를 텍스트 원본으로
  먼저 바꿔야 합니다(소유 밖). 그 전에 cache GC가 옛 generation을 지우면 Git 추적 파일 삭제가 되므로 GC는 연결하지
  않았습니다.
- **새 프로젝트 첫 저장**: `stellar_project::create`가 최초 commit 전에 generation 1을 쓰는 경로는 소유 밖입니다. 앱이 그
  프로젝트를 처음 열 때 `Migrate`로 원본을 만듭니다. 최초 commit에 원본을 넣는 일은 create 경로 소유 작업입니다.
- **dirty 창의 3-way reconciliation(XIII.3.4)**: runtime에서 semantic merge를 부르려면 ops가 메모리 레코드 세 snapshot을
  받는 API를 내거나 crate graph에 runtime → semantic-merge 간선이 있어야 합니다(둘 다 소유 밖). 외부 writer와 dirty가
  겹치면 ADR-0083 결정 2대로 대기합니다.

## Alternatives

- **원본만 쓰고 generation을 바로 버림**: CLI·MCP·semantic Git이 아직 generation을 읽으므로 앱 편집이 그 도구에서
  사라집니다. 탈락.
- **cache를 먼저 쓰고 원본을 나중에 씀**: 원본 쓰기 실패 때 cache만 앞서고 session의 원본 base가 디스크와 어긋나 다음
  저장이 자기 저장을 stale로 오판합니다. 원본 먼저면 cache 실패는 stamp가 옛 짝이라 다음 열기가 저절로 고칩니다.
- **원본과 cache가 함께 움직였을 때 cache를 따름**: Git 원본 원칙과 어긋납니다. 원본이 이기고 옛 generation은 남깁니다.
- **변환이 검증되지 않아도 원본으로 전환(정보 손실 허용)**: 계획 V.10.7 "검증된 전체 변환 경로에서만"에 어긋납니다.
- **외부 변경 뒤 맞춤 쓰기를 document thread에서 동기로**: 대형 문서에서 저장 시간(아래 Performance)이 인터랙션 경로를
  막습니다. writer로 미룹니다.
- **stamp를 generation manifest에 기록**: 보호 경로 storage 포맷 변경입니다. 로컬 파일 하나로 같은 판정을 냅니다.

## Consequences

- 앱 창의 열기·저장·자동 저장·stale 두 선택지·외부 변경 반영이 텍스트 원본을 Git 원본으로 씁니다. CLI·MCP가 쓴
  generation은 열거나 반영할 때 원본으로 옮겨지고, Git pull·편집기로 바뀐 원본은 cache를 다시 만듭니다.
- 기존 chunk 프로젝트를 앱에서 처음 열면 `design/` 아래 텍스트 원본 파일과 `.stellar/` 로컬 파일이 생깁니다(Git에는 새
  파일로 보임).
- root 문서가 둘 이상이거나 component library chunk가 있는 프로젝트는 chunk 전용으로 남습니다.
- 저장 비용은 원본 인코딩과 journal fsync만큼 늘어납니다(Performance). 자동 저장은 writer thread라 인터랙션은 막지 않습니다.
- 자동 저장 실패 시험은 cache가 아니라 원본 journal을 막아 실패를 만듭니다(cache 실패는 저장 실패가 아님).

## Compatibility

- 파일 포맷·generation manifest·journal 형식·C ABI·FFI 헤더를 바꾸지 않습니다. 보호 경로 가운데 `crates/ffi`는 시험 helper
  하나만 바꿉니다: `crates/ffi/src/window_session.rs`의 시험용 `next_event`가 요청 없는 `WindowViewChanged`(글꼴 읽기 완료 등)를
  건너뜁니다. 열기가 원본 전환으로 조금 길어지면서 그 event가 요청 완료보다 먼저 오는 경쟁이 잦아졌기 때문입니다. 제품
  코드·C ABI는 그대로입니다.
- 새 로컬 파일: `.stellar/cache/text-source.stamp`(3줄: 형식 이름 `stellar-text-source-stamp 1`, `source <hash>`,
  `manifest <hash>`), `.stellar/cache/.gitignore`. 읽을 수 없는 stamp는 짝을 모르는 cache로 봅니다.
- 공개 Rust API 추가: `stellar_project::document_io::session`(`SourceBase`·`SourceFiles`·`SourceLibrary`·`Stamp`·
  `OpenPlan`·`read_source`·`read_source_dir`·`commit`·`commit_files`·`plan_changes`·`read_stamp`·`write_stamp`·
  `active_style_defaults`), runtime `ProjectWindowSession::{open_plan, chunk_only_reason, source_base,
  reconcile_pending, needs_write}`, `SaveOutcome::{source_files, cache_stale, wrote_anything}`,
  `WindowSessionSaveError::{SourceStale, Source, StyleDefaults, is_stale}`, `WindowSessionLoadError::Source`.
- `stellar_project::external_change::source::DESIGN_DIR`는 `document_io::session::DESIGN_DIR` 재수출로 바뀌었습니다(값 같음).
- `RuntimeEvent::WindowSessionSaved::written`과 flush의 `Saved`는 원본 파일만 쓴 저장도 참입니다.

## Performance

DEV-MAC(Apple Silicon, release, 다른 lane 5개가 동시에 빌드·시험하던 부하 상태) 실측입니다. 기준 장비(REF-MAC, M1 16GB)
실측과 열기·저장 예산 판정은 남아 있습니다. 기술 아키텍처에 프로젝트 열기·저장 시간의 수치 예산은 없고, §6.4는 저장을
인터랙션 동기 경로에서 금지합니다. 측정: `cargo test -p stellar-runtime --release --lib window_session::source_bench --
--ignored --nocapture`.

| 문서 node | 원본 파일 | Migrate 열기 | Cache 열기 p50 | FromText 열기 | 이름 하나 저장 p50 |
|---:|---:|---:|---:|---:|---:|
| 1,011 | 3 | 625 ms | 6.9 ms | 190 ms | 367 ms |
| 10,101 | 93 | 5,987 ms | 54 ms | 1,715 ms | 1,244 ms |
| 50,201 | 199 | 19,895 ms | 266 ms | 12,474 ms | 7,982 ms |

부분 측정(같은 문서, 부하가 적을 때): chunk 직렬화 1.3·13·66 ms, `DocumentModel::from_records` 1.0·14·65 ms, 텍스트
원본 encode 66·966·5,793 ms, decode 22·210·1,233 ms.

- `Cache` 열기는 generation 부분 로딩이라 원본 크기와 거의 무관합니다(빠른 열기). stamp 확인을 위해 원본 bytes를 읽고
  hash합니다.
- 저장·FromText·Migrate는 텍스트 codec이 지배합니다. encode가 node당 약 60\~115 µs로 chunk 직렬화보다 약 50배 느리고 node
  수에 대해 선형보다 조금 빠르게 늡니다. 작은 문서의 저장은 journal fsync(파일·디렉터리마다)가 대부분입니다.
- 결론: 기본 원본 전환은 동작하지만, 저장마다 전체 원본을 다시 인코딩하는 비용이 5만 node 문서에서 수 초입니다. codec
  (`source_v4`, 소유 밖)의 증분 인코딩(바뀐 View·Frame 파일만)과 encode 비용 개선이 REF-MAC 예산 판정 전에 필요합니다.

## Security

- stamp·진단은 경로를 `design/` 기준 상대 경로로만 씁니다. 원본 저장은 journal 경로 규칙(`..`·절대 경로·`.git`·
  `.stellar/` 거부)과 symlink 거부를 그대로 따릅니다.
- 변환 때 컴포넌트 원본은 외부 symlink target을 읽지 않는 정책(`ExternalAccess::MetadataOnly`)으로 읽습니다.

## Migration

- 기존 chunk 프로젝트: 앱에서 처음 열 때 결정 2의 검증을 통과하면 원본 파일을 만들고 stamp를 씁니다. generation은 그대로
  둡니다. 통과하지 못하면 아무것도 쓰지 않고 chunk 전용으로 엽니다.
- 되돌리기: `design/` 아래 원본 파일과 `.stellar/cache/text-source.stamp`를 지우면 다음 열기가 다시 `Migrate`입니다.
