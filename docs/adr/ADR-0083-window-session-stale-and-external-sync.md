# ADR-0083: 창 세션 stale 두 선택지, writer 출처, 외부 변경 반영과 종료·flush 순서

## Status

Accepted (2026-11-23). engine lane `window-session-stale`의 결정입니다. ADR-0072 §6·§7("남은 일(UX.4.3)")을
보완합니다. ADR-0072 본문은 고치지 않고, 이 ADR이 stale 두 경로·외부 변경 반영·flush 결정 상태를 정합니다.

## Context

- 기술 아키텍처 §22.2는 같은 프로젝트를 연 두 창 가운데 두 번째 창이 저장할 때 "이 창의 버전으로 저장" /
  "디스크 버전 불러오기" 두 선택지만 두도록 정합니다. 구현은 stale을 감지하면 자동 저장을 멈추기만 했고(ADR-0072
  §6), 두 선택을 실행하는 코어 경로가 없었습니다(ADR-0072 §7 남은 일).
- CLI·MCP·Git이 같은 파일을 쓰는 흐름(아키텍처 §22.5, ADR-0079)에서는 열린 창이 외부 변경을 command 하나로
  받아 Undo 한 번으로 되돌릴 수 있어야 합니다. persistent-commands lane이 `ApplyExternalSnapshot`을 만들었고 runtime
  연결이 남아 있었습니다.
- multi-file-journal lane(ADR-0078)의 `journal::commit`·`recover`와 text-source-codec lane(ADR-0077)의 다중 파일
  원본이 저장·열기 경로에 연결되지 않았습니다(V.10.6). 기본 원본 전환(V.10.7)은 이번 범위가 아닙니다.
- 사용자 방향(2026-11-23): 엔진·기능만 만들고 Swift UI·셸 FFI는 더하지 않습니다. 기능은 Rust 시험·CLI·MCP로 봅니다.
- Windows는 ADR-0006에 따라 미룹니다. XII.2.4의 선행 XII.2.3(Windows)은 이 lane에서 빼고 macOS 기준으로 진행합니다.

## Decision

### 1. stale 결과는 두 값의 core enum이다 (XII.7.2·XII.7.5·XII.7.6)

- `stellar_project::window_session::StaleSaveChoice`는 `UseCurrentWindow`·`UseLatestDisk` 두 값뿐입니다. 안정 code
  (`use-current-window`·`use-latest-disk`)와 셸 경계 숫자(1·2, 0은 쓰지 않음)를 golden test로 고정합니다. merge·시각
  diff 선택지는 없습니다.
- **UseCurrentWindow**(`ProjectWindowSession::resolve_stale`의 `UseCurrentWindow`): 디스크 최신 generation을 다시 읽어 새 base로
  삼고, 다른 문서·에셋 chunk는 최신 것을 잇고 이 창 문서가 최신 generation에서 차지한 chunk만 창 문서로 바꿉니다.
  쓰기는 일반 저장과 같은 `SaveCapture::write`이므로 공용 저장 잠금 안에서 base를 **commit 직전에 다시 확인**합니다.
  최신을 읽은 뒤 commit 전에 다른 저장이 끝나면 `Stale`이고 아무것도 쓰지 않습니다(다시 stale). 성공하면 디스크의
  이 문서는 창 snapshot과 같고 history는 그대로 둡니다. 최신 generation에서 이 문서가 지워졌으면 창 문서를 새 root로
  더합니다. 다시 읽은 최신 generation이 `resolve_stale`이 writer 출처를 판정한 generation과 다르면(판정 뒤 CLI 등
  다른 writer가 저장함) 쓰지 않고 `Stale`입니다. 그래서 외부 writer의 generation을 두 선택지 경로로 덮어쓰지 않습니다.
  이 쓰기의 공개 진입점은 `resolve_stale` 하나이며, 출처 판정 없이 최신 위에 덮어쓰는 공개 API는 두지 않습니다.
- **UseLatestDisk**(`ProjectWindowSession::reload_latest_disk`): 최신 generation을 검증하고 이 창 문서를 끝까지 읽은
  **뒤에만** 미저장 편집·history·선택을 버립니다. 읽기에 실패하면 메모리 편집과 history를 그대로 둡니다. 문서
  revision은 앞으로만 움직여(이전 + 1) revision 기반 cache가 무효화됩니다. 열린 탭은 새 문서에 남은 View만 viewport와
  함께 유지합니다.
