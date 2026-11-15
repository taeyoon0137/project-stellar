# 작업 보고서: XXII.2.4 accessibility tree → HTML/ARIA 매핑

## WBS

- WBS: XXII.2.4
- 작업명: accessibility tree를 HTML/ARIA로 매핑
- 상태: 완료 (`[x]`)
- 선행: IX.8.6(기준 브랜치 `[x]`)

## 변경 파일

- `crates/codegen/src/web/mod.rs` (신규) — Web adapter module.
- `crates/codegen/src/web/html.rs` (신규) — 생성 HTML 구조 모델 `HtmlDocument`와 결정론 writer(재귀 없음).
- `crates/codegen/src/web/aria.rs` (신규) — `map_accessibility`: IR 접근성 tree → HTML/ARIA, 매핑 노트 `WebNote`.
- `crates/codegen/src/web/check.rs` (신규) — `check_document`: 정적 접근성 검사(`WebA11yIssue` 14종).
- `crates/codegen/src/web/tests.rs` (신규) — 검증 테스트.
- `crates/codegen/src/facet.rs` (신규) — adapter 노트 공용 `Facet`.
- `crates/codegen/src/lib.rs` (수정) — `facet`, `web` module.
- `crates/codegen/tests/fixtures/web_aria_site.html` (신규) — golden.
- `docs/guides/crate-map.md` (수정) — codegen module 목록.
- `docs/task-reports/XXII.2.4.md`(신규), 계획서(XXII.2.4 `[ ]`→`[x]`)

## 구현 요약

- **매핑**(native HTML 우선, ARIA in HTML): generic `div`, group `div role=group`, button `button type=button`, textField
  `input type=text`(자식 있으면 `div role=textbox`), dialog `dialog open`, list `ul`/`div role=list`(선택 항목·다중 선택이면
  `listbox`), listItem `li`/`div`(listbox 안이면 `option`), radioGroup `div role=radiogroup`, radio
  `button role=radio`, staticText `span`, image `img alt`(이름을 정적으로 모르면 `div role=img`). Fragment는 element 없이
  자식을 부모에 붙입니다(layout flatten과 같음).
- **이름**: 상수·속성·상태 초기값 → `aria-label`(`img`는 `alt`), 다른 node 텍스트 → `aria-labelledby`, 자기 텍스트는 내용.
- **상태**: focusable → `tabindex="0"`(native control 제외), focused → `autofocus`, disabled → `disabled`/`aria-disabled`,
  pressed → `aria-pressed`, selected → `aria-selected`(option), checked → `aria-checked`(radio), editable=false →
  `readonly`/`aria-readonly`, modal → `aria-modal`, multiselectable → `aria-multiselectable`. 상태 변수 binding은 초기값을
  쓰고 `DynamicBinding` 노트를 남깁니다(SelectionMatches는 상태 초기값 = item StableId 문자열).
- **숨김**: 접근성 entry가 없는 subtree의 맨 위 element에 `aria-hidden="true"`(이미지는 `alt=""`도).
- **읽기 순서**: entry 자식의 읽기 순서가 DOM 순서와 다르거나 DOM 하위가 아니면 부모에 `aria-owns`(읽기 순서). 참조되는
  element에만 `id="s-<StableId 16진>"`.
