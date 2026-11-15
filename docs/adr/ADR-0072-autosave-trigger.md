# ADR-0072: 지연 자동 저장 트리거, worker 쓰기와 앱 비활성화 flush

## Status

Accepted (2026-11-15, UX.2.11 구현 기록. 같은 날 검토 지적을 반영해 worker 쓰기·상태 노출·상태 변경 신호를 더함)

## Context

기술 아키텍처 22.1은 완료된 편집을 마지막 입력 뒤 짧은 debounce 후 **비동기로** 저장하고, 연속 drag·scrub 중에는
디스크 쓰기를 반복하지 않으며, `Save Now`·창 종료·앱 비활성화 때 flush하도록 정합니다. 22.2는 자동 저장에서 다른 창의
저장(stale)을 발견하면 자동 저장을 멈추도록 정합니다. 구현 계획 XIII.1.1은 그 수치를 `AutosavePolicy`로
`config/performance-budgets.yaml`에서 읽게 했지만 저장소 config에 `autosave` 절이 없었고, runtime에는 Save Now와 창 닫기
저장만 있었습니다. 그래서 편집은 사용자가 ⌘S를 누르거나 창을 닫을 때만 저장됐습니다.

UX 명세 흐름 2는 "저장은 자동입니다"를 전제로 하고, 사용자 결정 D6은 저장 실패·stale 상태에서 창을 닫을 때 선택
시트를 띄우도록 정합니다. 첫 구현은 직렬화와 쓰기를 document thread에서 동기로 했고(XIII.1.4 생략), 실패·stale과 저장
완료를 셸에 알리지 않았습니다. 검토는 이것이 10 ms 입력 계약과 22.1의 "비동기 저장"을 어기고, 사용자가 저장되지 않는
편집을 모른 채 계속하게 하며, 동시에 진행 중인 ADR-0073(부제목·배지·윤곽을 "자동 저장 flush 뒤 worker 결과로만 갱신")과
맞지 않는다고 지적했습니다. 이 ADR은 UX.2.11에서 바꾼 ADR 보호 경로(`config/performance-budgets.yaml`, `crates/ffi/`)와 그
결정을 기록합니다. 기존 함수·event의 의미는 바꾸지 않습니다.

## Decision

### 1. 잠정 수치(`config/performance-budgets.yaml`의 `autosave` 절)

| 키 | 값 | 쓰임 |
|---|---|---|
| `debounce_ms` | 1000 | 마지막 완료 편집 뒤 저장까지 기다림 |
| `max_delay_ms` | 10000 | 완료 편집이 이어져도 저장되지 않은 첫 편집에서 이 시간이 지나면 저장. 실패 뒤 다시 시도까지의 시간 |
| `app_deactivate_flush_timeout_ms` | 2000 | 아직 쓰지 않음(XIII.1.6). 비활성화 flush는 셸을 기다리게 하지 않음 |
| `window_close_flush_timeout_ms` | 10000 | Save Now(창 닫기 flush)가 진행 중인 자동 저장 쓰기를 기다리는 상한. 넘으면 저장 실패 |

수치는 사용성·SSD 쓰기·복구 손실 범위 벤치마크 전의 잠정값입니다. 1초는 편집을 멈춘 뒤 터미널에서 `git status`를
볼 때 이미 저장된 시간이고, 10초는 비정상 종료 때 잃는 범위의 상한입니다. reader는 기존대로 누락·범위 밖 값을
거부하고 기본값을 만들지 않습니다. 앱 runtime은 빌드할 때 이 파일을 포함해 읽습니다. macOS 셸의 닫기 응답
watchdog(`closeReplyTimeout`, 30초, ADR-0028)은 셸 값이고 이 파일을 읽지 않습니다. 30초는 §2의 settle 상한
`window_close_flush_timeout_ms`(10초)와 쓰기 시간보다 길게 잡은 값입니다(9f1f8efc). 두 값을 하나로 합치는 일은 XIII.1.6입니다.

### 2. 시점 계산은 코어, 시계는 runtime

