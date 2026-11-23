# 작업 보고서: XI.2.1 scene dirty set 변환

- WBS: XI.2.1
- 작업명: layout/binding/modifier invalidation을 scene dirty set으로 변환
- 상태: 완료 (`[x]`)
- 결정: ADR-0084 결정 2
- 선행: VII.1.4(`[x]`), X.7.1(`[x]`), VI.9.7(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 측정 장비는 개발 장비(Apple M3 Max, 36GB)이고 기준 장비(M1 16GB)가 아닙니다.

## 변경 파일 목록

- `crates/scene/src/dirty.rs`(신규) — `SceneInvalidation`·`InvalidationClasses`·`SceneDirtySet`·`SceneDependents`.
- `crates/scene/src/lib.rs` — 모듈 등록.
- `crates/runtime/src/canvas/scene_source.rs` — command change set → `SceneInvalidation`(`scene_invalidations`).
- `docs/task-reports/XI.2.1.md`(신규).

## 구현 요약

- scene은 document·layout·commands를 import하지 않으므로 무효화를 scene 쪽 값으로 받습니다.
- paint class만이면 Paint입니다. geometry·layout class, layout rect 위치·크기 변경, 잠금 전환, 구조 변경은 Geometry입니다. 잠금 전환은 문서 class가 비어 있지만 scene hit metadata가 바뀝니다. View Variant·정의 교체는 Subtree입니다.
- 같은 node의 여러 입력은 큰 종류가 이깁니다. Boolean 결과처럼 다른 node geometry를 읽는 node는 `SceneDependents`로 geometry dirty를 전파합니다. operand의 paint 변경은 전파하지 않습니다.
- Paint와 Geometry의 구분은 집계와 Subtree 강제에만 씁니다. 자손 world 값 전파는 증분 builder의 scope 비교가 맡습니다. 그래서 문서 class가 paint인 Frame transform도 자손이 맞게 다시 평가됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | lib 151 passed, 통합 3 passed(1 ignored), 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo test -p stellar-runtime --lib scene_source` | 6 passed, 0 failed |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `dirty::tests`: paint-only fixture, geometry fixture(layout rect, 잠금, class 없음 제외), subtree fixture(큰 종류 우선·구조 변경), Boolean 의존 전파 — 4개 통과.
- `scene_source::tests::change_set_drives_a_minimal_rebuild`: 실제 command change set에서 Paint(배경색)·Paint(Frame transform 문서 class)·Geometry(잠금)를 확인.

## 성능

- 해당 없음(분류 함수, 입력 크기에 선형).

## 차이(작업 지시와 다른 점)

- 구조 변경은 계획의 subtree 대신 그 node의 Geometry로 분류합니다. 자식 목록 차이는 증분 builder가 비교하므로 형제 subtree를 다시 만들지 않습니다. Subtree는 Variant·정의 교체에만 씁니다(ADR-0084 결정 2).

## 남은 위험 / 한계

- binding 재평가(VIII) 출처의 무효화는 아직 하류 연결이 없습니다. `InvalidationOrigin::Binding`으로 받을 수 있게만 두었습니다.
