# ADR-0072: 지연 자동 저장 트리거와 앱 비활성화 flush

## Status

Accepted (2026-11-15, UX.2.11 구현 기록)

## Context

기술 아키텍처 22.1은 완료된 편집을 마지막 입력 뒤 짧은 debounce 후 저장하고, 연속 drag·scrub 중에는 디스크
쓰기를 반복하지 않으며, `Save Now`·창 종료·앱 비활성화 때 flush하도록 정합니다. 22.2는 자동 저장에서 다른 창의
저장(stale)을 발견하면 자동 저장을 멈추도록 정합니다. 구현 계획 XIII.1.1은 그 수치를 `AutosavePolicy`로
`config/performance-budgets.yaml`에서 읽게 했지만 저장소 config에 `autosave` 절이 없었고, runtime에는 Save
Now와 창 닫기 저장만 있었습니다. 그래서 편집은 사용자가 ⌘S를 누르거나 창을 닫을 때만 저장됐습니다.

UX 명세 흐름 2는 "저장은 자동입니다"를 전제로 하고, 사용자 결정 D6은 저장 실패·stale 상태에서 창을 닫을 때 선택
시트를 띄우도록 정합니다. 이 ADR은 UX.2.11에서 바꾼 ADR 보호 경로(`config/performance-budgets.yaml`,
`crates/ffi/`)를 기록합니다. 기존 함수·event·구조체의 의미는 바꾸지 않습니다.

## Decision

### 1. 잠정 수치(`config/performance-budgets.yaml`의 `autosave` 절)

| 키 | 값 | 쓰임 |
|---|---|---|
| `debounce_ms` | 1000 | 마지막 완료 편집 뒤 저장까지 기다림 |
| `max_delay_ms` | 10000 | 완료 편집이 이어져도 저장되지 않은 첫 편집에서 이 시간이 지나면 저장 |
| `app_deactivate_flush_timeout_ms` | 2000 | 아직 대기 상한으로 쓰지 않음(비활성화 flush는 셸을 기다리게 하지 않음, XIII.1.6) |
| `window_close_flush_timeout_ms` | 10000 | macOS 셸 닫기 응답 watchdog(`closeReplyTimeout`)과 같은 값 |

수치는 사용성·SSD 쓰기·복구 손실 범위 벤치마크 전의 잠정값입니다. 1초는 편집을 멈춘 뒤 터미널에서 `git status`를
볼 때 이미 저장된 시간이고, 10초는 비정상 종료 때 잃는 범위의 상한입니다. reader는 기존대로 누락·범위 밖 값을
거부하고 기본값을 만들지 않습니다. 앱 runtime은 빌드할 때 이 파일을 포함해 읽습니다.

### 2. 시점 계산은 코어, 시계는 runtime

- `stellar-project`의 `autosave::schedule::AutosaveSchedule`이 창 하나의 시점을 정하는 순수 상태 기계입니다.
  관찰한 문서 revision이 바뀔 때만 debounce를 다시 시작하므로 선택·viewport 변경은 저장을 미루지 않습니다.
  history preview(drag·scrub 제스처)가 진행 중이면 기한 없이 기다립니다.
- `stellar-runtime`은 상태 snapshot을 게시할 때마다(`build_state`) 모든 창을 관찰해 가장 이른 기한을 시계
  thread(`stellar-autosave`)에 알립니다. 시계 thread는 그 시각에 자동 저장 job을 background lane에 고정 병합
  key로 보냅니다. 입력·편집 command보다 늦게 처리되고, 밀린 job은 하나로 합쳐집니다.
- 저장은 Save Now와 같은 `ProjectWindowSession::save_now`(공용 저장 잠금, base generation 검사, 원자적 교체)를
  씁니다.
- 자동 저장은 요청이 아니므로 완료 event를 만들지 않고 상태 snapshot만 게시합니다(`has_unsaved_changes`, loaded
  generation). outbox 요청 상한 회계를 바꾸지 않습니다.
- 이미지 가져오기 중에는 미룹니다. 가져오기가 시작 시점의 loaded generation을 확인하므로 그 사이 저장하면 배치가
  실패하기 때문입니다.

### 3. Save Now 합치기

Save Now(⌘S, 창 닫기 flush)가 먼저 저장하면 게시된 상태가 clean이라 대기가 비고, 이미 접수한 자동 저장 job은
clean인 창을 쓰지 않습니다. 둘 다 document thread에서 차례로 실행되므로 같은 revision을 두 번 쓰지 않습니다.

### 4. flush

- 앱 비활성화: 새 C 함수 `StellarStatus stellar_runtime_flush_autosave(uint64_t handle)`. 저장되지 않은 완료
  편집을 debounce 없이 저장하도록 command lane에 접수하고 기다리지 않습니다. 완료 event가 없습니다. 앱 runtime이
  아직 없으면 `STELLAR_OK`, 대기열이 차면 `STELLAR_QUEUE_FULL`(그 편집은 다음 자동 저장이 씀)입니다. 제스처 중이거나
  stale인 창은 쓰지 않습니다. macOS 셸은 `applicationDidResignActive`에서 부릅니다.
