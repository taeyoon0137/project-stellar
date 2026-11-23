# 작업 보고서: XXII.2.1 Frame·Fragment·Text·Image·Control의 HTML semantic 구조

## WBS

- WBS: XXII.2.1
- 작업명: Frame/Fragment/Text/Image/Control contract를 HTML semantic structure로 매핑
- 상태: 완료 보고(`reported_done`). 선행 XXII.1.1이 `[!]`이라 원장 판정은 통합 때 정합니다.
- 결정: ADR-0106 결정 4
- 선행: XXII.1.1

## 변경 파일

- `crates/codegen/src/web/structure.rs`(신규): `map_structure`(IR → `WebStructure`), golden용 `dom_tree_text`, test.
- `crates/codegen/src/web/aria.rs`: 직접 만들던 `div` tree 대신 구조 결과에서 시작합니다. 접근성 entry가 있는 element만
  role로 고치고, entry가 없는 element는 구조 tag를 그대로 둡니다.
- `crates/codegen/src/web/html.rs`: `clear_attributes`.
- `crates/codegen/src/web/mod.rs`: module 등록.
- `crates/codegen/tests/fixtures/structure_{site,list}.tree`(신규).

## 구현 요약

- Fragment가 아닌 IR node마다 element 하나, Fragment 자식은 가장 가까운 element 조상에 붙습니다.
- tag: Frame `div`, Text `span`(텍스트 내용), 자식 없는 Image `img alt=""`, Shape·Path·ComponentInstance `div`,
  Button `button type="button"`, 자식·텍스트 없는 TextInput `input type="text"`, Modal `dialog open`, contract 없는
  element 자식만 가진 List `ul`(자식 `li`), SingleSelectionGroup `div`. contract가 둘 이상이면 번호가 가장 작은
  contract가 정합니다.
- 같은 판정을 두 곳에 두지 않도록 접근성 mapper(XXII.2.4)가 구조 위에서만 동작하게 바꿨습니다. role이 있는
  element는 구조의 기본 속성을 버리고 role 규칙을 씁니다. 접근성 entry가 없는 숨김 subtree의 native control tag
  (`button`·`input`·`dialog`)는 focus를 받으면 안 되므로 `div`로 낮춥니다. 기존 XXII.2.4 golden HTML은 바뀌지
  않았습니다.

## 명령과 결과

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-codegen` | 통과(73 + 2), 기존 Web·Apple golden 변경 없음 |
| `cargo clippy -p stellar-codegen --all-targets -- -D warnings` | 경고 0 |

검증 기준 대응:

- golden DOM tree: `web::structure::tests::golden_dom_tree_maps_frame_fragment_text_image_and_controls`
  (`structure_site.tree`), `list_contract_with_plain_items_is_ul_li`(`structure_list.tree`), 결정론
  `structure_is_deterministic`.

## 남은 위험

- `dialog open`은 정적 표시입니다. modal 표시(`showModal()`)는 생성 script 범위입니다(XXII.2.4 노트와 같음).
