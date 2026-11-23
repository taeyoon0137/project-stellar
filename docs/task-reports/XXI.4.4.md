# 작업 보고서: XXI.4.4 대용량 payload 전송(macOS chunked reply 부분)

- WBS: XXI.4.4
- 작업명: Windows 대용량 payload는 Shared Buffer, macOS는 chunked binary reply 또는 승인 방식으로 전송
- 상태: 부분 완료 (`[~]`) — Host 쪽 조각 전송과 JS client 재조립은 완료. WKWebView reply handler 연결은 XXI.2(WebView) 때,
  Windows Shared Buffer는 Windows 지원 보류(ADR-0006)로 하지 않았습니다.
- 선행: XXI.4.3
- 결정: [ADR-0097](../adr/ADR-0097-plugin-host-core.md) 결정 8

## 필수 실행 환경

- `DEV-MAC` (Rust, node 26 형식 제거 실행, deno 형식 검사).

## 변경 파일 목록

- `crates/plugin-host/src/rpc.rs` — `chunk_reply`(UTF-8 경계 조각), `join_chunks`.
- `crates/plugin-host/tests/chunked_reply.rs`(신규) — 조각 규칙, 100 MiB 측정, session 조각 응답, 생성 TypeScript client 종단 smoke.
- `crates/plugin-host/tests/ts_client/smoke.mts`(신규) — node로 실행하는 client 쪽 script.

## 구현 요약

- 직렬화한 응답이 256 KiB(`RpcLimits::chunk_bytes`)를 넘으면 `{"stellar_rpc":1,"id":…,"chunk":{"index","count","data"}}` 메시지로
  나눕니다. 응답 문자열을 조각으로 한 번만 복사하고 UTF-8 경계에서 자릅니다.
- 생성 `createHostClient`가 같은 id의 조각을 index 순으로 모아 원래 응답으로 해석합니다(XXI.4.1 생성 코드).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-plugin-host --test chunked_reply -- --nocapture` | 4 passed |
| `deno check crates/plugin-host/tests/ts_client/smoke.mts` | 통과 |

| 검사 | 결과 |
|---|---|
| 한글 응답을 100 bytes 조각으로 나눠 모든 조각 ≤ 100 bytes, 이어 붙이면 원본, 빠짐·중복 조각 거부 | 통과 |
| 100 MiB 응답: 조각 401개, 전송 104,884,802 bytes(원본 104,857,636, 부가 0.03%), 나누기 4.1 s(debug build, 부하 있는 개발 장비) | 측정 |
| session이 1 KiB 상한에서 16 KiB `file.read` 결과를 조각으로 보내고 이어 붙인 결과가 원본과 같음 | 통과 |
| node로 띄운 생성 TypeScript client ↔ Rust session: 쓰기·조각 읽기·batch(권한 거부 포함)·오류 class·`host.info` | 통과 |

## 남은 위험 / 한계

- 조각 data는 JSON 문자열이라 따옴표·escape가 많은 응답은 전송 크기가 늘어납니다. 진짜 binary 전송(WKWebView `WKScriptMessage` reply의
  binary 또는 승인 방식)은 WebView 연결 때 정합니다.
- 100 MiB 측정은 debug build 한 번입니다. release·기준 장비 측정과 memory peak 측정은 하지 않았습니다.
- node·deno가 없는 환경에서는 TypeScript 종단 test가 건너뜁니다.