- `ProjectWindowSession::resolve_stale`은 디스크가 base와 같으면 `NotStale`, 외부 writer면 `ReconciliationRequired`
  (결정 2)로 선택을 실행하지 않습니다. 실패는 어느 경우도 문서·history를 바꾸지 않습니다.

### 2. writer 출처는 이 프로세스의 앱 창 저장 기록으로 가른다

- 앱 창 저장(`SaveCapture::write`, Save Now·자동 저장·UseCurrentWindow 모두)은 공용 저장 잠금 안에서 current
  pointer를 바꾸기 직전에 `record_app_window_save(canonical root, manifest hash)`를 부릅니다. pointer 변화를 다른 창의
  감시가 먼저 보고 앱 창 저장을 외부로 오판하지 않게 하려는 순서입니다. stale을 본 창은 `writer_provenance`로 디스크
  generation이 앱 창 것인지(`AppWindow`) 아닌지(`External`: CLI·MCP·Git·다른 프로세스) 가릅니다. 기록은 프로세스
  메모리의 최근 4,096개(root + hash 쌍)입니다.
- 앱 창이 쓴 generation이면 두 선택지(`StaleResolution::Choose`), 외부 writer와 dirty가 겹치면 두 선택지가 아니라
  `StaleResolution::ReconciliationPending`입니다. XIII.3.4 semantic reconciliation이 연결되기 전까지 이 상태로 남고
  자동 저장도 멈춘 채입니다. 창을 닫으면 D6 시트(ADR-0072 §6)로 갑니다.
- lane 결정 원문은 writer provenance를 generation·journal manifest에 기록하라고 했습니다. generation manifest와
  journal manifest는 보호 경로 `crates/storage`의 파일 형식이고 이 lane의 소유 범위가 아닙니다. 그래서 **포맷을 바꾸지
  않고** 프로세스 기록으로 같은 판정(앱 창 vs 그 밖)을 냅니다. CLI·MCP·Git을 서로 구분하는 일(XXIV.4.6)은 manifest
  필드가 필요하며 그 행에서 storage ADR과 함께 정합니다. 앱을 다시 시작하면 기록이 비므로, 재시작 뒤 처음 만나는
  다른 창 generation은 외부로 봅니다(보수적: 덮어쓰기 선택지를 주지 않음).

### 3. clean 창의 외부 변경은 ApplyExternalSnapshot 하나로 반영한다 (UX.4.1·UX.4.4)

- `ProjectWindowSession::sync_external`: current pointer가 base와 같으면(자기 저장 event 포함) `Unchanged`로 hash만
  비교해 무시합니다. 제스처(history preview) 중이면 `Deferred`, 미저장 편집이 있으면 `Stale`(결정 2), clean이면 새
  generation을 검증·로드하고 `ApplyExternalSnapshot::between`의 레코드 변경 집합을 transaction 하나로 이 창 history에
  기록합니다. Undo 한 번이 외부 변경 전체를 되돌리고(결과는 디스크와 달라 dirty), 선택은 남은 node만 유지하며
  (`WindowHistory::record_document`의 prune), 없어진 View의 탭은 닫습니다. 반영 뒤 base·소유 chunk·저장 revision은 새
  generation입니다.
- 깨진 generation·문서(손상, schema 거부, 다른 문서, validator 실패)는 `Rejected`로 아무것도 반영하지 않습니다.
- 다른 앱 창의 저장도 외부 변경입니다. clean 창은 그 저장을 같은 경로로 받습니다(XIII.3.1과 같은 규칙). 그래서
  두 창 stale 시나리오는 두 창이 모두 dirty일 때만 생깁니다.