- 창 닫기·앱 종료: 기존처럼 셸이 Save Now로 flush합니다.

### 5. 실패와 stale

- 쓰기 실패: 그 창의 자동 저장을 멈추고 다음 완료 편집부터 센 debounce 뒤 다시 시도합니다. 같은 실패를 계속 쓰지
  않습니다. 앱 비활성화 flush는 다시 시도합니다.
- stale: clean이 될 때까지(다시 불러오기, 이후의 두 선택지 결과) 자동 저장을 멈추고 디스크를 바꾸지 않습니다.
- 편집은 메모리에 남습니다. 창을 닫을 때의 Save Now가 같은 실패·stale을 셸에 돌려주고, 셸은 그 응답에서 D6 선택
  시트("다시 시도" / "저장하지 않고 닫기" / "취소")로 갑니다. 자동 저장 상태를 셸에 따로 노출하는 일(부제목
  "저장 안 됨", 자동 저장 중 stale의 두 선택지 sheet)은 UX.5.3·UX.4.3에서 합니다.

## Alternatives

- **셸 timer가 기한을 재고 Save Now를 부름**: 저장 시점 판단이 셸로 나뉘고, Save Now 실패 alert 경로를 자동 저장이
  타게 됩니다. 탈락.
- **자동 저장 결과를 outbox event로 보냄**: 요청이 아닌 event가 outbox 요청 상한 회계를 흐리고, 지금 셸 펌프는
  기다리는 요청이 없으면 event를 꺼내지 않습니다. 셸이 결과를 쓸 때(UX.5.3) 상태 snapshot 필드로 노출합니다. 탈락.
- **직렬화·쓰기를 지금 worker로 옮김**(XIII.1.4): 진행 중인 쓰기와 Save Now의 순서, 결과를 되돌려 받는 stale 판정을
  같이 바꿔야 해서 이번 행 범위를 넘습니다. 측정값을 남기고 후속으로 둡니다.

## Consequences

- 앱에서 편집하면 멈춘 뒤 약 1초 안에 저장되고, 다른 앱으로 전환하면 바로 저장됩니다. ⌘S는 그대로 동작합니다.
- 지금 셸의 drag는 놓을 때 transaction 하나만 보내므로 drag 중에는 완료 편집이 없고, 코어 preview 제스처도 쓰지
  않습니다. 연속 drag 중 쓰기 반복은 0입니다.
- 자동 저장이 document thread를 점유하는 동안 들어온 입력·편집은 기다립니다(아래 측정). 편집을 멈춘 뒤 debounce마다
  한 번, 편집이 이어지면 최대 지연마다 한 번입니다.
- 자동 저장 실패·stale은 아직 화면에 표시되지 않습니다. 창을 닫을 때 처음 알게 됩니다.

## Compatibility

- C ABI: 함수 `stellar_runtime_flush_autosave` 하나를 덧붙입니다. 기존 함수·상태 코드·event·구조체는 그대로입니다.
  헤더는 `cargo xtask generate-ffi-header`로 생성합니다. ADR-0014·ADR-0028의 셸 경계 규칙(정수 handle,
  `StellarStatus`, 기다리지 않음)을 따릅니다.
- `config/performance-budgets.yaml`: 새 최상위 절 `autosave`. 다른 절의 reader는 바뀌지 않습니다.
- 저장 파일 형식은 바뀌지 않습니다(Save Now와 같은 저장 경로).

## Performance

document thread 점유 측정(Release, Apple M3 Max, macOS 27.2, 로컬 SSD 임시 폴더, REF-MAC 판정 아님). 한 View 아래
Frame N개를 저장한 프로젝트에서 이름 하나를 바꾼 뒤 `save_now`를 document thread에서 7번 잰 값입니다.

| Frame 수 | `save_now` | 그중 직렬화 | 저장 뒤 상태 게시(`build_state`) |
|---|---|---|---|
| 2,000 | 29–33 ms(1회차), 85–95 ms(2회차) | 약 2.2 ms | 약 0.5 ms |
| 20,000 | 77–84 ms(1회차), 83–112 ms(2회차) | 26–39 ms | 5–11 ms |

- 대부분은 쓰기·commit I/O이고 회차마다 크게 흔들립니다. 입력에서 Present 예약까지 10 ms 계약(아키텍처 6장)을
  자동 저장과 겹친 입력 하나에서 넘깁니다. 해소는 직렬화·쓰기를 worker로 옮기는 XIII.1.4입니다.
- 관찰 비용은 상태 게시마다 창 수만큼의 O(1) 비교입니다.

## Security

- 자동 저장은 Save Now와 같은 저장 잠금·경로 규칙을 쓰고 새 파일 위치를 만들지 않습니다.
- 실패는 stderr에 Save Now 실패와 같은 오류 문장으로만 남깁니다.
- 신뢰 여부와 상관없이 저장합니다(사용자 결정 D1: 편집과 저장은 제한 모드에서도 허용).

## Migration

없음. 기존 프로젝트는 그대로 열리고, 저장 시점만 자동으로 바뀝니다.
