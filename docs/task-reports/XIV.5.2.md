# 작업 보고서: XIV.5.2 변경 dependency graph

- WBS: XIV.5.2
- 작업명: change dependency graph — 새 object의 property, binding이 참조하는 token/property, component interface와 instance override 관계 포함
- 상태: 완료 (`[x]`)
- 선행: XIV.3.2(`[x]`), VI.2.5(`[x]`), VIII.6.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/dependency.rs`(신규) — `DependencyGraph`, `DependencyReason`, 참조 scan.
- `crates/semantic-diff/src/lib.rs` — `pub mod dependency;`.
- `docs/task-reports/XIV.5.2.md`(신규), 계획서(XIV.5.2 `[ ]`→`[x]`).

## 구현 요약

- `DependencyGraph::build(base, target, changes)`는 base → target diff의 변경 사이 의존(“a만 적용하면
  무결성이 깨지므로 b가 함께 필요”)을 `ChangeId` 간선과 이유로 만듭니다. 규칙(이유):
  - 새 parent(`ParentAdded`): 객체 추가·이동의 새 parent가 같은 diff에서 추가됨.
  - 참조 대상 추가(`ReferenceAdded`): 새 값·경로가 같은 diff에서 추가되는 객체(token·page·asset·node 등)를
    가리킴. 새 object의 modifier·expression node·View는 parent 규칙으로, 문서 페이지 목록 등은 이 규칙으로 연결됩니다.
  - 소속 목록(`Membership`): 새 Page·View·Slot → 문서 페이지·페이지 View·컴포넌트 Slot 목록 변경, 새 root
    Frame → View root·컴포넌트 root 참조, 목록에서 빠지는 객체 → 그 객체 삭제.
  - 자식 정리(`ChildDetached`), 참조 해제(`ReferenceReleased`): 객체 삭제 → base 자식의 삭제·이동, base에서 그
    객체를 가리키던 속성 변경(또는 참조 객체 삭제).
  - 교체(`Replacement`): 같은 StableId 삭제 ↔ 추가.
  - interface member(`InterfaceMemberAdded`·`InterfaceMemberReleased`): 컴포넌트 Property·Slot을 가리키는
    instance override 경로·binding payload → 그 member를 interface 목록에 넣는 변경, 목록에서 빼는 변경 → 그
    member를 가리키던 override·binding 변경.
  - OrderKey 비움(`OrderKeyVacated`): 새 위치(parent, key)를 base에서 쓰던 형제의 이동·삭제(교환 reorder).
- **참조 찾기**: semantic-diff는 expressions·component 내부 형식에 의존하지 않으므로 값 canonical bytes와 경로
  key segment(`/`로 나눈 StableId 문자열, 16진수 bytes)에서 관심 id의 16바이트 창을 찾습니다(binding의 token·
  component Property 참조, override 경로의 Property id, Variant patch 대상). 일치는 의존을 더하는 방향이라
  무결성을 해치지 않습니다.
- `closure`는 반복 worklist, `select`는 graph에 없는 ID를 `UnknownChange`로 거부하고 `StageSelection`을
  만듭니다. 결과는 `ChangeId` 순 BTreeMap으로 입력 순서와 무관합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | unit 62 passed, fuzz 통합 4 passed, 0 failed |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **새 object fixture**: 새 페이지 4·View 5·root frame 20·자식 21·modifier 60 — 각 간선과 이유 정확, modifier 하나 선택 closure = 페이지 목록 변경 + 새 subtree 전체(`new_objects_depend_on_parents_lists_and_roots`) | 통과 |
| **binding·component fixture**: 새 binding → 새 token, binding payload 변경 → 새 token, token 삭제 → 참조하던 payload 변경, 새 override·Property 참조 binding → interface member 추가 변경, interface member 제거 → override 제거; 의존 없는 변경은 빈 목록(`binding_token_and_component_interface_dependencies`) | 통과 |
| **삭제·교체·순서**: parent 삭제 → 자식 삭제, 교체 상호 의존, 삭제된 형제 key 재사용 추가 → 그 삭제, key 교환 reorder 상호 의존, closure의 required additions, 없는 ID 거부(`removals_replacements_and_order_keys`) | 통과 |
| 독립 변경(부분 staged fixture)은 간선 0, 입력 역순에도 같은 graph(`independent_changes_have_no_dependencies_and_graph_is_deterministic`) | 통과 |
| 참조 scan: 값 bytes·nested override segment·Variant 16진수 segment 검출, 무관 값 0(`reference_scan_reads_values_and_path_segments`) | 통과 |

## 차이(작업 지시와 다른 점)

- 새 object의 property는 semantic diff에서 AddObject에 포함되므로(XIV.3.2) 별도 property 변경이 없습니다. 대신
  새 object에 딸린 객체(modifier·expression node·View)는 새 parent 규칙, 새 object를 가리키는 기존 속성은 참조
  대상 추가 규칙으로 연결했습니다.
- binding 대상이 component Property인 경우는 Property id가 객체가 아니라 interface 목록 member이므로
  interface member 규칙으로 처리했습니다.

## 남은 위험 / 한계

- 참조 scan은 의존을 과대 추정할 수 있습니다(우연한 16바이트 일치 — 사실상 0). 과소 추정 방향(참조를 놓침)은
  StableId를 원시 16바이트·문자열·16진수 이외 형태로 담는 미래 형식에서 생길 수 있어, 새 값 형식이 생기면 이
  scan 규칙을 함께 갱신해야 합니다.
- base 전체를 scan하는 참조 해제 규칙은 삭제가 있을 때 base 값 bytes 총량에 선형입니다.

## 후속 수정 (리뷰 반영)

- **새 root Frame 소속 규칙의 이중 반복**: parent 없는 새 Node마다 모든 변경을 `introduces_root`로 훑어 (새 root
  수 × 변경 수 × 속성 수) 비용이었고, 모듈 문서의 "선형" 설명과 맞지 않았습니다. 변경 목록을 한 번 scan해 root id
  → root 참조를 도입하는 변경 index(`introduced_roots`)를 만들고 조회하도록 바꿨습니다. 의존 결과는 같습니다.
  모듈 문서도 실제 비용(BTreeMap 조회 log 인자 제외 선형)으로 고쳤습니다.
- 성능 지적이라 기능 실패 테스트로 재현할 수 없었습니다. 동작 보존 회귀 테스트
  `many_new_root_frames_each_depend_only_on_their_introducer`(새 View 40개와 각 root frame, 기존 View 3의 root
  교체)를 먼저 추가해 수정 전후 모두 통과함을 확인했습니다. 커밋: `Fix: 새 root Frame의 소속 dependency를 변경
  목록 한 번 scan한 index로 찾음`.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | unit 78 passed + fuzz 통합 4 passed, 0 failed |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |
