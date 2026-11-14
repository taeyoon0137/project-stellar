# ADR-0029: 입력 T0 수신 기록과 hit-test request 지연 trace

## Status

Accepted (2026-11-14, 구현 기록)

## Context

XII.3.5는 InputEvent를 받을 때 T0 trace를 찍고 DocumentThread hit-test request까지의 queue latency를
재도록 요구합니다. 검증 기준은 "trace에 T0/T1 후보 timestamp 존재"입니다. 아키텍처 §6.1은 T0을
"운영체제 입력 이벤트를 Stellar가 수신", T1을 "이벤트 병합·좌표 변환·Hit Test 완료"로 정의합니다.
입력별 trace ID와 T0 확정은 계측 단계(XVI.1.1)가 맡습니다.

제약은 다음과 같습니다.

- 셸 입력은 `stellar-ffi`가 공통 `InputEvent`로 바꾸고(XII.3.1, XII.3.2) 셸→코어 대기열에서 연속
  move·wheel을 합칩니다(XII.3.4). XII.3.4 보고서는 병합 전 최초 수신 시각을 어떻게 남길지를 XII.3.5에
  넘겼습니다.
- document thread 수신함(XII.1.2)은 `stellar-runtime`에 있고, runtime은 ffi를 역참조하지 않습니다
  (ADR-0013). 따라서 runtime은 `InputEvent` 타입을 모릅니다.
- 시각을 서로 비교하려면 수신 쪽과 document thread가 같은 단조 시계를 읽어야 합니다. 시스템 wall
  clock은 쓰지 않습니다(II.3.3).
- 진단 trace에는 입력 내용(위치, key, text)을 남기지 않습니다(XVI 진단 원칙).

## Decision

1. **T0 후보 두 개**를 `InputReceipt`에 담습니다. 하나는 운영체제 이벤트 시각(native timestamp를
   코어 단조 domain으로 옮긴 `InputEvent::timestamp`)입니다. 다른 하나는 코어 경계가 이벤트를 받은 시각으로,
   `stellar_runtime_submit_mac_event`가 잠금·변환 전에 프로세스 단조 시계로 잽니다.
2. **병합 규칙**: 셸→코어 대기열이 이벤트를 합치면 이벤트 값은 최신으로 바꾸고, receipt는 묶음에서
   **가장 이른** 값을 유지합니다. 합쳐진 이벤트 수도 셉니다. 아직 반영되지 않은 가장 오래된 입력을
   기준으로 지연을 재기 위해서입니다.
3. **hit-test request**: `DocumentSender::submit_hit_test(class, receipt, clock, hit_test)`가 input
   lane에 job을 접수합니다. 접수 직전 `enqueued`를 찍고, document thread에서 `hit_test` 앞뒤로 T1
   후보 `started`·`finished`를 찍습니다. 결과는 `HitTestResponse { value, trace }`입니다. hit test 본문은
   호출자가 넘깁니다. 실제 canvas hit test를 넣는 앱 경로 연결은 XII.3.7 재개 조건이 맡습니다(Consequences).
   XII.4.1\~XII.4.4는 렌더 연결·camera·overlay만 다루고 입력 전달을 맡지 않습니다.
4. **시계**는 호출자가 `TraceClock`(`Arc<dyn MonotonicClock + Send + Sync>`)으로 넘깁니다. 앱은 FFI의
   프로세스 단조 시계를, 테스트는 수동 시계를 씁니다.
5. **접수 분류**: `InputEvent::queue_class()`가 공통 이벤트를 `InputEventClass`로 바꿉니다
   (XII.1.2에서 ffi 책임으로 남긴 변환). pointer move는 장치별로 병합하고, resize는 최신 값으로
   병합합니다. 그 밖의 이벤트는 병합하지 않습니다.
6. trace에는 시각, 입력 분류, 병합 수만 담습니다.
7. C ABI는 바꾸지 않습니다. receipt는 Rust 쪽 대기열 항목에만 붙고, `stellar_runtime_poll_input`
   바이트와 `StellarMacEvent` 레이아웃은 그대로입니다.

## Alternatives

- **셸(Swift)에서 T0 기록:** 셸 시각(`ProcessInfo.systemUptime`)은 document thread가 읽는 코어 시계와
  축이 다르고 변환 오차가 생깁니다. 셸이 넘기는 `receive_uptime`은 시각 domain 기준점으로만 쓰고,
  T0 후보는 코어 경계에서 잽니다.
- **병합 시 최신 receipt 유지:** 연속 move의 지연이 마지막 이벤트 기준으로만 남아 실제 대기 시간을
  과소평가하므로 탈락했습니다.
- **trace를 runtime 전역 log에 저장:** 지금은 소비자가 없습니다. 보존 정책(용량, 내보내기)은 XVI.1이
  정하므로 결과값으로만 돌려줍니다.
- **runtime이 `InputEvent`를 직접 받기:** runtime → ffi 역의존이 생기므로 탈락했습니다(ADR-0013).

## Consequences

- 대기열 항목마다 receipt(u64 두 개와 u32 하나)가 붙습니다.
- document thread 수신함에서 같은 key의 더 새 request로 대체된 request는 실행되지 않습니다. 그래서
  trace도 남지 않고, 대체한 request의 receipt는 자기 값입니다. 수신함 단계의 과소평가는 남습니다.
- 셸 대기열을 document thread로 자동 전달하는 연결은 이 결정에 없습니다. 지금 셸 대기열은 입력 검사
  경로(`stellar_runtime_poll_input`)만 읽고, 이 경로의 `InputQueue::pop`은 receipt를 버립니다. 그래서 앱에서는
  아직 hit-test request trace가 생기지 않습니다(Rust 종단 테스트에서만 확인). 앱 경로 연결(대기열
  `pop_received` → `submit_hit_test` → canvas hit test → 상호작용 상태 기계)은 XII.3.7 재개 조건이 맡습니다
  (`docs/task-reports/XII.3.7-blocked.md`).

## Compatibility

- C ABI, FFI 헤더, 입력 이벤트 직렬화 레이아웃은 바뀌지 않습니다.
- Rust 공개 API를 추가합니다: `stellar_runtime::input_trace` 모듈, `InputQueue::push_received`·
  `front_receipt`·`pop_received`, `InputEvent::queue_class`. 기존 `InputQueue::push`는 이벤트 시각을
  수신 시각으로 씁니다.

## Performance

- 수신 경로에 단조 시계 읽기 1회를, hit-test request마다 시계 읽기 3회와 `Arc` 복제 1회를 더합니다.
  §6.2의 입력 전달 예산(0.15ms) 충족 여부는 실제 앱 경로와 기준 장비에서 아직 측정하지 않았습니다.
  XII.1.2 lane 기본 용량은 실제 부하를 잰 뒤 조정합니다.

## Security

- trace에 입력 내용이 없으므로 진단 내보내기에서 민감 데이터가 새지 않습니다.

## Migration

- 없음. 저장 데이터와 C ABI에 영향이 없습니다.
