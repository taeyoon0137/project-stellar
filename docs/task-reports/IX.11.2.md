# 작업 보고서: IX.11.2 Frame·Modifier registry CSS 매핑

- WBS: IX.11.2
- 작업명: Frame·Modifier metadata registry의 모든 항목에 속성 모델 5장·9장 CSS 대응을 채우고 이름을 CSS longhand 또는 Stellar 이름으로 맞춤
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록(ledger-refresh): 이 행의 남은 범위가 없고, `[~]`의 이유였던 선행 사슬(IX.11.22 → IX.11.1)이 풀렸습니다. IX.11.22는 integration-followups lane이 다값 snapshot 경로·Shadow offset 투영·variant validator 연결을 끝내 `[x]`입니다. 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 원장이 근거 커밋을 찾지 못했습니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 선행 IX.11.1(`[~]`), IX.11.22(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]`)
- 결정: ADR-0075 결정 3·4

## 필수 실행 환경

- `DEV-MAC`(Rust 단위 테스트).

## 변경 파일 목록

- `crates/document/src/frame_metadata.rs` — `FramePropertyMetadata.css`(13개 항목 모두), `display_label`,
  `CLIPPING_RULES`(Disabled `overflow: visible`, FrameBounds `overflow: clip; overflow-clip-margin: border-box`,
  Path `clip-path`), 이름 `background`→`background-color`, `corner-radius`→`border-radius`(묶음 key).
- `crates/document/src/modifier_metadata.rs` — `ModifierPropertyMetadata.css`(20개 항목), 이름을 CSS longhand
  (`transform`, `fill`, `stroke`, `stroke-width`, `mask-image`, `clip-path`, `mix-blend-mode`, `opacity`) 또는
  Stellar 이름(`enabled`, `modifier-order`, `boolean-operation`, `boolean-operands`, `stroke-expand-width`,
  `corner-radius`, `shadow-color`, `shadow-offset[-x|-y]`, `shadow-blur`, `blur-radius`)으로 바꿈, `display_label`.
- `crates/document/src/css/registry.rs`(신규) — 범위별 통합 view, `entry`·`entry_by_name`·`display_label`,
  `snapshot_text`, 매핑 누락·이름 규칙·golden·속성 모델 표 비교 test. `registry_snapshot.txt`(golden, 신규).
- `crates/semantic-diff/src/keys.rs` — `display_name`(semantic diff 표시 문자열 = registry 이름, 묶음은 longhand 목록).
- `docs/design/property_model.md` — 5장에 `width`·`height`(0x0108·0x0109)와 모서리 longhand(0x010a\~0x010d) 행,
  9장 Mask 상태 `approximated`, Shadow 범위 0x1091\~0x1095, registry 이름 규칙 문단.

## 구현 요약

- 모든 Frame·Modifier 항목이 `CssMapping`을 가집니다. `clipping`은 CSS longhand로 나누지 않는 Stellar 단일
  속성이고 상태 `converted`·`initial-differs`이며 변형별 규칙을 가집니다. clip 기준(border box와 바깥 corner
  radius)은 차이 설명에 적었습니다. Mask는 `approximated`입니다.
- 묶음 key 표시 문자열은 longhand 목록입니다(`width, height`, `shadow-offset-x, shadow-offset-y`).
- 이름은 식별자가 아니며 저장 bytes·PropertyKey 숫자는 바뀌지 않습니다. 이름을 쓰던 곳은 test와
  `PropertyNameRegistry` 등록 함수뿐이었습니다(코드 검색으로 확인).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib` | 379 passed |
| `cargo test -p stellar-semantic-diff --lib keys` | 7 passed |
| `cargo test -p stellar-commands --lib` | 126 passed, 4 ignored |
| `cargo clippy -p stellar-document -p stellar-commands -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 매핑 누락 0(모든 항목에 상태, `stellar-only`가 아니면 CSS 대상, 모든 longhand가 inventory에 있음) | 통과 |
| registry 이름 = CSS longhand·shorthand 또는 Stellar 이름(허용 목록·`stellar-only`·묶음) | 통과 |
| 속성 모델 표(5장·9장) 상태 열과 registry 상태 일치, registry key가 모두 표에 있음 | 통과 |
| registry snapshot golden | 통과 |
| PropertyKey 숫자 불변(key 중복 0, 기존 key 값 그대로) | 통과 |
| storage golden 불변 | 저장 코드 변경 없음(lane 최종 check-core에 포함) |

## 차이(작업 지시와 다른 점)

- registry 이름 변경에 맞춰 `PropertyNameRegistry` 등록 test의 기대 이름을 바꿨습니다. 이름은 범위(Frame·Modifier)마다 유일하며, 두 범위를 한 이름 registry에 함께 등록하면 `transform`처럼 같은 이름이 생깁니다(제품 코드에서 함께 등록하는 곳 없음).

## 남은 위험 / 한계

- `stellar query`의 속성 경로 표기는 여전히 숫자 key(`crates/ops/src/query.rs`, 이 lane 소유 밖)입니다. 이름 표기가
  필요하면 `stellar_semantic_diff::keys::display_name`을 쓰면 됩니다. 이 lane은 이름을 내는 CLI를 `stellar css`
  (IX.11.15)로 제공합니다.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(IX.11.2)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `59479ff0` Feat: Frame·Modifier registry에 CSS 매핑과 CSS 이름 채움
