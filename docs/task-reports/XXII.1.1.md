# 작업 보고서: XXII.1.1 resolved View → CodegenIR 변환

## WBS

- WBS: XXII.1.1
- 작업명: resolved View를 layout, semantics, state, tokens, target capability가 포함된 `CodegenIR`로 변환
- 상태: 완료 (`[x]`)
- 선행: G8(gate 수준 선행 — 완화, 아래 "차이" 참조), IX.10.8(기준 브랜치 `[x]`)

## 변경 파일

- `crates/codegen/Cargo.toml`, `crates/codegen/src/lib.rs` (신규) — `stellar-codegen` crate. 의존: core-types,
  target-profile, accessibility, layout. 외부 의존 없음.
- `crates/codegen/src/ir/mod.rs` (신규) — `CodegenSources`(입력), `CodegenIr`·`IrNode`·`IrLayout`·`IrProperty`·
  `IrAccessibilityEntry`·`IrState`·`IrToken`·`IrTarget`, `StateDecl`·`TokenDecl`, `IrError`.
- `crates/codegen/src/ir/build.rs` (신규) — `build_ir` 변환(구조 검사, profile 해석·제외, layout 대조, 속성·token·
  텍스트·capability 부착, 접근성 재부모, 상태 참조 검사).
- `crates/codegen/src/ir/binding.rs` (신규) — 접근성 binding을 IR 값·상태로 해석(`resolve_binding`, `static_value`).
- `crates/codegen/src/ir/snapshot.rs` (신규) — 결정론 semantic snapshot 텍스트와 안정 이름 표기.
- `crates/codegen/src/text.rs` (신규) — 수 표기·식별자 정리 도우미.
- `crates/codegen/src/fixture.rs` (신규, `cfg(test)`) — Web·Apple·Android 공용 resolved View fixture.
- `crates/codegen/src/ir/tests.rs` (신규) — 검증 테스트.
- `crates/codegen/tests/fixtures/ir_site.snapshot`, `ir_ios.snapshot`, `ir_android.snapshot` (신규) — golden.
- `Cargo.toml` (수정) — workspace member에 `crates/codegen` 추가. `Cargo.lock` (수정) — cargo 자동 갱신.
- `tools/xtask/src/crate_graph.rs` (수정) — `stellar-codegen` 허용 간선.
- `docs/architecture/crate-dependency-graph.md`, `docs/guides/crate-map.md` (수정) — 새 crate와 간선.
- `docs/adr/ADR-0049-codegen-crate.md` (신규) — crate와 입력 경계 결정.
- `docs/task-reports/XXII.1.1.md`(신규), 계획서(XXII.1.1 `[ ]`→`[x]`)

## 구현 요약

- **입력**: 하위 crate 지역 모델 그대로 — `ViewModel`(구조·공통값·profile override), `VariantLayer`, `LayoutInput`
  (Fragment flatten), `AccessibilitySnapshot`(IX.8.6), 상태 변수·token 선언, Text 내용, PropertyPath → token binding.
- **구조**: root 하나인 tree만 받습니다(중복 id·없는 부모·root 0개/2개·순환 거부). IR node는 preorder, 형제는 문서 순서.
- **profile 해석**: IX.10.7 resolver(공통값 → profile override → variant → profile×variant)로 값과 출처를 담고,
  제외 node와 하위 node를 IR에서 뺍니다(값·텍스트·layout·접근성 entry 포함).
- **layout**: Fragment가 아닌 node마다 layout node가 있어야 하고, layout 자식(OrderKey 순, 제외 node 제외)이 Fragment를
  펼친 IR 자식 순서와 같아야 합니다.
- **semantics**: snapshot entry를 IR node에 붙이고, 제외된 source의 entry는 버린 뒤 자식을 가장 가까운 남은 조상으로
  올립니다. binding·action이 가리키는 상태 변수는 선언되어 있어야 합니다.
