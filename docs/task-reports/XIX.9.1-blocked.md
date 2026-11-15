# 차단 보고서: XIX.9.1 MCP library의 mcp-core 경계 이동

## WBS

- WBS: XIX.9.1
- 작업명: 기존 MCP library를 ADR-0005의 mcp-core 경계로 이동
- 상태: `[!]`
- 선행: XIX.1.3 `[x]`, XXIV.4.2 `[~]`, XXIX.1.1 `[~]`

## 실패 선행조건

- XXIV.4.2와 XXIX.1.1이 `[~]`입니다. 계획 1.2 규칙 4는 선행이 모두 `[x]`가 아니면 진척을 올리지 않습니다.
- 행 범위 중 agent-protocol 등록과 정의 이동이 남아 있어 범례상 `[~]`(구현 완료·증거 미완)에도 해당하지 않습니다.
  2026-11-14 통합에서 `[~]`로 올렸고, 2026-11-15 검토에서 `[!]`로 고쳤습니다.

## 재현 명령

- `rg -n '^\| (XXIV\.4\.2|XXIX\.1\.1|XIX\.9\.1) ' docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`
- `cargo test -p stellar-mcp -p stellar-mcp-core` — 이미 있는 경계의 검사입니다. 2026-11-15 DEV-MAC의 `cargo test --workspace`
  (종료 0)에 포함되어 통과했습니다.

## 필요한 결정과 환경

- `DEV-MAC`. 새 결정은 필요 없습니다. ADR-0005 §8의 `stellar-mcp → stellar-mcp-core → stellar-ops` 간선을 따릅니다.

## 변경되지 않은 파일

- 선행 완료 전에 들어온 구현은 유지합니다: `crates/mcp-core/`(기존 MCP library 모듈 이동과 ops in-process 연결),
  `tools/stellar-mcp/src/lib.rs` 재노출, crate graph 등록. XIX.9.3의 한 읽기 경로에 필요한 범위만 옮겼습니다.
- agent-protocol 공용 정의·생성 schema 이동, catalog 이동(XIX.9.2), attached host trait는 하지 않았습니다.

## 재개 조건

- XXIV.4.2·XXIX.1.1이 `[x]`가 된 뒤 남은 이동을 끝내고 검사를 다시 실행해 보고서를 완료 형식으로 씁니다.