- 앱 runtime 감시 thread(`stellar-external-watch`)가 열린 프로젝트마다 FSEvents와 coalescer(XIII.2.5)를 두고, 조용한
  구간마다 current pointer 변화가 있으면 그 프로젝트 창들의 반영 job을 command lane에 보냅니다. coalescer 자기 저장
  기록은 비워 두고 판정은 창의 hash 비교가 합니다. 감시를 붙이기 전의 변화를 놓치지 않게 첫 poll에서 한 번 반영합니다.
  진행 중인 쓰기로 미룬 반영은 그 쓰기 결과를 반영하는 자리(collect)에서 실행하고 그 창에 요청 없는 표시 변경을
  알립니다. 제스처로 미룬 반영은 다음 상태 게시(`build_state`의 관찰)에서 실행하며, 제스처를 끝낸 요청의 표시 변경
  event가 그 결과를 셸에 알립니다. 신뢰 프로젝트의 외부
  symlink target 감시 root는 `ExternalTargetSync::sync`로 시작 때와 넘침·링크 변화 event 때 맞춥니다(XIII.4.4).
  감시기를 만들 수 없으면 같은 주기로 current pointer bytes를 비교합니다.
- 반영을 거부한 진단은 창 상태에 남기고(`AppRuntime::external_status`), 텍스트 원본은 결정 7의 파일·줄 진단을 냅니다.

### 4. 자동 저장 중 stale은 자동 저장을 멈추고 알림을 한 번만 낸다 (XII.7.7)

- `AutosaveSchedule::take_stale_alert`는 stale 구간마다 한 번만 참입니다. clean을 관찰한 뒤의 새 stale에서만 다시
  알립니다. 저장 밖(감시 경로)에서 stale을 발견하면 `mark_stale`로 같은 상태가 됩니다.
- `AppRuntime::take_stale_alerts`가 아직 꺼내지 않은 알림을 돌려줍니다(셸 이벤트·FFI는 더하지 않음). 선택 전에는
  자동 저장·flush·Save Now가 디스크를 바꾸지 않습니다. `AppRuntime::resolve_stale`은 진행 중인 자동 저장을 먼저
  반영한 뒤 선택을 실행하고, clean이 되면 자동 저장이 다시 동작합니다.

### 5. XII.2.4는 macOS 기준이다

같은 project path를 여러 창에서 열면 각 창이 독립 base generation과 memory model을 로드합니다(XII.2.1 구현 그대로).
선행 XII.2.3(Windows)은 ADR-0006에 따라 이 lane에서 빼고, macOS 판정만 기록합니다.

### 6. thread 종료 순서를 고정한다 (XII.1.6)

- `stellar_runtime::document_thread::ShutdownStage::ORDER` = UI stop → command drain/cancel → render stop → worker
  stop → runtime destroy. `RuntimeThreads::shutdown`(drop도 같음)만 이 순서로 종료합니다. drain은 접수를 닫고 남은
  job을 실행하며, 제한 시간을 넘기면 시작하지 않은 job을 버립니다(`JobError::Lost`). render·worker는 drain 뒤에
  멈추므로 drain 중 job의 render 전달·worker 제출이 막히지 않습니다.
- 앱 runtime(`AppRuntime`)은 render thread를 소유하지 않습니다. 종료는 감시 thread·자동 저장 시계 정지(새 job 없음) →
  document thread drain → writer(worker) 대기 순서로, 같은 고정 순서의 부분입니다.

### 7. 닫기·비활성화·Git UI flush는 결정 상태를 돌려주고 편집을 버리지 않는다 (XIII.1.6·XIV.2.8)

- `AppRuntime::flush_for_close(session, timeout)`·`flush_for_deactivate(timeout)`: 진행 중인 자동 저장을 반영하고
  남은 편집을 저장합니다. 결과는 `FlushOutcome::{Clean, Saved, Blocked(FlushBlock)}`이고 `FlushBlock`은 시간 초과·
  실패(경로 없는 code)·stale(두 선택지 또는 reconciliation)·제스처입니다. 시간 초과여도 넘긴 flush job은 끝까지
  실행되며 취소·폐기하지 않습니다. 셸은 `Blocked`이면 사용자에게 묻습니다.
- `AppRuntime::git_ui_preflight`는 쓰지 않고 `GitUiPreflight::{Clean, PendingEdits, Blocked}`를 돌려줍니다.
  `prepare_git_ui(choice)`의 `SaveFirst`는 닫기와 같은 flush, `ContinueWithDisk`는 아무것도 쓰지 않고 메모리 편집을
  유지합니다.