- **정적 검사**: 알 수 없는 role, tag에 허용되지 않은 role, role이 지원하지 않는 `aria-*`·이름 금지 role의 이름·tag에 없는
  HTML 속성, 잘못된 값, radio `aria-checked` 누락, 이름 필요 role의 이름 없음(aria-labelledby·aria-label·alt·내용 계산),
  깨진 id 참조·id 중복, option·listitem·radio의 필수 문맥, listbox·list·radiogroup의 필수 소유, `button` 안 interactive,
  `aria-hidden` 안 focusable, 이중 소유, 소유 순환.

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-codegen` | 0 (22 passed) |
| `cargo test -p stellar-codegen --doc` | 0 |
| `cargo fmt -p stellar-codegen -- --check` | 0 |
| `cargo clippy -p stellar-codegen --all-targets -- -D warnings` | 0 |
| `cargo xtask validate-task-report docs/task-reports/XXII.2.4.md` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC만이라 없음

## 테스트

- `site_profile_maps_to_golden_html_and_passes_static_check`: fixture View의 Web profile IR 매핑이 golden
  `web_aria_site.html`과 같고 정적 검사 issue 0건, 상태 binding 노트 4건(검증 기준).
- `every_profile_ir_passes_static_check_and_is_deterministic`: Web·iOS·Android IR 모두 issue 0건, 두 번 매핑 결과 동일.
- `mapping_uses_native_semantics_reading_order_and_hidden_subtrees`, `disabled_and_listbox_mapping_follow_state_values`,
  `selectable_list_maps_to_listbox_with_options`: 역할·이름·상태·숨김·`aria-owns` 읽기 순서, 비활성 초기값 true → `disabled`,
  선택 List → `ul role=listbox` + `li role=option aria-selected`.
- `static_check_detects_each_rule_violation`: 규칙별 위반 fixture 14건이 각각 기대 issue를 냄.
  `static_check_accepts_native_and_labelledby_names`: 올바른 문서 issue 0건.
- HTML writer 2건(속성 순서·escape·boolean 속성, 깊은 문서). 남은 경고 0.

## 성능

- 해당 없음.

## 차이

- 계획서상 HTML semantic structure 전체 매핑(XXII.2.1)은 이 행 범위 밖입니다. 이 행은 접근성 의미를 담는 element 선택과
  ARIA 속성까지이며, 시각 속성·layout CSS·Image 리소스는 쓰지 않습니다.
- "정적 접근성 검사"는 외부 도구(브라우저·axe 등) 대신 생성 HTML 구조 모델에 WAI-ARIA 1.2·ARIA in HTML 규칙 중 생성기가
  만드는 tag·role 범위를 이 crate 안에 구현해 검사했습니다(외부 의존 없음).
- 정적 HTML은 상태를 바꿀 수 없어 상태 변수 binding은 초기값과 노트로 표현했습니다. 실행 중 갱신은 Web 출력의 script 연결
  범위입니다.

## 남은 위험

- 정적 검사 규칙은 생성기가 쓰는 element·role 집합 중심이라 WAI-ARIA 전체를 검사하지 않습니다(예: `aria-describedby`
  내용, 색 대비, focus 순서의 시각 일치).
- 이름 계산은 aria-labelledby·aria-label·alt·내용만 보는 단순화입니다(CSS 생성 내용·`title` 미포함).

## 2026-11-15 재검증

- Codex 작업 검토(계획 B9)에서 이 행의 상태를 다시 정했습니다. 위 기록은 Codex 작업 시간에 작성·통합되어 상태의 근거로 쓰지 않고, 아래 실행 결과를 근거로 씁니다.
- 논리 검토: 계획 U023 수정. B1(`8e7ac57b`)에서 open dialog의 `aria-modal`을 빼고 목록 계산을 선형으로 고쳤습니다.
- 환경: `DEV-MAC`(Apple M3 Max, macOS 27.2, rustc 1.96.0, Apple Git 2.54.0). 코드 기준은 `3463761b`이며 이 검토의 문서 커밋은 코드를 바꾸지 않습니다.

| 명령 | 결과 |
|---|---|
| `cargo test --workspace --no-fail-fast` | 0 — 4,039 통과, 0 실패, 10 ignored(수동 측정·장기 fuzz) |

- 검증 기준(정적 접근성 검사 fixture): `web::tests::static_check_detects_each_rule_violation`, `site_profile_maps_to_golden_html_and_passes_static_check`, `every_profile_ir_passes_static_check_and_is_deterministic`, `fragment_is_transparent_for_hidden_subtrees` 통과.
- 판정: `[x]` 유지(계획의 `[~]` 뒤 검증 조건을 충족).
