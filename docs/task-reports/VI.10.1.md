# 작업 보고서: VI.10.1 전체 document validator

- WBS: VI.10.1
- 작업명: 전체 document validator — ID uniqueness, parent consistency, root count, OrderKey
  uniqueness, ref type, modifier cycle, depth≤64
- 상태: 완료 (`[x]`)
- 선행: VI.1~VI.9

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/records.rs`(신규) — 영속 레코드 `NodePayload`, `NodeRecord`,
  `DocumentRecords`(runtime 핸들 없이 StableId만 사용, kind는 payload에서 유도).
- `crates/document/src/validate.rs`(신규) — `validate_document`, `DocumentViolation`,
  `ViolationLocation`, `RefExpectation`, `RefTarget`, `DocumentValidationError`,
  `DocumentDiagnostics`, `MAX_NODE_DEPTH`(64), `can_have_children`.
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.10.1.md`(신규), 계획서(VI.10.1 `[ ]`→`[x]`).

## 구현 요약

- **입력**: `DocumentRecords` = 문서 정의(VI.1) + node 레코드(부모는 StableId) + 분리 저장된
  modifier stack(VI.9.1). 저장 전 검증과 loader(VI.10.3)의 로드 후 검증이 같은 함수를 씁니다.
- **보고 방식**: 첫 위반에서 멈추지 않고 모든 위반을 결정론적 순서로 모읍니다. 각 위반은
  `location()`으로 정확한 위치를 줍니다 — node는 레코드 index+StableId(id가 중복·NIL이어도
  식별 가능), modifier는 (소유 node, ModifierId), 속성은 `PropertyPath`(node payload 속성은
  node root, modifier 속성은 modifier id root + key + 선택 index).
- **불변식**:
  1. ID uniqueness — 문서·페이지·View·node·modifier·path point/handle id 전역 유일, NIL 금지.
  2. root count — View root 존재·Frame·부모 없음, View 간 공유 금지, 부모 없는 node는 반드시
     어떤 View의 root(View당 root 하나).
  3. parent consistency — 부모 존재, 부모가 Frame·Fragment(§12.3 Children, §12.4 자식 순서),
     부모 chain cycle 금지(cycle의 최소 StableId node에 경로와 함께 보고).
  4. OrderKey uniqueness — 같은 부모의 형제끼리 key 중복 금지(나중 형제에 보고).
  5. ref type — Frame clip은 Path node, modifier(Boolean·Mask·Clip) 참조는 node여야 하며 다른
     종류(View·modifier 등)를 가리키면 위반. 대상이 없거나 NIL이면 위반이 아니라 미해결 진단
     (`DocumentDiagnostics::unresolved`, III.2.4/VI.3.4와 같은 정책).
  6. modifier — stack 소유 node 존재·유일, Fragment modifier 금지(VI.9.6, 항목별 보고),
     `find_modifier_cycle`(VI.9.5) 재사용.
  7. depth — View root=0 기준 64까지 허용, 처음 한도를 넘은 node만 보고.
- 중복 id가 자기 자신을 부모로 지정하는 입력에서도 depth·cycle 검사가 종료하도록 레코드
  index로 방문을 표시합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 164 passed + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사(각 불변식 위반 fixture가 정확한 위치 오류) | 결과 |
|---|---|
| 유효 fixture(모든 node 종류·View 2개·modifier 참조) 통과(`valid_fixture_passes_without_diagnostics`) | 통과 |
| node 중복·View↔node 충돌·modifier↔path point 충돌·NIL id, 두 위치 정확(`duplicate_and_nil_ids_report_exact_locations`) | 통과 |
| root 없음·root 둘·root가 Frame 아님·root에 부모·root 공유(`root_count_violations_report_view_and_node`) | 통과 |
| 부모 없음·leaf 부모·2-cycle·self parent(`parent_consistency_violations_report_child_node`) | 통과 |
| 형제 OrderKey 중복은 나중 형제에 보고, 다른 부모는 허용(`duplicate_sibling_order_key_reports_later_sibling`) | 통과 |
| clip→Text, Boolean operand→View의 속성 경로·기대/실제 종류(`ref_type_mismatch_reports_property_location`) | 통과 |
| modifier cycle 경로, Fragment modifier 항목별 위치(`modifier_cycle_and_fragment_modifier_report_locations`) | 통과 |
| stack 소유 node 없음·중복 stack(`modifier_stack_owner_violations`) | 통과 |
| depth 64 허용, 66 chain은 depth 65 node 하나만 보고(`depth_limit_reports_first_node_over_limit`) | 통과 |
| 없는/NIL 참조는 진단으로만 남음(`missing_reference_targets_are_diagnostics_not_violations`), 결정론·Display, 중복 id 무한 루프 없음 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 다만 "parent consistency"에 부모 종류 제한(Frame·Fragment만 자식 허용)을 포함했습니다
  — 기술 아키텍처 §12.3(Frame Children capability)·§12.4(Fragment 자식 순서)에 근거한 해석입니다.

## 남은 위험 / 한계

- ComponentInstance는 payload 스키마(Phase IX) 전이라 레코드로 표현할 수 없고 따라서 검증
  대상이 아닙니다. Phase IX에서 payload variant와 정의 참조 ref type 검사를 추가해야 합니다.
- Frame `layout_mode`(layout 정의 참조)와 Image asset·Text style 표 참조는 문서 밖 resource라
  여기서 검사하지 않습니다(asset 누락은 VI.8.2 진단 경로).
- 검사 비용은 O(N log N)입니다. 100만 node 규모 성능 측정은 XVI.3에서 수행합니다.