- 자동 저장마다 dirty revision·캡처·직렬화·쓰기 시간·결과 code만 담은 trace를 창별 최근 32개 남깁니다(XIII.1.7).
  한 줄 JSON의 key 집합·순서가 schema이고, 문자열 값은 schema 이름과 결과 code뿐이라 경로·이름이 들어갈 자리가
  없습니다.

### 8. 자동 저장과 복구 체크포인트는 같은 불변 snapshot을 공유한다 (XIII.8.2)

`ProjectWindowSession::shared_snapshot`은 (문서, revision)마다 레코드를 한 번 복사하고, chunk 직렬화는 먼저 필요한
worker가 `OnceLock`으로 한 번만 합니다. 자동 저장 캡처와 복구 `Capture`가 같은 `Arc`를 씁니다. 그래서 같은 revision의
문서 직렬화는 최대 한 번입니다(ADR-0072 Performance의 후속).

### 9. 텍스트 원본 session을 journal·감시 경로에 연결한다 (V.10.6)

- `stellar_project::external_change::source::TextSourceSession`: 열 때 `journal::recover`를 먼저 부르고 원본 전체를
  읽어 검증합니다. 저장은 모델을 다시 encode해 base와 다른 파일만 journal 한 거래로 쓰고, 각 변경의 기대 hash는 열거나
  마지막으로 쓴 bytes입니다. 그 사이 외부가 바꾼 파일이 있으면 `Stale`이고 아무것도 쓰지 않습니다.
- `poll_external`: hash가 base와 같으면 무시, 깨진 원본은 반영 0과 `SourceDiagnostic`(경로, 문법 오류면 줄·열,
  원인 code), dirty면 메모리 보존과 대기(merge는 XIII.3.4), 제스처 중이면 미룸, clean이면 결정 3과 같이
  `ApplyExternalSnapshot` 하나로 반영합니다. 문서 밖 의미(컴포넌트·token·catalog)는 history 대상이 아니라 그대로
  바꿉니다. 감시는 `FileWatchEvent::affects_text_source`로 `design/` 아래 원본 event만 고릅니다.
- 문법 진단은 실패 경로에서만 같은 제한 YAML loader(`load_restricted`)로 파일을 다시 읽어 줄·열을 찾습니다. 새
  parser를 만들지 않습니다. 의미 오류는 메시지가 이름을 댄 파일, 없으면 바뀐 파일이 하나일 때 그 파일을 경로로 씁니다.
- 앱 기본 열기·저장은 아직 chunk generation입니다. 기본 원본 전환은 V.10.7(다음 배치)입니다.

### 10. 편집 열기와 headless 읽기가 migration 판정을 거친다 (IX.11.21 연결)

- `ProjectWindowSession::load`·다시 읽기는 `migration::decide`가 `Current`일 때만 편집 session을 엽니다. 높은 schema·
  경로 없는 schema·메모리 migration 결과(읽기 전용)는 `WindowSessionLoadError::Schema`입니다. 낮은 schema의 V.8.4
  디스크 migration(ADR-0077 결정 4 정정본)은 제품 step 표가 비어 있어 지금 도달하지 않으며, step이 생길 때 연결합니다.
- `ops::snapshot::open_snapshot`은 낮은 schema를 메모리에서만 migration해 읽고, 높은 schema는 상태 보고(`stellar ci`·
  `stellar migrate` 판정)용으로 열되 문서 읽기를 `DocumentReadError::Schema`로 거부합니다. 새 schema bytes를 옛
  decoder로 읽지 않습니다.

## Alternatives

- **stale 결과에 merge 선택지를 둠**: §22.2가 금지합니다. 탈락.
- **UseCurrentWindow가 base 검사 없이 덮어씀**: 최신을 읽은 뒤의 다른 저장을 잃습니다. 일반 저장 경로의 잠금 안
  base 검사를 그대로 씁니다.
- **UseLatestDisk가 먼저 history를 버리고 읽음**: 읽기 실패 때 편집을 잃습니다. 탈락.
- **writer 출처를 generation manifest에 기록**: 보호 경로 storage 포맷 변경이라 소유 lane·ADR이 따로 필요합니다.
  이번에는 프로세스 기록으로 같은 두 갈래 판정을 내고, 세부 구분은 XXIV.4.6으로 미룹니다.
