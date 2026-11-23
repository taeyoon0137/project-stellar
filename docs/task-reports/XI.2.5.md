# 작업 보고서: XI.2.5 dirty subtree만 재빌드하는 증분 scene build

- WBS: XI.2.5
- 작업명: dirty subtree만 재빌드하고 변경되지 않은 scene node/resource refs를 공유
- 상태: 완료 (`[x]`)
- 결정: ADR-0084 결정 3
- 선행: XI.1.4(`[x]`), XI.2.1(이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/scene/src/source.rs`(신규) — `SceneSource`, `SourceNode`, `SourceEdit`, 국소 불변식 검사와 되돌리기.
- `crates/scene/src/incremental.rs`(신규), `incremental/tests.rs`(신규) — `IncrementalScene`.
- `crates/scene/src/builder.rs` — z-order 건너뛰기·다음 값 hook(crate 내부).
- `crates/scene/src/spatial.rs` — `SceneSpatialIndex::refresh`(공유 subtree 건너뜀).
- `crates/runtime/src/canvas/scene_source.rs` — change set → subtree 단위 source 편집(`scene_edits`), 구조가 그대로면 색인 재사용.
- `docs/task-reports/XI.2.5.md`(신규).

## 구현 요약

- node마다 이전 결과·부모 scope(world·clip·숨김·잠금)·시작 z를 기억합니다. 자신·자손이 깨끗하고 scope·z가 같으면 `Arc`를 공유합니다. z만 다르면 content·paint·clip을 공유하고 z만 다시 매깁니다. dirty·scope 변경 node는 다시 평가합니다. 자손만 dirty면 path copying만 합니다.
- z-order는 기존 builder와 같은 pre-order 연속 번호입니다. 그래서 증분 결과의 semantic hash가 같은 source의 전체 build와 같습니다.
- source 편집은 바뀐 node·자식·조상 chain만 검사하고 실패하면 편집 전으로 되돌립니다. 공간 인덱스도 공유 subtree를 건너뛰며 갱신합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | lib 151 passed, 통합 3 passed(1 ignored), 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo test -p stellar-runtime --lib scene_source` | 9 passed, 0 failed(검토 보완 3개 포함) |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `single_paint_edit_rebuilds_one_node_and_shares_the_rest`: node 25개 중 rebuilt 1, copied 2, shared 22(subtree 7개), 새 할당 3, 바뀌지 않은 node `Arc`·paint 참조 동일, resource refs 동일, 전체 build와 hash 동일.
- Frame 이동(자식 5개 재평가), Frame paint(자식 공유), Subtree(자손 전부), 삽입(뒤쪽 z만 다시 매김, paint 공유)·삭제 후 원래 hash, 잘못된 편집 거부·상태 유지, Fragment 공유, 무작위 편집 38회 전체 build와 hash 동일.
- `spatial_index_refresh_matches_a_fresh_index`: paint 편집 뒤 갱신 node 3개, 삭제 뒤에도 새 인덱스와 같음.
- `change_set_drives_a_minimal_rebuild`(runtime): 실제 command → rebuilt 1·copied 1·shared 5, 이동 뒤 source와 전체 변환 동일.
- 검토 보완: `removing_a_node_its_parent_or_roots_still_list_is_rejected`는 부모·root가 아직 가리키는 node 삭제를 거부하는지 봅니다. runtime의 `operand_moved_by_an_ancestor_frame_updates_the_boolean_host`, `adding_a_boolean_modifier_absorbs_an_existing_node`, `a_node_that_becomes_unsupported_leaves_its_parent_list`는 증분 변환 결과(source·operand 색인·skip·scene hash)가 처음부터 변환한 결과와 같은지 봅니다.

## 성능

- 2만 가시 node 재생(release)에서 scene build p99는 0.6\~1.2ms입니다(§6.2 목표 1.00ms, 상한 1.75ms). 이동 preview는 node 1개만 다시 평가합니다.

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 삽입·삭제는 뒤쪽 node의 z를 다시 매기므로 O(뒤쪽 node 수)의 node 할당이 남습니다(ADR-0084 Consequences).
- 앱의 캔버스 행(`CanvasScene`)은 아직 이 경로를 쓰지 않습니다(UI 재설계 때 연결).
