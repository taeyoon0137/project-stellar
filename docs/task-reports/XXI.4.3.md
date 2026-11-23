# 작업 보고서: XXI.4.3 비동기 request/response·batch RPC와 고빈도 호출 금지

- WBS: XXI.4.3
- 작업명: RPC는 async request/response와 batch를 지원하고 high-frequency pointer/render loop 호출을 금지
- 상태: 완료 (`[x]`)
- 선행: XXI.4.2
- 결정: [ADR-0097](../adr/ADR-0097-plugin-host-core.md) 결정 6·7

## 필수 실행 환경

- `DEV-MAC` (Rust, 로컬 검증).

## 변경 파일 목록

- `crates/plugin-host/src/session.rs`(신규) — `HostSession`(worker thread, `submit`·`recv_timeout`·`set_permissions`·`close`),
  `Clock`, `SystemClock`, envelope 검사, 호출량·처리 중 상한.
- `crates/plugin-host/src/rpc.rs` — 응답 메시지 목록 함수 자리(`chunk_reply`, 조각 나누기는 XXI.4.4).
- `crates/plugin-host/src/lib.rs` — module 등록.
- `crates/plugin-host/tests/rpc_session.rs`(신규).

## 구현 요약

- `submit`은 크기(1 MiB), JSON, `stellar_rpc` 1, id(음이 아닌 정수 또는 64자 이하 문자열), 모르는 envelope key, batch 형식(1..=64)을
  바로 검사하고 즉시 돌아옵니다. 거부도 같은 응답 channel로 id를 붙여 보냅니다.
- 통과한 메시지는 session worker thread 하나가 제출 순서대로 실행합니다. batch 항목은 순서대로 실행하고 앞 항목 실패가 뒤 항목을
  막지 않습니다.
- 호출량: 1초 창 메시지 20개(batch는 1개, 형식 오류로 거부한 메시지도 셈). 넘으면 `rate-limited`와 `retry_after_ms`입니다. 처리 중 16개를 넘으면
  `too-many-in-flight`입니다. 시간은 주입한 `Clock`으로 잽니다.
- 고빈도 namespace(`pointer`, `render`, `frame`, `input`, `animation`)는 IDL 검사가 거부하므로 그런 method가 없습니다(XXI.4.1).
- 권한 변경(`set_permissions`)은 아직 시작하지 않은 요청(queue에 있는 요청 포함)부터 적용합니다. `abort`는 queue에 남은
  요청을 실행하지 않고 `closed`로 답합니다(검토 보완).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-plugin-host --test rpc_session` | 8 passed(검토 보완 뒤) |
| `cargo clippy -p stellar-plugin-host --all-targets -- -D warnings` | 경고 0 |

| 검사(rate/batch fixture) | 결과 |
|---|---|
| 요청 6개가 즉시 queue되고 응답 id가 제출 순서와 같음 | 통과 |
| batch 4항목(성공·모르는 method·성공·형식 오류)이 순서대로 각각 결과 | 통과 |
| batch 65항목 `batch-too-large`, 빈 batch `invalid-request` | 통과 |
| 60Hz loop 60 frame에서 20개만 받고 40개 `rate-limited`(`retry_after_ms` 1..=1000) | 통과 |
| 창이 지난 뒤 64항목 batch 20개(1,280 호출)를 받고 21번째 메시지는 거부 | 통과 |
| 처리 중 상한 2에서 세 번째 요청 거부, worker가 끝나면 다시 받음 | 통과 |
| 잘못된 envelope 7종과 1 MiB 초과 메시지 거부 | 통과 |
| 형식 오류 메시지 30개 중 21번째부터 `rate-limited`, 뒤이은 정상 요청도 거부 | 통과 |
| worker가 실행 중일 때 쌓인 쓰기·읽기가 권한 철회 뒤 `permission-denied`, 파일 쓰기 없음 | 통과 |
| `abort` 뒤 queue의 쓰기는 `closed`, 파일 쓰기 없음 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- session당 worker 하나라 느린 요청(예: 큰 프로젝트 Git status)이 같은 플러그인의 다음 요청을 늦춥니다. 쓰기 순서 결정성을 위해 택했습니다.
- WebView bridge에서 이 session을 부르는 연결은 WebView 작업(XXI.2) 때 합니다.
