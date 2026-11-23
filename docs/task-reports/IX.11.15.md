# 작업 보고서: IX.11.15 CSS 선언 블록 복사·붙여넣기 core API

- WBS: IX.11.15
- 작업명: 선택 node 속성을 CSS 선언 블록으로 복사하고, 선언 블록을 붙여넣어 CommandTransaction 하나로 적용(건너뜀 보고)
- 상태: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 남은 범위가 있음(아래 본문); 선행 IX.11.14(`[~]`), IX.11.16(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]`)
- 결정: ADR-0075 결정 6

## 필수 실행 환경

- `DEV-MAC`(Rust 단위·CLI 통합 테스트).

## 변경 파일 목록

- `crates/document/src/css/declaration.rs`(신규) — 선언 블록 해석(주석 제거, `selector { … }` 규칙 하나, 괄호·따옴표
  밖 `;` 분리, 형식이 틀린 조각은 이유와 함께 보존, `!important` 거부, 16 KiB·256 선언 상한)과 정규 출력
  (`property: value;` 한 줄에 하나). `crates/document/src/css/mod.rs` 등록.
- `crates/commands/src/css_value.rs` — `copy_declarations`·`declaration_block_text`(속성 패널과 같은 이름·정규
  값, 묶음 행은 longhand로, 레이어 속성 제외, Stellar 전용 표시), `paste_declarations`·`PastePlan`·
  `SkippedDeclaration`·`AppliedDeclaration`·`PasteError`(shorthand 펼침, 문서 사본에서 선언을 순서대로 적용해 뒤
  선언이 이기고 새 modifier 순서 key가 겹치지 않음, 결과는 transaction 하나), test.
- `crates/commands/src/inspector.rs` — 같은 값이면 빈 이동 Transform modifier를 넣지 않음, 값이 같은
  `SetModifierProperty`를 빼는 규칙("값이 같으면 command 없음"의 누락, 검토 보완). 물려받은 Text 글자색과 같은 색을
  넣어도 명시 Fill을 만듭니다(ADR-0082 결정 5, 검토 보완에서 처음 구현의 억제 규칙을 되돌림).
- `crates/commands/src/modifier_ops.rs`(소유 밖, 최소 변경) — payload 읽기 함수를 crate 안에 공개.
- `crates/ops/src/css.rs`(신규) — headless `copy`·`paste`(붙여넣기는 `apply`의 base revision 검사·저장 잠금·세대
  전환 저장 한 transaction). `crates/ops/src/lib.rs` 등록.
- `tools/stellar-cli/src/css.rs`(신규) — `stellar css copy|paste [--dry-run]`, 건너뜀은 결과 `skipped`와 경고
  진단으로 보고. `tools/stellar-cli/src/cli.rs`·`lib.rs` 등록, `tools/stellar-cli/tests/fixtures/help.txt`(도움말
  snapshot에 `css` 한 줄), `tools/stellar-cli/tests/css.rs`(신규 통합 test).

## 구현 요약

- 건너뜀 이유 코드: 값 오류 코드(`unsupported-unit`·`unsupported-value`·`unknown-token`·`forbidden-value`·
  `invalid-value`), `invalid-declaration`, `unknown-property`(저장 위치 없음, 예: `padding`·`margin`), `unsupported-property`(registry에
  저장 위치가 있지만 속성 패널 행이 없어 붙여넣기 경로가 아직 없음, 예: `opacity`·`transform`·`stroke-linecap`), `not-applicable`
  (선택 node 종류에 없는 속성, 레이어 속성), `rejected`(잠금·D7 등 편집 규칙). Constraint CenterX·CenterY가
  내는 `calc(50% + <offset>)`는 parser가 받지 않아 `unsupported-value`로 건너뜁니다.
- `var(--name)`은 해석은 하지만 값 binding 저장 경로가 없어 `unsupported-value`로 건너뜁니다(조용히 버리지 않음).
- shorthand는 펼친 longhand가 모두 속성 패널 속성이면 longhand로(`border-radius` → 4 모서리), 그렇지 않고 같은
  이름의 행이 있으면 그 행으로(`border`, `overflow`), 둘 다 아니면 longhand별로 판정합니다.
- 붙여넣기는 문서 사본(레코드에서 재구성)에서 계산하므로 비용은 문서 크기에 선형입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands --lib` | 통과(css_value 5개 포함) |
| `cargo test -p stellar-document --lib css::declaration` | 3 passed |
| `cargo test -p stellar-ops --lib css` | 1 passed |
| `cargo test -p stellar-cli` | 실패 0(새 `tests/css.rs` 2개, 도움말 snapshot 포함) |
| `cargo clippy -p stellar-commands -p stellar-ops -p stellar-cli -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| copy→paste 왕복: 같은 node에 붙이면 건너뜀 0·command 0(7종 node), 바꾼 뒤 복사해 둔 블록을 붙이면 style이 같아짐 | 통과 |
| undo 한 번(transaction 하나의 역)으로 semantic snapshot 전체 복원 | 통과 |
| 건너뜀 보고 fixture(Constraint `calc()` 형태 포함, 이유 메시지 비어 있지 않음) | 통과 |
| 뒤 선언 우선, Fill·Stroke·이동 modifier 삽입 순서 key 충돌 없음 | 통과 |
| `stellar css copy` JSON 속성 이름이 CSS longhand(`width`, `background-color`, `border-*-radius`), 묶음 이름·옛 modifier 이름 없음 | 통과 |
| `stellar css paste` 한 transaction 저장, dry-run은 쓰지 않음, 블록 오류 65·사용법 64·없는 node 66 | 통과 |
| `stellar query node` 속성 경로가 longhand key(0x0108·0x010b)이고 묶음 key(0x0101·0x0106)가 아님 | 통과 |
| Swift·셸 CSS 문자열 조립 신규 0 | `apps/`·`crates/ffi` 미변경(`git diff --stat`) |

## 차이(작업 지시와 다른 점)

- `stellar query`·`stellar apply` JSON의 속성 경로는 숫자 key 표기이며 이 lane 소유 밖이라 이름 표기로 바꾸지 않았습니다. 대신 경로가 longhand key(0x0108·0x010b)인지 CLI 통합 test로 확인하고, CSS 이름은 새 `stellar css`가 냅니다.
- 속성 패널 편집 규칙(빈 이동 modifier, 같은 값 modifier 편집)을 "값이 같으면 command 없음"에 맞게 고쳤습니다.
  물려받은 Text 색은 편집 규칙을 바꾸지 않고 복사가 명시 값만 내도록 했습니다(ADR-0082 결정 5).
- 복사·붙여넣기 범위는 속성 패널 행이 있는 속성입니다. `opacity`·`mix-blend-mode`·`transform`·`stroke-linecap`·
  `image-rendering`·shadow·blur는 복사에 나오지 않으므로, 이 속성이 있는 node의 copy→paste 왕복은 semantic snapshot
  전체가 아니라 속성 패널 속성 범위에서만 같습니다.

## 남은 위험 / 한계

- `stellar query`·`stellar apply` 결과의 속성 경로 표기는 숫자 key 그대로입니다(`crates/ops/src/query.rs`, 이 lane
  소유 밖). CSS 이름은 `stellar css`와 `stellar_semantic_diff::keys::display_name`으로 얻습니다.
- `docs/guides/stellar-cli.md`에 `css` 명령 설명을 더하는 일은 문서 소유 범위 밖이라 하지 않았습니다(도움말에는 있음).
- 셸 클립보드 연결과 MCP 도구 노출은 UI 재설계·agent-protocol-mcp lane 범위입니다.
