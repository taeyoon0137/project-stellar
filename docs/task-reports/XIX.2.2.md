# 작업 보고서: XIX.2.2 resource 응답의 revision과 page cursor

- WBS: XIX.2.2
- 작업명: resource 응답에 revision과 pagination cursor를 넣고 대형 문서를 한 응답에 보내지 않음
- 상태: 완료 (`[x]`)
- 선행: XIX.2.1(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `tools/stellar-mcp/src/catalog/paging.rs`(신규) — `PageSource` trait, `PageRequest`, `build_page`, `read_result`, `parse_page_query`, `PageError`, 한도 상수.
- `tools/stellar-mcp/src/catalog/mod.rs` — `paging` 모듈.
- `docs/task-reports/XIX.2.2.md`(신규), 계획서(XIX.2.2 `[ ]`→`[x]`).

## 구현 요약

- **응답 형태**: `resources/read` 결과는 MCP 명세 2025-11-25 Resources "Text Content"대로 `{"contents":[{"uri","mimeType":"application/json","text"}]}`이고 `text`가 내용 JSON입니다. page 내용은 `revision`, resource별 header(예: `document`), `total`, `offset`, `limit`, `items`, `next_cursor`(마지막이면 `null`)이며 XIX.2.1 page schema와 같은 key입니다.
- **한도**

| 한도 | 값 | 근거 |
|---|---|---|
| 기본 item 수 | 500 | 20만 node 문서를 400 page 안에 순회 |
| 최대 item 수 | 2000 | 요청 limit 상한(넘으면 `InvalidLimit`) |
| page 내용 JSON 최대 | 512 KiB | 브리지 MCP 메시지 상한 1 MiB(`server::MAX_MESSAGE_BYTES`)의 절반. text content로 감쌀 때 escape와 JSON-RPC envelope 여유 |

  item 수와 별도로 byte 한도를 넘지 않게 item을 덜 담고 그 위치부터 `next_cursor`를 줍니다. item 하나가 한도를 넘으면 `ItemTooLarge`로 거부합니다. 그래서 대형 문서 전체가 한 응답에 실리지 않습니다.
- **cursor**: 명세 "Pagination"대로 client에게 불투명한 문자열이며 내부 표기는 `p1.<offset>.<resource key>.<revision>`입니다. 다른 resource key(`ForeignCursor`), 다른 revision(`StaleCursor`, 그 사이 저장), 형식 오류(`InvalidCursor`), 범위 밖(`CursorOutOfRange`)을 거부합니다.
- **URI query**: page resource URI의 `?cursor=..&limit=..`만 받고, 다른 key·중복·선행 0·빈 값은 거부합니다(`parse_page_query`).
- backend 데이터 연결 전이라 `PageSource`는 trait입니다. headless(XIX.9.3)는 `stellar-ops` 문서 트리를, attached(XIX.9.4)는 앱 snapshot을 구현체로 연결합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-mcp` | lib 43 passed, 통합 test 전부 passed, 0 failed |
| `cargo clippy -p stellar-mcp --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-mcp -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 20만 node pagination: 깊이 2 tree 200,000 item을 기본 limit로 끝까지 순회, 매 page `revision`·`total`·`offset` 정확, 순서·중복·누락 0, 매 `resources/read` 메시지 < 1 MiB, 내용 ≤ 512 KiB(`two_hundred_thousand_nodes_are_paged_with_revision_and_bounded_messages`, debug 약 13초) | 통과 |
| 큰 item이면 byte 한도로 page를 줄이고 이어지는 cursor 제공(`byte_budget_shrinks_pages_with_large_items`) | 통과 |
| 낡은·다른 resource·형식 오류·범위 밖 cursor, 잘못된 limit 거부(`foreign_stale_malformed_and_out_of_range_cursors_are_rejected`) | 통과 |
| URI query 엄격 해석(`page_query_is_parsed_strictly`), text content 감싸기 왕복(`read_result_wraps_text_content`) | 통과 |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- 20만 node fixture는 실제 문서가 아니라 같은 모양의 합성 page source입니다. `stellar-mcp`는 정책상 `stellar-ops`에 의존할 수 없어(ADR-0005 §8, mcp-core는 XIX.9.1 범위) 실제 문서 연결은 backend 작업에서 합니다. 실제 20만 node 문서의 page 순회는 `stellar-ops`(XXIV.4.3)에서 확인했습니다.

## 남은 위험 / 한계

- 호출량·대기 시간 상한은 XIX.9.10에서 정합니다.
- revision이 바뀌면 진행 중인 순회는 처음부터 다시 해야 합니다(낡은 cursor 거부).
