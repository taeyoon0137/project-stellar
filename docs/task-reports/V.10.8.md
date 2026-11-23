# 작업 보고서: V.10.8 컴포넌트·View Frame codec 쓰기 v4

## WBS

- WBS: V.10.8
- 등록: 2026-11-23 plan-revision이 임시 번호 TS-R2를 이 ID로 계획서에 등록하고 보고서를 `docs/task-reports/pending-wbs/TS-R2.md`에서 옮김
- 작업명: 컴포넌트·View 원본 쓰기 버전 v4(registry 이름·CSS 값 문법, v1~v3 읽기 전용)
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록(ledger-refresh): 구현·검증은 끝났고, `[~]`의 이유였던 선행 IX.11.2·IX.11.3이 `[x]`가 되었습니다. v4 codec이 문서 CSS 값 formatter·parser와 나뉜 longhand 이름을 쓰는 보완도 들어왔습니다. 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 원장이 근거 커밋을 찾지 못했습니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장이 근거 커밋을 채우지 못합니다; 선행 IX.11.2(`[~]`), IX.11.3(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]`)
- 선행: V.10.2 `[x]`, IX.11.2 `[~]`, IX.11.3 `[~]`
- 구현 커밋: `58709614`. 결정: [ADR-0077](../adr/ADR-0077-text-source-full-semantics-and-migration-registry.md) 결정 2. 명세: [`text-source-v4.md`](../spec/text-source-v4.md).

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일

- `crates/project/src/document_io/source_v4/{mod,css,keys,node,styles,view,tests}.rs`, `golden/view-v4.stellar.yaml.in`(신규).
- `crates/project/src/document_io/text_codec.rs`(+`document_font.rs`, `shape.rs`): 쓰기는 v4로 위임, v1·v2는 읽기 전용(v2 writer는 읽기 회귀 fixture용).
- `crates/project/src/document_io/component_source.rs`: 컴포넌트 원본 쓰기 v4(`nodes` 목록 하나, `local-tokens`), v1~v3 읽기.
- 테스트 갱신: `crates/project/tests/text_source.rs`, `tools/stellar-cli/tests/{component_source,text_source,web_frame}.rs`.
- 명세: `docs/spec/text-source-v4.md`(신규), `docs/spec/{text-view-source-v1,component-source-v1}.md`(읽기 전용 표기).

## 구현 요약

- 파일 키는 `frame_metadata`·`payload_metadata`·`modifier_metadata` registry의 `name`, 순서는 registry 순서입니다. codec은 registry를 조회만 합니다(`keys.rs`). 묶음 `border`는 CSS longhand 열두 줄이고 네 변이 같아야 합니다.
- 값은 CSS 문법(`px`, `deg`, `color()`, `matrix()`, `transparent`, 초기값 `medium`·`none`·`currentcolor`)이며 최단 왕복 표기만 받습니다.
- 모든 node 종류와 모든 modifier를 씁니다. 인스턴스 override는 리터럴 값 타입만 씁니다.
- golden 테스트는 registry 의존 키를 registry 조회로 채워, registry 이름이 바뀌어도 codec·golden 파일을 고치지 않습니다.
- CLI·MCP 컴포넌트 조회 JSON은 MCP catalog schema가 고정한 기존 표현을 유지합니다(css-registry lane 범위).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project --lib source_v4` | golden bytes 왕복, 모르는 키·값·비canonical 숫자·변마다 다른 테두리·옛 버전 거부, 리터럴 아닌 값 타입 쓰기 거부 통과 |
| `cargo test -p stellar-project --test text_source` | 13 passed |
| `cargo test -p stellar-cli --test component_source` | 10 passed: v1→v4 재작성, 비인접 줄(width·background, 떨어진 모서리) git merge 깨끗, 인접 줄 git 충돌 |
| `cargo test -p stellar-cli --test text_source --test web_frame` | 통과 |
| `cargo clippy -p stellar-project -p stellar-ops -p stellar-cli --all-targets -- -D warnings` | 0 |

## 차이

- registry 순서에서 이웃한 줄(`width`·`height`, 이웃한 모서리)은 Git 텍스트 충돌입니다. ADR-0061의 이름순 배치보다 후퇴하며 merge driver(XXIV.2.1)로 해결합니다. 기존 CLI 병합 테스트는 사이에 다른 줄이 있는 width·background로 바꿨습니다.

## 테스트

- 위 명령 표의 테스트가 모두 통과했습니다. 남은 경고 수: clippy `-D warnings` 0.

## 성능

- 측정하지 않았습니다. 열기·저장 예산 판정은 기본 저장 전환(V.10.7)에서 실측합니다.

## 남은 위험

- CLI·MCP 컴포넌트 조회 JSON 키는 아직 옛 한 줄 표현이라 파일 키와 이름이 다릅니다(css-registry lane이 MCP schema와 함께 바꿈).

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(V.10.8)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `58709614` Feat: View·컴포넌트 원본 쓰기 v4를 registry 이름과 CSS 값 문법으로 전환
- `c6e5e0ba` Fix: 텍스트 원본 codec이 나뉜 longhand와 Image registry 이름을 그대로 쓰게 함
- `a6a51169` Refactor: 텍스트 원본 v4가 문서 CSS 값 formatter·parser를 쓰게 함