- `stellar-project`의 `autosave::schedule::AutosaveSchedule`이 창 하나의 시점을 정하는 순수 상태 기계입니다.
  관찰한 문서 revision이 바뀔 때만 debounce를 다시 시작하므로 선택·viewport 변경은 저장을 미루지 않습니다.
  history preview(drag·scrub 제스처)가 진행 중이면 기한 없이 기다립니다. 쓰기가 진행 중이면 그 결과가 올 때까지
  기다리고, 캡처한 revision 뒤의 편집은 새 대기로 셉니다.
- 저장 대상 판단은 창 session의 `has_unsaved_changes`(작업 문서 revision이 마지막 저장 revision과 다름)입니다.
  XIII.1.2의 `DirtyTrackedHistory`는 앱 runtime에서 쓰지 않습니다. preview 갱신도 revision을 바꾸므로 dirty로 세지만
  preview가 진행 중이면 쓰지 않습니다.
- `stellar-runtime`은 상태 snapshot을 게시할 때마다(`build_state`, 창 요약을 만들기 전) 모든 창을 관찰해 가장 이른
  기한을 시계 thread(`stellar-autosave`)에 알립니다. 시계 thread는 그 시각에 자동 저장 job을 background lane에 고정
  병합 key로 보냅니다. 입력·편집 command보다 늦게 처리되고, 밀린 job은 하나로 합쳐집니다.

### 3. 캡처는 document thread, 직렬화·쓰기는 writer thread (XIII.1.3·XIII.1.4)

- 기한이 지난 창은 document thread에서 `ProjectWindowSession::capture_save`로 문서 레코드(`DocumentModel::to_records`),
  기준 generation, 작업 문서가 차지한 chunk 집합, 이미지 asset record(Arc)만 복사합니다.
- 직렬화(`serialize_records`)·SavePlan·공용 저장 잠금·base generation 검사·chunk 쓰기·current pointer 원자적 교체는
  writer thread(`stellar-autosave-write`) 하나가 받은 순서대로 합니다(`SaveCapture::write`). 잠금 안에서 current pointer가
  캡처한 기준 generation과 다르면 stale이고 아무것도 쓰지 않으며 current pointer를 바꾸지 않습니다.
- writer는 결과를 창별 결과 자리에 두고 결과 반영 job을 command lane에 보냅니다(병합 key 없음, 밀려나지 않음).
  document thread는 `apply_save`로 loaded generation·소유 chunk·저장 revision을 바꾸고 상태를 게시합니다. 캡처 뒤 그
  창의 기준 generation이나 작업 문서가 바뀌었으면 반영하지 않습니다.
- Save Now는 같은 `capture_save → write → apply_save`를 document thread에서 차례로 실행합니다(저장 경로 하나).
- 창마다 진행 중인 쓰기는 하나입니다. writer가 처리하지 못하고 버린 쓰기(종료, panic)는 결과 자리에 중단을 남겨
  실패로 처리합니다. 종료할 때는 시계 → document thread → writer 순서로 멈추고, document thread가 제때 끝났을 때만 진행
  중인 쓰기를 `SHUTDOWN_TIMEOUT`까지 기다립니다.

### 4. Save Now 합치기 (XIII.1.5)

Save Now(⌘S, 창 닫기 flush)는 먼저 같은 프로젝트에서 진행 중인 자동 저장 쓰기를 기다려 반영하고(`settle`), 그 뒤 남은
편집만 저장합니다. 다른 프로젝트의 쓰기는 잠금·generation이 따로라 기다리지 않습니다(처음 구현은 모든 프로젝트를 기다려,
느린 다른 프로젝트 때문에 이 창의 Save Now가 늦거나 실패할 수 있었습니다). 셸의 닫기 응답 watchdog은 이 상한과 쓰기 시간보다
긴 30초입니다. 그래서 같은 revision을 두 번 쓰지 않고, 같은 프로세스 안에서 저장 잠금(`flock` 즉시 실패)을 다투지 않으며,
자동 저장이 막 쓴 generation을 stale로 오인하지 않습니다. 기다리는 상한은 `window_close_flush_timeout_ms`이고, 넘으면
`STELLAR_FAILURE_SAVE_FAILED`입니다(편집은 메모리에 남음). Save Now가 먼저 저장해 clean이 되면 대기가 비고, 이미 접수한
자동 저장 job은 clean인 창을 쓰지 않습니다.

