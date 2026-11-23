# 작업 보고서: XXV.3.1 DiagnosticIndex pagination·filter API (코어·CLI·MCP)

- WBS: XXV.3.1
- 작업명: pagination·filter API로 DiagnosticIndex 노출
- 상태: 부분 완료 (`[~]`). 코어 API·CLI·MCP는 완료했습니다. 셸 FFI는 사용자 방향(2026-11-23, FFI 추가 금지)에 따라 UI 재설계
  때 만듭니다.
- 결정: ADR-0095 결정 6·7

## 필수 실행 환경

- `DEV-MAC`(Rust 단위·CLI·MCP 통합 test).

## 변경 파일 목록

- `crates/integrity/src/index.rs`
  - `DiagnosticFilter`: 종류, 최소 심각도, 문서 또는 프로젝트 단위, source.
  - `Cursor`: key와 index revision, 문자열 token.
  - `page`, `count`, 100만 진단 test.
- `crates/ops/src/lib.rs`(lib.rs 등록): `stellar_ops::integrity` 재노출과 저장 snapshot 수집 `diagnose_snapshot`.
  `crates/ops/Cargo.toml`에 의존 한 줄.
- `tools/stellar-cli/src/diagnose.rs`(신규): `stellar diagnose`.
  - 등록: `tools/stellar-cli/src/cli.rs`, `lib.rs`, `tools/stellar-cli/tests/fixtures/help.txt` 한 줄.
  - test: `tools/stellar-cli/tests/diagnose.rs`(신규).
- MCP(소유 밖, tool 등록)
  - `crates/agent-protocol/src/catalog/tools.rs`: `document.diagnostics` 정의와 목록.
  - `crates/mcp-core/src/headless.rs`(등록), `crates/mcp-core/src/headless/integrity.rs`(신규 page).
  - `tools/stellar-mcp/tests/headless_diagnostics.rs`(신규), `tools/stellar-mcp/tests/headless_read.rs`·`headless_tools.rs`(목록).
  - `tools/stellar-mcp/tests/fixtures/catalog.json`·`trust-boundary.jsonl`(`STELLAR_MCP_BLESS=1`로 재생성).

## 구현 요약

- 문서·종류 필터는 key 범위 탐색이라 page 비용이 page 크기에 비례합니다. 심각도·source는 범위 안 조건입니다.
- cursor는 마지막 key와 index revision입니다. index가 바뀌면 거부합니다.
- CLI cursor는 프로젝트 revision token을 함께 담습니다. 저장 프로젝트가 바뀌면 `stale-cursor`(65)로 거부합니다.
- CLI·MCP는 같은 수집 함수와 같은 index 필터를 씁니다. MCP는 연결 공용 paging(`PageSource`)으로 나눕니다.
- 진단 항목: id(16자리 hex), kind, severity, code, document, source, property(ops 속성 경로 표기), message, remediation(action·form).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-integrity index` | 통과(4 passed, 100만 진단 test 포함) |
| `cargo test -p stellar-cli --test diagnose --test headless_help` | 통과 |
| `cargo test -p stellar-mcp --test headless_diagnostics --test headless_read --test headless_tools --test catalog_fixtures --test trust_boundary` | 통과 |
| `cargo test -p stellar-agent-protocol -p stellar-mcp-core` | 통과 |
| `cargo xtask generate-ffi-header --check` | FFI 헤더 drift 없음 |

| 검사 | 결과 |
|---|---|
| 100만 diagnostic pagination: 10,000개 page로 전체를 순서·중복 없이 읽음, 문서+종류+심각도 필터 page 합 = 개수 | 통과(`index::tests::million_diagnostics_paginate_with_filters`) |
| page·필터 결과 = 전체 탐색 결과(limit 1·3·64·1000) | 통과(`index::tests::filtered_pages_equal_filtered_scan_in_key_order`) |
| `stellar diagnose`: 미해결 clip 25개, 10개 page 3개가 전체와 같은 순서, 종류·문서·프로젝트·심각도 필터, 사용법 오류 64, 저장 뒤 cursor 65, 파일 불변 | 통과(`tests/diagnose.rs`) |
| MCP `document.diagnostics`: 같은 수집·필터, 3개 page 이어 읽기, 잘못된 인자 `invalid-arguments`, 파일 불변 | 통과(`tests/headless_diagnostics.rs`) |

## 차이(작업 지시와 다른 점)

- 셸 FFI pagination API를 만들지 않았습니다(사용자 방향). 코어 API `DiagnosticIndex::page`가 FFI가 감쌀 대상입니다.
- MCP tool은 agent-protocol·mcp-core에 등록 범위로만 더했습니다.

## 남은 위험 / 한계

- headless 수집은 미해결 참조와 migration 상태만 포함합니다(ADR-0095 결정 7).
- `docs/guides/stellar-cli.md`에 `diagnose`·`document.diagnostics` 설명을 더하지 않았습니다(소유 밖).
