# 작업 보고서: II.2.8 crate 의존 그래프와 금지 edge 검사

- WBS: II.2.8
- 작업명: `docs/architecture/crate-dependency-graph.md` + `validate-crate-graph` 검사기 + CI 연결
- 상태: 완료 (`[x]`)
- 선행: II.2.2~II.2.7 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/crate_graph.rs` (신규, 허용 edge 정책 + 검사)
- `tools/xtask/src/commands.rs` (`validate-crate-graph` 명령, COMMANDS=13)
- `tools/xtask/src/lib.rs` (`pub mod crate_graph`)
- `docs/architecture/crate-dependency-graph.md` (신규, Mermaid + 표)
- `.github/workflows/bootstrap.yml` (plan-consistency에 validate-crate-graph 추가)
- `docs/task-reports/II.2.8.md`, 계획서(II.2.8 `[ ]`→`[x]`)

## 구현 요약

- 허용 직접 의존 edge를 정책으로 인코딩(문서와 동일). `crates/*/Cargo.toml`에서 내부 edge를 추출해 정책 위반(`forbidden-edge`/`unknown-crate`)을 검사.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 허용 edge fixture | 통과 |
| 역방향 edge(core-types→document) fixture | `forbidden-edge` |
| 금지 cross-layer edge fixture | `forbidden-edge` |
| 실제 `cargo xtask validate-crate-graph` | 종료 0 |
| fmt / clippy --locked / test(101) | 통과 |
| bootstrap.yml YAML | 유효 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 정책을 `crate_graph.rs`에 인코딩하고 문서(graph.md)와 수동 정합. 실제 CI 실행은 CI 환경에서 확인됩니다.