### 5. flush

- 앱 비활성화: C 함수 `StellarStatus stellar_runtime_flush_autosave(uint64_t handle)`. 저장되지 않은 완료 편집을
  debounce 없이 캡처하도록 command lane에 접수하고 기다리지 않습니다. 완료 event가 없습니다. 앱 runtime이 아직 없으면
  `STELLAR_OK`, 대기열이 차면 `STELLAR_QUEUE_FULL`(그 편집은 다음 자동 저장이 씀)입니다. 제스처 중이거나 stale인 창은
  쓰지 않습니다. 쓰기가 진행 중인 창이 있거나 이미지 가져오기 중이라 다 하지 못한 flush는 그 일이 끝난 뒤 flush로 다시
  실행합니다(일반 debounce로 바뀌지 않음). macOS 셸은 `applicationDidResignActive`에서 부릅니다.
- 창 닫기·앱 종료: 기존처럼 셸이 Save Now로 flush합니다(4절).

### 6. 실패와 stale

- 쓰기 실패(잠금 경합, 디스크, 직렬화): 실패로 표시하고 **실패 시각 + `max_delay_ms`에 시간 기반으로 다시 시도**합니다.
  그 사이 완료 편집이 오면 그 편집의 debounce 뒤입니다(최대 지연 상한은 실패 시각 기준). 앱 비활성화 flush는 바로 다시
  시도합니다. 실패 표시는 저장에 성공하거나 clean이 될 때까지 남습니다. 일시적 실패(`stellar apply`가 잠금을 잡은 동안
  등)는 다음 편집 없이도 풀리고, 지속 실패는 10초마다 한 번만 씁니다.
- stale: clean이 될 때까지(다시 불러오기, 22.2 두 선택지의 결과) 자동 저장을 멈추고 디스크를 바꾸지 않습니다.
- 편집은 메모리에 남습니다. 창을 닫을 때의 Save Now가 같은 실패·stale을 셸에 돌려주고, 셸은 그 응답에서 D6 선택
  시트("다시 시도" / "저장하지 않고 닫기" / "취소")로 갑니다. D6 시트(B7)와 자동 저장 stale의 두 선택지 sheet(UX.4.3)가
  들어오기 전에는 지금의 확인 alert가 창을 닫지 못하게 하므로, 통합 순서는 B7과 UX.4.3을 UX.2.11보다 먼저 또는 함께
  넣습니다.

### 7. 셸 노출과 상태 변경 신호

- 창 요약 `WindowSessionSummary::autosave`(`AutosaveStatus`: Idle·Pending·Saving·Failed·Stale)와
  `StellarWindowSessionInfo` version 3의 `autosave`(`STELLAR_AUTOSAVE_IDLE` 0, `_PENDING` 1, `_SAVING` 2, `_FAILED` 3,
  `_STALE` 4)로 노출합니다. 상태가 겹치면 stale → 실패 → 쓰는 중 → 대기 순으로 앞선 것을 씁니다. UX.5.3("저장 안 됨"
  부제목)과 UX.4.3은 이 값만 읽으면 됩니다.
- 자동 저장은 요청이 아니므로 완료 event를 만들지 않습니다(outbox 요청 상한 회계를 바꾸지 않음). 대신 **상태 변경 신호**를
  이렇게 정합니다: 코어는 상태를 게시할 때마다 상태 세대(`StellarRuntimeState.generation`)를 올리고, 셸의 event 펌프
  (`ShellRuntimeEventPump.observeState`)는 관찰자 하나라도 기다리는 상태(그 창 `autosave`가 `PENDING`·`SAVING`·`FAILED`)를
  보고하는 동안만 250 ms마다 상태 세대를 읽어, 바뀌면 관찰자를 부릅니다. 기다리는 상태가 없으면 timer를 끕니다. 요청
  완료 뒤마다 다시 판단하므로 편집 뒤 대기가 생기면 관찰이 켜집니다. callback은 쓰지 않습니다(ADR-0009).
