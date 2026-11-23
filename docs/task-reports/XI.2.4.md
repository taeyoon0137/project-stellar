# 작업 보고서: XI.2.4 ComponentInstance 가상 subtree와 View Variant 결과의 scene 생성

- WBS: XI.2.4
- 작업명: ComponentInstance virtual subtree와 View Variant 적용 결과를 scene으로 생성
- 상태: 부분 완료 (`[~]`)
- 결정: ADR-0084 결정 5
- 선행: IX.2.2(`[x]`), IX.7.6(`[x]`), XI.2.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/scene/src/source.rs` — `SceneKey::virtual_node`(인스턴스 chain + 정의 node → version 8 id).
- `crates/runtime/src/canvas/scene_source.rs`(신규) — 문서·인스턴스 expansion → `SceneSource`, `expand`, Variant 표시 patch.
- `crates/runtime/src/canvas.rs`, `canvas/shape.rs`, `canvas/image.rs` — 캔버스 행과 scene source가 같은 입력 함수(`frame_input`·`shape_source`·`image_input`)를 쓰도록 나눔.
- `docs/task-reports/XI.2.4.md`(신규).

## 구현 요약

- 인스턴스는 문서에 복제하지 않고 expansion view의 `walk`·`children`·`node`로 가상 subtree를 펼칩니다. 인스턴스 key는 scene node가 없는 Fragment이고 그 아래 정의 root 투영이 옵니다.
- 가상 node key는 FNV-1a 128 결정적 id입니다. scene `source`는 바깥 인스턴스라 hit test가 인스턴스를 고릅니다(캔버스 행과 같은 규칙). 정의 root의 캔버스 원점은 인스턴스 배치로 바꿉니다(`placed_free_frame`). slot에 투영한 소유 node는 자기 StableId를 씁니다.
- View Variant는 해석한 `VariantOverrideSet`의 표시 여부 patch(`node_keys::VISIBLE` + Boolean)를 적용합니다. 나머지 property·구조·slot patch는 `unapplied_variant_patches`에 셉니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --lib scene_source` | 6 passed, 0 failed |
| `cargo test -p stellar-scene` | lib 151 passed, 통합 3 passed(1 ignored), 0 failed |
| `cargo clippy -p stellar-runtime --all-targets -- -D warnings` | 경고 0 |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `component_instances_expand_to_virtual_scene_nodes`: 인스턴스 2개 × 가상 node 3개, source = 인스턴스, 배치 원점 (60,100), nested 위치 (91,110).
- `variant_visibility_patches_apply_and_others_are_counted`: 표시 patch 적용, 다른 patch 1개 집계.
- `ix9_component_fixture_expands_structurally`: IX.9.1 fixture 인스턴스 5개의 expansion을 끝까지 펼치고, 건너뜀 이유가 payload 하나뿐임을 확인.
- `virtual_keys_are_deterministic_and_distinct_from_document_ids`(scene).

## 성능

- 인스턴스마다 expansion 색인(`SceneIndex`)을 문서 전체 payload로 한 번 만듭니다. 인스턴스가 있는 문서에서만 O(node 수)입니다.

## 차이(작업 지시와 다른 점)

- IX.9.1 fixture는 정의 node payload bytes가 비어 있습니다(IX.9.1 소유). 그래서 golden scene snapshot 대신 expansion 구조 확인을 합니다. payload가 있는 자체 fixture로 scene 내용을 검증했습니다.

## 남은 위험 / 한계

- View Variant property·구조·slot patch는 적용하지 않습니다. 문서 계층에 `TypedValue → payload` 쓰기 함수가 없고, adapter가 따로 해석하면 이중 경로가 되기 때문입니다. 그 함수가 생기면 연결해야 합니다. 그래서 상태를 `[~]`로 둡니다.
- 인스턴스 property override(예: Button label)는 binding(VIII) 연결 전이라 scene에 반영하지 않습니다.
- 정의 node의 Text는 text layout hook이 없으면 건너뜁니다. 정의 tree에는 modifier stack이 없습니다.

## 후속 보완 (2026-11-23, 가상 key skip 기록 정리)

- 결함: 인스턴스 펼침에서 나온 skip 기록(가상 key)은 `scene_edits`가 다시 변환할 때 지우지 않았습니다. 정리 조건이 문서 node id만 봤는데, 가상 key의 id는 (인스턴스 chain, 정의 node)의 hash라 맞지 않았습니다. 인스턴스를 지워도 그 안의 skip 기록이 남았습니다.
- 수정(ADR-0084 결정 5 개정):
  - 변환이 인스턴스 펼침 skip 기록마다 바깥 인스턴스를 함께 남깁니다(`DocumentSceneSource`의 내부 표).
  - 인스턴스를 다시 변환하거나 지우면 그 기록을 지웁니다.
- 변경 파일: `crates/runtime/src/canvas/scene_source.rs`, `crates/runtime/src/canvas/scene_source/tests.rs`, `docs/adr/ADR-0084-…`.
- 테스트: `scene_source::tests::virtual_skip_records_follow_their_instance`. 정의에 Text node를 둔 인스턴스 두 개는 가상 key마다 Text skip을 남깁니다. 인스턴스 하나를 지우면 그 기록만 사라지고, 증분 결과의 skip·source가 처음부터 변환한 결과와 같습니다.
- 검증: `cargo test -p stellar-runtime --lib scene_source` 12 passed.
