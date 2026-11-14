# 작업 보고서: X.11.2 nested component·variant·token 조건 layout fixture 20개

- WBS: X.11.2
- 작업명: nested component, variant, token 조건이 포함된 layout fixture 20개 추가(expected rect golden)
- 상태: 부분 완료 (`[~]`) — 리뷰 상태 정정(아래 "상태 정정"). 테스트 모델 기준 expected rect golden은 통과했지만, 실제 문서
  resolver 결과와 같은지는 확인하지 않았습니다.
- 선행: IX.9(절 단위, IX.9.1~IX.9.4 모두 `[ ]`), X.7(절 단위)
  - **완화**: 절 단위 선행이므로 필요한 코드가 있으면 완화합니다. IX.9 통합 fixture는 미완이지만 이 행이 쓰는 계약 —
    instance expansion(IX.2.2 `[x]`), variant override 우선순위(IX.7.6 `[x]`), token 모델·평가(VIII.5.1~VIII.5.4 `[x]`) —
    과 layout tree view(X.1.2, 이 lane `ec1107c`)가 있습니다. X.7은 X.7.1~X.7.6 모두 `[x]`(X.7.1·X.7.3·X.7.4·X.7.6은 이 lane).

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/tests/component_layout_corpus.rs`(신규) — fixture 모델(token 표, component 정의 4종, scene 5개),
  `LayoutSource` 구현, golden 비교·커버리지·증분 전환·가상 id 안정성 테스트.
- `crates/layout/tests/fixtures/component_layout_v1.golden`(신규) — fixture 20개의 모든 box rect·baseline(564줄).
- `docs/task-reports/X.11.2.md`(신규), 계획서(X.11.2 `[ ]`→`[x]`).

## 구현 요약

- **crate 경계**: layout crate는 문서·component·token crate에 의존하지 않으므로, 같은 계약을 따르는 작은 fixture 모델을
  `LayoutSource`(X.1.2)로 구현해 view로 펼친 뒤 레이아웃합니다.
- **nested component**: Button(icon + label), Toolbar(Fragment로 묶은 Button 둘 + title + icon-only Button),
  Card(Toolbar header + body + Button 둘), ListItem(avatar + text + small Button). Card → Toolbar → Button 3단계 중첩이며
  가상 node id는 `virtual_layout_id(chain, 정의 node)`입니다.
- **property·drilling**: 인스턴스 override와 `$name` drilling(Card title → Toolbar title, ListItem actionIconOnly → Button
  variant, Card의 menuSize → Toolbar menu Button variant).
- **variant**(IX.7.6 규칙): Button `size`(small·large)·`kind`(icon-only) nullable 차원. 일반 variant가 base를 덮고 구체
  조합(large + icon-only)이 일반 variant를 덮으며, 없는 조합(small + icon-only)은 base + 일반 variant만 적용합니다.
- **token 조건**: density(comfortable·compact) × scale(regular·large) 4조건에 따라 간격·control·icon·글자 token 값이 다릅니다.
- **fixture**: scene 5개(button, toolbar, card, list, dashboard-Grid) × 조건 4개 = 20개. golden은 box마다 원천 경로·rect·
  baseline을 1/64 px 정확값으로 고정합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | unit 171 + `component_layout_corpus` 4 + `layout_corpus` 2 + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **expected rect golden**: fixture 20개(box 11~45개, 진단 0)의 모든 rect·baseline이 golden과 같고, 재계산 결정적, 20개 결과가 서로 다름(`twenty_component_variant_token_fixtures_match_expected_rect_golden`) | 통과 |
| 3단계 중첩 virtual node, icon-only label 숨김, drilling된 menuSize=large의 구체 조합 padding, small 일반 variant 높이, title drilling, Fragment box 없음(`fixtures_cover_nesting_variants_and_tokens`) | 통과 |
| scene마다 조건을 바꾸는 node edit를 증분 레이아웃으로 적용 → 전체 계산과 bytes 동일, 전체 레이아웃 대체 0, unexpected fallback 0(`token_condition_switches_apply_incrementally`) | 통과 |
| 조건이 바뀌어도 가상 layout id 집합 동일(`virtual_ids_stay_stable_across_conditions`) | 통과 |

- golden 값 수동 확인(일부): button-comfortable `save` = 8 + 16 + 4 + 24 + 8 = 60 × 32, big-icon(large + icon-only 구체 조합
  padding 8) = 32 × 32, small-icon(일반 variant만, padding 2·높이 24) = 20 × 24, card 높이 = 16 + 40 + 8 + 16 + 8 + 36 + 16 =
  140, dashboard 2열 fr(900 − 16)/2 = 442 → 두 번째 열 x 458.

## 차이(작업 지시와 다른 점)

- fixture의 component·variant·token 해석은 문서 crate 구현이 아니라 같은 계약의 테스트 모델이 합니다(layout crate 경계).
  문서 crate의 실제 resolver와 같은 결과를 내는지는 adapter를 두는 상위 crate에서 이 fixture를 재사용해 비교해야 합니다.
- golden은 X.11.1처럼 digest가 아니라 box별 rect를 그대로 적었습니다(검증 기준이 "expected rect golden").

## 남은 위험 / 한계

- Toolbar root의 `Fill` 너비는 Hug 인스턴스 box 안이라 콘텐츠 크기로 해석됩니다(엔진 규칙대로이며 golden에 그대로 고정).
- 양 플랫폼 비교(X.11.3)는 Windows 보류(ADR-0006)로 이 행 범위 밖입니다.

## 상태 정정(리뷰, 2026-11-14)

- `[x]` → `[~]`. 근거:
  - 절 단위 선행 IX.9(IX.9.1\~IX.9.4)가 기준 commit `24728e3`에서 모두 `[ ]`입니다. 위 완화는 "필요한 코드가 있으면"을
    전제로 하지만, 이 행이 IX.9에서 받아야 할 것은 통합 component library·drilling·nullable variant fixture 자체이고 아직
    없습니다. 완화 근거로 든 IX.2.2·IX.7.6·VIII.5 코드도 이 fixture가 호출하지 않습니다(layout crate는 core-types만 의존).
  - fixture의 component 펼침·variant override·token 값은 `crates/layout/tests/component_layout_corpus.rs` 안의 테스트 모델이
    만듭니다. 검증 열 "expected rect golden 통과"는 이 모델의 출력에 대해서만 충족했고, 문서 crate의 실제 resolver가 같은
    layout 입력(따라서 같은 rect)을 내는지는 확인하지 않았습니다(위 "차이").
- 이미 있는 증거(유지): 테스트 모델 기준 fixture 20개 golden 일치, 증분 전환 = 전체 계산, 가상 layout id 안정성.
- `[x]` 조건: IX.9.1\~IX.9.3이 `[x]`가 되고, 상위 crate의 adapter(문서 → `LayoutSource`)로 같은 scene·조건을 실제
  resolver에서 펼쳐 이 golden(또는 IX.9 fixture에서 다시 만든 golden)과 같은 rect를 내는 테스트가 통과해야 합니다.