- 관찰자는 창 info의 loaded `generation`이 바뀌면 자동 저장이 디스크에 썼다고 보고 Git semantic status를 worker에 다시
  요청합니다. ADR-0073의 "부제목·배지·캔버스 윤곽은 자동 저장 flush 뒤 worker에서 계산한 semantic status 결과로만 갱신"은
  이 신호를 뜻합니다(UX.5.3·UX.6.2·UX.6.3·UX.6.4가 같은 관찰자를 씀).
- 지금 연결한 관찰자는 프로젝트 창 하나뿐입니다(`ShellAutosaveStatus.swift`). 창 session을 열면 등록하고 닫거나 다시 열면
  해제하며, 끝난 결과만 부제목에 반영합니다: `FAILED`이면 앞에 경고 글리프와 "Not Saved", `STALE`이면 "Not Saved — Changed
  Elsewhere"(UX 명세 3.3 "저장 실패"). 곧 끝나는 대기·쓰는 중 상태는 표시하지 않으므로 편집할 때마다 부제목이 흔들리지
  않습니다. Git 패널 data source는 아직 이 신호를 읽지 않습니다(UX.5.2에서 사이드바에 붙을 때 같은 관찰자를 씀). 이전
  판의 "Git 패널 data source가 이 신호로 배너를 고친다"는 문장은 통합 결과와 달라 고쳤습니다.