- **clean 창의 외부 변경을 history 폐기 reload로 반영(XIII.3.1 project 경로)**: Undo로 되돌릴 수 없어 UX.4.1을
  만족하지 않습니다. runtime 경로는 `ApplyExternalSnapshot`을 씁니다.
- **감시 thread가 자기 저장 기록으로 event를 거름**: 앱 창마다 기록을 공유해야 하고 판정이 둘로 나뉩니다. 창의
  hash 비교 하나로 판정합니다(job 하나가 더 생길 뿐).
- **flush 시간 초과 때 job 취소**: 편집이 저장되었는지 알 수 없게 되고 사용자 편집을 버릴 위험이 있습니다. 취소하지
  않고 결정 상태만 돌려줍니다.

## Consequences

- 두 창 stale은 코어에서 두 선택 모두 실행할 수 있습니다. 셸 시트(UX.4.3)는 `take_stale_alerts`·`resolve_stale`만
  부르면 됩니다(이번에는 만들지 않음).
- clean 창은 다른 창·CLI·MCP·Git의 저장을 Undo 가능한 항목 하나로 받습니다. 앱 시험은 두 창 stale을 만들 때 두 창을
  모두 먼저 편집해야 합니다(기존 시험을 그렇게 고침).
- 외부 writer와 dirty가 겹치면 XIII.3.4 전까지 자동 저장이 멈춘 채입니다. 사용자는 창을 닫을 때 D6 시트로 결정합니다.
- 텍스트 원본은 엔진 경로로 저장·재열기·외부 변경·복구가 됩니다. 앱 기본 경로 전환은 V.10.7입니다.

## Compatibility

- 파일 포맷·generation manifest·journal 형식·C ABI·FFI 헤더를 바꾸지 않습니다. 보호 경로 변경이 없습니다.
- 공개 Rust API 추가: `StaleSaveChoice`·`WriterProvenance`·`StaleResolution`, `autosave::{flush, trace}`,
  `external_change::source`, runtime `ProjectWindowSession::{stale_state, resolve_stale,
  reload_latest_disk, sync_external}`, `AppRuntime::{take_stale_alerts, resolve_stale, sync_external_changes,
  external_status, autosave_traces, flush_for_close, flush_for_deactivate, git_ui_preflight, prepare_git_ui}`,
  `RuntimeThreads`·`ShutdownStage`.
- 오류 변형 추가: `WindowSessionLoadError::Schema`, `WindowSessionSaveError::Rebase`, ops `OpenError::Migration`,
  `DocumentReadError::Schema`(CLI는 기존 `document` 진단으로 보고). `stellar ci` 결과 golden의 새 schema 문서 진단이
  chunk decode 오류에서 schema 거부 문장으로 바뀝니다.

## Performance

- stale 판정·외부 변경 판정은 current pointer 한 번 읽기와 hash 비교입니다. 반영은 새 generation 문서 로드와 레코드
  변경 집합 계산으로 문서 크기에 비례하며 document thread에서 실행합니다(대형 문서 반영 시간은 아직 측정하지 않음).
- 감시 thread는 50 ms마다 쌓인 FSEvents를 꺼내고 조용한 구간 30 ms로 묶습니다. 감시기가 없을 때만 50 ms마다 pointer
  파일을 읽습니다.
- 공유 snapshot으로 같은 revision의 자동 저장·복구 직렬화가 한 번으로 줄어듭니다.

## Security

- trace·flush 실패 code에는 경로·이름·문서 내용이 없습니다. 텍스트 원본 진단은 `design/` 기준 상대 경로만 씁니다.
- 비신뢰 프로젝트는 외부 symlink target을 감시하지 않습니다(`ExternalTargetSync`·`PathResolver` 규칙 그대로).
- 텍스트 원본 저장은 journal의 경로 규칙(`..`·절대 경로·`.git`·`.stellar/` 거부)과 symlink 거부를 그대로 따릅니다.

## Migration

기존 데이터 migration은 없습니다. 앱 창 저장 기록은 프로세스 메모리라 남는 파일이 없습니다.