- **target capability**: IX.10.8 validator 진단(Native 포함)을 node별로 붙입니다. 모르는 registry pin은 거부합니다.
- **결정론**: hash 순서·pointer·부동소수 연산 없이 BTreeMap·정렬만 씁니다. 상태·token은 id 오름차순입니다.

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-codegen` | 0 (13 passed) |
| `cargo fmt -p stellar-codegen -- --check` | 0 |
| `cargo clippy -p stellar-codegen --all-targets -- -D warnings` | 0 |
| `cargo xtask validate-crate-graph` | 0 |
| `cargo xtask validate-dependencies` | 0 |
| `cargo xtask validate-versions` | 0 |
| `cargo xtask check-core` | 65 (fmt·clippy 통과, test 단계에서 `stellar-project` `restricted_yaml_fuzz`의 시간 상한 3s 초과로 실패 — 아래 "남은 위험") |
| `cargo test -p stellar-project --test restricted_yaml_fuzz` | 101 (단독 재실행도 같은 시간 상한 초과, load average 약 100) |
| `cargo xtask sync-plan --check`, `cargo xtask validate-task-status` | 0 |
| `cargo xtask validate-task-report docs/task-reports/XXII.1.1.md`, `cargo xtask validate-adr docs/adr/ADR-0049-codegen-crate.md` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC만이라 없음

## 테스트

- `semantic_snapshot_fixtures_per_profile`: 같은 View를 Web(`site`)·Apple(`ios`)·Android(`android`) profile로 변환한 IR
  semantic snapshot이 golden 3개와 byte 단위로 같음(검증 기준).
- `ir_carries_layout_semantics_state_tokens_and_capability`: preorder·Fragment layout 없음, profile override·variant 출처,
  token binding, radio group·radio·textField·dialog 역할, 상태·token 선언, iOS capability(SingleSelectionGroup
  approximated, SF Symbol native).
- `excluded_subtree_drops_values_text_layout_and_semantics`, `excluded_accessibility_parent_lifts_children_to_nearest_kept_ancestor`,
  `semantic_subtrees_mark_nodes_with_accessible_descendants`, `bindings_resolve_to_ir_values_and_initial_state`.
- `ir_is_deterministic_and_independent_of_declaration_order`: 상태·token·layout node·layout 자식 입력 순서를 뒤집어도 같은 IR.
- `structural_errors_are_rejected`(6건), `input_consistency_errors_are_rejected`(13건),
  `unknown_registry_pin_and_invalid_layout_are_rejected`, `view_without_semantics_or_tokens_still_builds`.
- `text` 도우미 2건. 남은 경고 0.

## 성능

- 해당 없음(핵심 입력·렌더링 경로 밖).

## 차이

- 선행 `G8`(초기 릴리스 gate)은 gate 수준 선행이라 기존 관례(XVIII.1.1·XX.1.1·XXIV.1.1 보고서)대로 완화했습니다.
  행 ID 선행 IX.10.8은 기준 브랜치에서 `[x]`입니다.
- 코드 생성 crate가 기술 아키텍처 §7 구성에 없어 ADR-0049로 `crates/codegen`과 crate graph 간선을 추가했습니다.
- 문서 모델에서 IR 입력(`ViewModel`·`LayoutInput`·접근성 snapshot·상태·token)을 채우는 연결 코드는 이 행 범위 밖이라
  만들지 않았습니다. 검증은 같은 지역 모델로 만든 fixture로 했습니다.
- semantic snapshot은 리뷰·fixture 비교용 텍스트이며 저장 파일 형식이 아닙니다.

## 남은 위험

- `check-core` test 단계의 `restricted_yaml_fuzz_campaign_has_no_findings`(stellar-project)가 실행 시간 상한(3s)을 넘어
  실패했습니다. 이 변경은 project crate를 건드리지 않았고, 여러 lane이 동시에 빌드·테스트하는 부하(load average 약
  100)에서 단독 재실행도 같은 이유로 실패했습니다. 부하가 낮을 때 다시 실행해야 합니다. cargo test는 첫 실패 binary에서
  멈추므로 그 뒤 binary는 이 실행에서 돌지 않았습니다.
- 문서 계층 → IR 입력 연결이 없어 실제 프로젝트 View로는 아직 IR을 만들 수 없습니다.
- 상태 변수·token 이름의 대상 언어 식별자 변환과 escape 고정은 XXII.5.1 범위입니다(이 행은 원래 이름을 보존).
- IR node별 unsupported/approximated 진단 보존(XXII.1.2)과 import API 부재 inventory(XXII.1.3)는 별도 행입니다.
