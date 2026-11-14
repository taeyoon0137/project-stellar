# 작업 보고서: X.7.3 layout mode별 dirty ancestor 전파 최소화

- WBS: X.7.3
- 작업명: dirty node의 ancestor propagation을 layout mode별로 최소화(증분 레이아웃)
- 상태: 완료 (`[x]`)
- 선행: X.7.1(`[x]`, 이 lane `a7f6960`) — 명시 행 선행 충족, 완화 없음

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/incremental.rs`(신규) — `IncrementalLayout`(node 단위 입력·마지막 배치 보관), `LayoutEdit`,
  `UpdateStats`, batch 검증(`stage`/`validate_structure`), 반영(`commit`), mode별 전파(`propagate`), 대상 처리·subtree
  재사용 배치(`apply_target`/`rearrange`), overflow 조상 갱신, 테스트.
- `crates/layout/src/engine.rs` — `Engine`이 tree·변환 입력·subtree hash를 빌려 쓰도록 변경(세션과 공유),
  `arrange_own`(node 자신 배치 단계) 분리, `with_layout_stack`(깊은 tree 전용 stack 실행) 공통화, `prepare`/
  `prepare_all`/`content_source`/`child_link` crate 공개. 전체 계산 결과는 변경 없음.
- `crates/layout/src/measure.rs` — subtree hash를 node 자신 hash + 자식 subtree hash 합성으로 바꿈(`node_hash`,
  `combine_subtree`, `subtree_hashes_from`; 바뀐 node·조상만 다시 합성 가능). `MeasureCache`를 node별 한 버전 보관으로
  바꾸고(다른 hash 저장 시 이전 결과 제거) `remove_node` 추가.
- `crates/layout/src/free.rs`, `crates/layout/src/overlay.rs` — 자식 하나의 rect를 구하는 `child_rect` 분리(배치 결과 동일).
- `crates/layout/src/input.rs` — `validate_node_values` crate 공개, `validate_in_parent`(부모 배치 방식 조합 검증) 분리,
  `TreeIndex.depth`를 증분 경로에서 상한값으로 쓰는 문서.
- `crates/layout/src/error.rs` — `LayoutError::UnknownNode`, `LayoutError::LayoutUnavailable`.
- `crates/layout/src/lib.rs` — `incremental` 모듈 등록.
- `docs/task-reports/X.7.3.md`(신규), 계획서(X.7.3 `[ ]`→`[x]`).

## 구현 요약

- **dirty class**: edit마다 `changed_properties`(X.7.1)와 변경 전·후 상태로 class를 구합니다. PaintOnly만 바뀐 node(예:
  transform)는 배치하지 않습니다.
- **mode별 전파**(`propagate`): dirty node는 항상 다시 배치합니다. 변경이 부모에 보이지 않으면(DescendantConstraint만,
  부모가 baseline을 읽는 Constraint가 아님) 그 node가 relayout root입니다. SelfSize·ParentFlow면 부모가 다시 놓습니다 —
  **Free·Overlay**(두 축 Fixed) 부모는 형제와 무관하므로 그 자식 하나만 다시 놓고(부모 미방문), **Stack·Grid·Constraint**는
  부모 자식 배치를 다시 합니다. 부모 크기가 콘텐츠에 의존할 수 있으면 한 단계 더 올라가고, 두 축 Fixed 부모에서 멈춥니다.
  root에 닿으면 root 크기를 다시 해석합니다. 숨긴 node의 크기 변화는 숨긴 node에서 멈춥니다.
- **subtree 재사용**: 다시 배치하는 node의 자식이 stale이 아니고 새 rect 크기·HugContent 순환 link가 이전과 같으면 자손을
  방문하지 않고 rect만 갱신합니다(local 좌표). 측정은 subtree hash key 측정 cache를 계속 씁니다. 다시 배치한 node의 overflow가
  바뀌면 조상 overflow만 다시 합칩니다. 진단은 node별(자신 순환 진단 + 자식 배치 진단)로 보관해 pre-order로 합치므로 전체
  계산과 순서까지 같습니다.
- **숨긴 subtree**: 표시되지 않는 대상은 배치하지 않고 가장 위 숨김 조상까지 stale로 남겨, 다시 보일 때 그 경로를 따라
  배치합니다.
- **구조 edit 검증**: 적용 전에 값·부모 조합·자식 참조·부모 하나·순환·도달성·깊이를 바뀐 관계와 옮겨진 subtree 크기에 비례해
  검사하고, 실패하면 상태를 바꾸지 않습니다. 삭제는 swap_remove와 index 보정으로 반영하고, 바뀐 node·조상 subtree hash만 깊은
  쪽부터 다시 합성합니다. 계산 중 오류는 결과를 무효(`LayoutUnavailable`)로 두고 다음 edit가 전체 계산으로 복구합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | unit 163 + `layout_corpus` 2 + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |
| (일회성 stress, 커밋 안 함) `cargo test --release -p stellar-layout random_edits -- --nocapture`를 corpus 250개 × 120 step으로 늘려 seed 2개 실행 | 적용 24,996/25,334회·거부 4,403/4,256회 모두 전체 계산과 bytes 동일, 실패 0 |

| 검사 | 결과 |
|---|---|
| **leaf color 변경 0 layout**: X.7.1 변환으로 색 변경 → layout queue 비어 다시 읽을 node 없음 → 빈 batch 집계 전부 0, 방문 0, 결과 불변(`leaf_color_change_does_zero_layout`). transform edit도 방문 0·측정 0·paint_only 1 | 통과 |
| **leaf width 변경 예상 ancestor만 방문**(배치 순서까지 일치, root 방문 0): Stack leaf → `[A, B, L]`, Free leaf → `[M]`, Free 안 Hug container leaf → `[N, 12]`, Overlay leaf → `[P]`(형제 미방문), Grid item → `[G, 16]`, Constraint → `[K, 19]`, Fixed leaf padding → `[L]`, Fixed Stack padding → `[A]`, Free 위치 → `[N]`(`single_edits_visit_only_expected_nodes_per_layout_mode`) | 통과 |
| Hug 사슬은 root까지 올라가 root 크기 재해석(`hug_chain_reaches_root_and_resizes_it`) | 통과 |
| 숨긴 subtree 안 변경은 방문 0, 다시 보일 때 stale 경로 `[A, B, S, 6]`만 배치(`hidden_subtree_edits_are_deferred_until_shown`) | 통과 |
| 삽입·이동·subtree 삭제 후 결과 동일, 삭제 뒤 swap_remove index 보정(`structure_edits_insert_move_and_remove`) | 통과 |
| 잘못된 batch 12종(부모 둘, 없는 자식, root 자식, 도달 불가, 부모가 가진 node 삭제, root·없는 node 삭제, 미연결 새 node, 새 node 순환, root Fill, 깊이 초과)이 상태를 바꾸지 않음(`invalid_batches_leave_the_session_unchanged`) | 통과 |
| 초기 레이아웃 = 전체 계산(corpus 100개), 무작위 edit(corpus 60개 × 24 step)마다 전체 계산과 bytes 동일·잘못된 batch 거부·상태 불변·전체 레이아웃 대체 0(`initial_layout_matches_full_compute_for_corpus`, `random_edits_match_full_compute_and_reject_invalid_batches`) | 통과 |
| 측정기 revision 변경 → 전체 재배치 1회(`measurer_revision_change_relayouts_everything`), 깊이 256 사슬 증분 edit가 2 MiB thread에서 끝남(`deep_chain_updates_on_default_thread_stack`) | 통과 |
| 기존 측정 cache·corpus golden(X.7.2, X.11.1) | 통과(결과 변경 없음) |

## 차이(작업 지시와 다른 점)

- "콘텐츠와 무관한 부모 크기"는 보수적으로 두 축 `Fixed`로 판정합니다(`size_may_depend_on_content`). `Fill`·`Percentage`가
  definite 부모 안에서 실제로는 고정인 경우에도 한 단계 더 올라갑니다(과다 방문만 허용, 과소 방문 없음).
- 증분 상태는 문서·command가 아니라 node 단위 레이아웃 입력 edit를 받습니다(layout crate 경계). X.7.1 layout queue의 node를
  X.1.2 view로 다시 읽어 edit로 넘기는 연결은 상위 crate 몫입니다.

## 남은 위험 / 한계

- 구조 edit 검증은 바뀐 관계에 비례하지만, 한 batch에 같은 부모를 많이 바꾸면 자식 목록 비교 비용이 그만큼 듭니다.
- `TreeIndex.depth`는 증분 경로에서 줄어들지 않는 상한입니다(전용 stack thread 선택에만 쓰임).
- 전체 레이아웃 대체 경로의 debug assertion·metric(X.7.6)과 20만 node benchmark(X.7.4)는 후속 행입니다.
