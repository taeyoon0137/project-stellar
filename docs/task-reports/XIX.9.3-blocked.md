# 차단 보고서: XIX.9.3 승인 프로젝트의 headless MCP 읽기

## WBS

- WBS: XIX.9.3
- 작업명: 기존 신뢰 프로젝트 하나의 snapshot 읽기를 정상 stdio MCP 실행 파일에 연결
- 상태: `[!]`
- 선행: XIX.9.2 `[ ]`, XXIV.4.3 `[~]`, XVI.7.2 `[x]`, XIX.3.3 `[ ]`

## 실패 선행조건

- XIX.9.2·XIX.3.3이 `[ ]`이고 XXIV.4.3이 `[~]`입니다. 계획 1.2 규칙 4는 선행이 모두 `[x]`가 아니면 진척을 올리지 않습니다.
- 다른 Git revision 비교, dry-run tool, agent-protocol 카탈로그 이동이 남아 범례상 `[~]`에도 해당하지 않습니다.
  2026-11-14 통합에서 `[~]`로 올렸고, 2026-11-15 검토에서 `[!]`로 고쳤습니다.

## 재현 명령

- `rg -n '^\| (XIX\.9\.2|XXIV\.4\.3|XIX\.3\.3|XIX\.9\.3) ' docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`
- `cargo test -p stellar-mcp --test headless_read` — 이미 있는 읽기 경로의 실행 파일 검사입니다. 2026-11-15 DEV-MAC의
  `cargo test --workspace`(종료 0)에 포함되어 통과했습니다.

## 필요한 결정과 환경

- `DEV-MAC`. 프로젝트 선택은 ADR-0005 §3대로 절대 루트 인자(`stellar-mcp --headless --project <절대 경로>`)를 씁니다.

## 변경되지 않은 파일

- 선행 완료 전에 들어온 구현은 유지합니다: `crates/mcp-core/src/headless.rs`의 고정 backend, 구조·컴포넌트·문서 resource 읽기,
  `document.semantic_diff`(Git HEAD 대비)와 `document.validate` tool(`e7d46f02`, `2f0d881a`).
- 2026-11-15 검토에서 프로젝트 ID 선택을 절대 루트 인자로 바꿨습니다(`314709b3`). 처음 보고서는 ID 선택과 Codex 실행 결과를
  근거로 썼으므로 남기지 않았습니다.
- 다른 Git revision 비교, dry-run tool, 카탈로그 이동, 앱 UI·외부 클라이언트 연결은 하지 않았습니다.

## 재개 조건

- 선행이 모두 `[x]`가 된 뒤 남은 범위를 끝내고 검사를 다시 실행해 보고서를 완료 형식으로 씁니다.
