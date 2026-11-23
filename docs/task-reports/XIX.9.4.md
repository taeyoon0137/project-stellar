# 작업 보고서: XIX.9.4 attached backend

- WBS: XIX.9.4
- 작업명: attached backend(endpoint 실시간 검증으로 backend 선택, WindowSessionId 결합, 모든 쓰기를 앱의 lease·승인·DocumentThread 경로로)
- 상태: 미완료 (`[ ]`) — 앱 연결의 frame 왕복(bridge 쪽 frame 쓰기, 앱 쪽 연결 감시가 앞단으로 넘김)만 했습니다. backend 선택 matrix, MCP 서버 루프 연결, 다중 창 fixture는 남았고 선행 XIX.9.3이 `[x]`가 아닙니다.
- 선행: XIX.9.3, XIX.1.7, XIX.1.8, XIX.5.4, XII.2.1
- 결정: [ADR-0132](../adr/ADR-0132-agent-lease-judgement-cost-bridge-link-and-fail-closed-observation.md) 결정 2·3, [ADR-0124](../adr/ADR-0124-agent-runtime-host-link-frames-and-observation.md) 결정 3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 셸 UI와 FFI는 범위가 아닙니다(2026-11-23 엔진 우선 방향).

## 변경 파일 목록

- `crates/mcp-core/src/attached.rs`·`attached_tests.rs`(신규), `crates/mcp-core/src/lib.rs`(등록)
- `crates/local-ipc/src/session.rs`: `FrameService`, `ConnectionMonitor::serve`, `CloseCause::of_frame_error`
- `crates/runtime/src/agent_frame.rs`·`agent_frame_tests.rs`: `FrameConnection`, `ConnectionBinding`

## 구현 요약

- "모든 쓰기를 앱의 lease·승인·DocumentThread 경로로": bridge의 편집 tool(`document.begin_agent_edit`, `document.propose_commands`)과 완료는 `AttachedLink`가 frame으로만 앱에 보냅니다. bridge는 lease·transaction id를 만들지 않고, 앱 앞단이 host trait 상한 판정 뒤 I/O 스레드(`AgentIo::submit`)로 document thread에 넣습니다. 첫 연결 승인 전 요청은 앱이 거부합니다.
- "결합 단위는 WindowSessionId": 앱 쪽 연결 하나는 `ConnectionBinding`의 창 하나에 묶입니다(외부 클라이언트: 고른 창, 앱 시작 세션: 시작한 창).
- 자세한 결정과 검증은 `docs/task-reports/XXIX.3.4.md` "추가 작업 2"에 있습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-mcp-core --lib attached` | 통과 |
| `cargo test -p stellar-local-ipc --lib session` | 통과 |
| `cargo test -p stellar-runtime --lib agent_frame` | 통과 |

| 검증 기준 | 결과 |
|---|---|
| backend 선택 matrix(앱 없음, 검증 실패, 검증 성공) fixture | 하지 않음(서버 루프·진입점 연결 전) |
| 다중 창에서 결합 창 외 history 변화 0 | 하지 않음 |

## 남은 위험 / 한계

- MCP 서버 루프(`server`)·진입점(`entry`)이 attached 모드에서 `AttachedLink::call_tool`을 쓰고, bridge 앱 연결 감시(`attach::watch_app_connection`)를 link의 응답 읽기로 바꾸는 일이 남았습니다.
- endpoint 실시간 검증으로 backend를 고르는 일(앱 없으면 headless 등)은 하지 않았습니다.
- 앱 listener 연결 event를 `ConnectionMonitor::serve`에 잇는 앱 수명 코드와 창 선택 화면은 UI 재설계 뒤입니다.