- **남은 일(UX.4.3)**: stale이면 자동 저장이 clean이 될 때까지 멈추고 그 뒤 편집은 메모리에만 남습니다. 지금은 부제목으로만
  알리고, 창을 닫을 때 D6 sheet(다시 시도·저장하지 않고 닫기·취소)가 뜹니다. 명세 흐름 3의 두 선택지 sheet("이 창의 버전으로
  저장" / "디스크 버전 불러오기")는 runtime에 창 문서를 디스크 generation으로 다시 불러오는 경로와 새 base 위에 이 창 버전을
  쓰는 경로(XII.7.5·XII.7.6)가 없어 아직 띄우지 않습니다. CLI·MCP가 같은 파일을 쓰는 흐름에서는 약 1초 안에 stale이 될 수
  있으므로 UX.4.3이 이 두 경로를 먼저 둡니다.

### 8. 이미지 가져오기와 서로 배제

가져오기는 시작할 때의 loaded generation에 묶여 있으므로(`image_place`) 둘을 SeqCst 표시 두 개로 서로 배제합니다. 자동
저장은 쓰기 표시(`AppShared::autosave_writing`)를 먼저 세우고 가져오기 표시를 확인해, 가져오기 중이면 내리고 미룹니다.
가져오기는 가져오기 표시를 먼저 세우고 쓰기 표시가 내려갈 때까지(상한 10초) 기다린 뒤 상태를 읽습니다. 쓰기 표시는 결과를
반영해 상태를 게시한 뒤 내립니다. 그래서 진행 중인 자동 저장 뒤에 시작한 배치도 새 generation 위에서 확인합니다.

### 9. 제스처 판정의 한계

판정은 history preview만 봅니다. 지금 셸의 drag(이동·생성)와 텍스트 조합은 runtime preview를 쓰지 않고 끝날 때
transaction 하나만 보내므로, drag·조합 중에 그 전 편집의 자동 저장이 한 번 끼어들 수 있습니다. 끼어든 캡처는 document
thread에서 짧고(아래 측정) 쓰기는 writer가 하므로 drag 중 쓰기 반복은 없습니다. 셸 제스처를 runtime 제스처로 옮기는
UX.2.2(scrub)·UX.2.9·UX.3.7이 통합되면 같은 판정이 그 제스처에도 적용되고, 그때 scrub이 debounce를 넘겨 이어져도 쓰지
않는지 통합 테스트를 더합니다.

### 10. Debug 검사 실행

Debug native 검사(`--…-check`)는 dirty·stale 상태와 디스크 bytes를 시간에 걸쳐 단정합니다. 그래서 검사 실행의
`RuntimeHost`는 코어 runtime을 만들기 전에 검사 전용 환경 변수 `STELLAR_CHECK_DISABLE_AUTOSAVE=1`을 두고(Swift
`#if DEBUG` 안), Debug 코어는 앱 runtime을 시작할 때 이 값을 보고 자동 저장을 켜지 않습니다. Release 코어는 변수를 읽지
않습니다. 일반 앱 실행은 늘 자동 저장을 켭니다.

처음에는 Debug 전용 C 함수(`stellar_runtime_check_disable_autosave`, 헤더의 `DEBUG` 블록)로 두었지만, 공개 헤더가 구성마다
달라지고 Debug 구성에 `DEBUG=1`을 되살려야 해서 ADR-0028의 결정("Debug 전용 창 편집 검사 ABI 삭제")과 어긋났습니다. 헤더를
바꾸지 않는 환경 변수로 바꿨습니다.

## Alternatives

- **셸 timer가 기한을 재고 Save Now를 부름**: 저장 시점 판단이 셸로 나뉘고, Save Now 실패 alert 경로를 자동 저장이
  타게 됩니다. 탈락.
- **자동 저장 결과를 outbox event로 보냄**: 요청이 아닌 event가 outbox 요청 상한 회계를 흐리고, 셸 펌프는 기다리는
  요청이 없으면 event를 꺼내지 않습니다. 상태 필드 + 상태 세대 관찰(7절)로 대신합니다. 탈락.
- **document thread에서 동기 쓰기 유지(첫 구현)**: 편집을 멈출 때마다 document thread를 저장 시간만큼 점유해 10 ms 입력
  계약을 넘습니다. 탈락.
- **Save Now도 writer로 보냄**: ⌘S의 document thread 점유도 없어지지만 Save Now 완료 event를 writer 결과에서 내야 하고 창
  닫기 순서가 바뀝니다. 이번에는 Save Now를 동기로 두고(전과 같은 점유) 후속으로 둡니다.
- **writer를 창마다 둠**: 같은 프로젝트 창 둘이 동시에 쓰면 프로세스 안에서 저장 잠금을 다퉈 한쪽이 실패합니다. 탈락.

## Consequences

- 앱에서 편집하면 멈춘 뒤 약 1초 안에 저장되고, 다른 앱으로 전환하면 바로 저장됩니다. ⌘S는 그대로 동작합니다.
- 자동 저장이 document thread를 점유하는 시간은 캡처뿐입니다(아래 측정). 직렬화·쓰기 동안의 입력·편집은 기다리지 않고,
  그 편집은 다음 대기가 됩니다.
- 자동 저장 실패·stale은 창 info와 상태 세대로 셸에 보입니다. 표시(부제목, D6 시트)는 UX.5.3·B7·UX.4.3이 붙입니다.
- 복구 프롬프트의 "준비되면 저장하세요" 문구는 복원한 문서가 약 1초 뒤 자동으로 저장되는 동작과 맞지 않습니다. 의도인지
  사용자 확인이 필요하고, 문구는 해당 S 행에서 고칩니다.

## Compatibility

- C ABI(ADR-0014·ADR-0028에 항목 추가):
  - 함수 `stellar_runtime_flush_autosave`를 덧붙입니다.
  - `StellarWindowSessionInfo`를 version 3으로 올립니다. 끝에 `autosave`(u32)와 `reserved`(u32, 0)를 더해 88 → 96 byte가
    됩니다. 앞 필드의 offset은 그대로이고, 셸은 기존대로 `version == STELLAR_WINDOW_SESSION_INFO_VERSION`을 확인합니다.
  - 상수 `STELLAR_AUTOSAVE_IDLE`·`_PENDING`·`_SAVING`·`_FAILED`·`_STALE`을 더합니다.
  - 검사 실행의 자동 저장 끄기는 C ABI가 아니라 Debug 코어만 읽는 환경 변수입니다(10절). 헤더는 구성마다 같습니다.
  - 헤더는 `cargo xtask generate-ffi-header`로 생성합니다. 기존 함수·상태 코드·event는 그대로입니다.
- `config/performance-budgets.yaml`: 최상위 절 `autosave`. 다른 절의 reader는 바뀌지 않습니다.
- 저장 파일 형식은 바뀌지 않습니다(Save Now와 같은 저장 경로).

## Performance

Release, Apple M3 Max, macOS 27.2, 로컬 SSD 임시 폴더(REF-MAC 판정 아님). 한 View root 아래 Frame N개를 저장한 프로젝트에서
이름 하나를 바꾼 뒤 캡처·쓰기·반영을 7번 잰 값입니다. 재현:
`cargo test -p stellar-runtime --release --lib app::autosave::tests::measure_document_thread_occupancy -- --ignored --nocapture`

| Frame 수 | 캡처(document thread) | 쓰기(writer, 직렬화 포함) | 반영(document thread) |
|---|---|---|---|
| 2,000 | 0.46–0.62 ms | 37–42 ms | 0.001 ms 미만 |
| 20,000 | 5.6–7.1 ms | 376–384 ms | 0.001 ms 미만 |

- 자동 저장 하나가 document thread에 더하는 일은 캡처 한 번과 상태 게시 두 번(쓰기 시작, 결과 반영)입니다. 상태 게시는
  편집 요청마다 하는 게시와 같은 비용이고, 결과 반영 뒤 revision이 같으므로 캔버스 scene은 캐시를 씁니다.
- 20,000 Frame 캡처는 10 ms 계약 안이지만 크기에 비례합니다. 복구 체크포인트도 dirty revision마다 같은 `to_records`를
  하므로 두 캡처를 하나로 합치는 일(같은 revision의 레코드 공유)을 후속으로 둡니다.
- 첫 구현 표의 값(document thread 동기 `save_now`, 2,000개 29–95 ms, 20,000개 77–112 ms)은 커밋하지 않은 다른 fixture로 잰
  값이라 위 표와 직접 비교하지 않습니다. 위 표는 커밋한 측정으로 다시 잴 수 있습니다.
- 관찰 비용은 상태 게시마다 창 수만큼의 O(1) 비교이고, 셸 상태 관찰은 기다리는 창이 있을 때만 250 ms마다 상태 snapshot을
  한 번 읽습니다(document thread를 기다리지 않음).

## Security

- 자동 저장은 Save Now와 같은 저장 잠금·경로 규칙을 쓰고 새 파일 위치를 만들지 않습니다.
- 실패는 stderr에 Save Now 실패와 같은 오류 문장으로만 남깁니다.
- 신뢰 여부와 상관없이 저장합니다(사용자 결정 D1: 편집과 저장은 제한 모드에서도 허용).
- Debug 검사 전환은 Release 심벌이 없습니다.

## Migration

없음. 기존 프로젝트는 그대로 열리고, 저장 시점만 자동으로 바뀝니다. 셸은 새 헤더로 다시 빌드합니다.

## 통합 기록

- R lane 행이지만 셸 연결을 위해 S 파일(`AppDelegate.swift`, `RuntimeHost.swift`, `UI/Shell/ShellRuntimeEventPump.swift`,
  `UI/Shell/ShellGitDataSource.swift`)과 C 파일(`crates/ffi/src/autosave.rs`, `window_session.rs`, `lib.rs`의 모듈 선언·
  `Runtime` 필드·헤더 검사)을 고쳤습니다.
- 공유 파일에서 다른 lane과 같은 줄을 피하도록 `crates/runtime/src/app.rs`의 `mod autosave;`는 공개 use 묶음 뒤,
  `WindowEntry` 필드와 초기화는 구조체 끝, `crates/ffi/src/lib.rs`의 `mod autosave;`는 모듈 목록 뒤 별도 묶음에 둡니다.
- 남은 일: XIII.1.6(앱 비활성화 flush 제한 시간과 셸 결정 상태, 셸 watchdog과 값 합치기), XIII.1.7(autosave trace),
  캡처 공유(Performance), 복구 프롬프트 문구 확인, 구현 계획 XIII.1.x 상태 갱신(D lane).
