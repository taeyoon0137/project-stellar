# 작업 보고서: XI.2.3 Fragment flatten

- WBS: XI.2.3
- 작업명: Fragment를 scene node로 만들지 않고 자식을 부모 scene scope에 flatten
- 상태: 완료 (`[x]`)
- 선행: VI.4.1 (`[x]`, b703259), XI.2.2 (이 lane에서 완료: `3ecc948`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/scene/src/builder.rs` — `FragmentInput`, `FragmentScope`, `SceneBuilder::begin_fragment`/
  `finish_fragment`, 모듈 문서 "Fragment flatten" 절.
- `crates/scene/src/builder/tests.rs` — Fragment 유무 동등성·표시/잠금 평탄화·최상위/빈 Fragment 테스트.
- `docs/task-reports/XI.2.3.md`(신규), 계획서(XI.2.3 `[ ]`→`[x]`).

## 구현 요약

- Fragment는 렌더·layout 필드와 좌표계가 없는 marker(아키텍처 §12.4, VI.4.1)이므로 scene node를
  만들지 않고 z-order도 받지 않습니다.
- `begin_fragment`는 부모와 **같은** world transform·clip을 쓰는 scope를 열고, Fragment의 표시·잠금만
  자식 상태(숨김 누적·잠금 누적)에 더합니다. `finish_fragment`는 모은 자식을 순서대로 부모 scope에 이어
  붙입니다. 중첩 Fragment도 같은 방식으로 펼쳐집니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | unit 52 passed, doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| frame { shape, [path, text, [image]], image }를 중첩 Fragment로 감싼 scene과 감싸지 않은 scene이 값·canonical bytes·semantic hash 모두 동일, Fragment source가 scene에 없음, 자식 5개가 Frame 직접 자식, Frame clip 공유(`fragment_presence_does_not_change_scene_primitives`) | 통과 |
| 숨긴 Fragment = 자식마다 숨김, 잠긴 Fragment = 자식마다 잠금(값 동일), 형제는 영향 없음(`fragment_visibility_and_lock_flatten_into_children`) | 통과 |
| 최상위 scope Fragment는 최상위 node로 펼쳐짐, 빈 Fragment는 node·z 미소비(`root_level_and_empty_fragments_flatten`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- hit test 결과는 Fragment 자식의 source를 돌려줍니다. 클릭을 그룹(Fragment) 선택으로 바꾸는 선택
  해석은 document·interaction 계층(XII.3.6) 소관입니다.
- 자식 bounds 합집합인 Fragment visual bounds(VI.4.3)는 scene에 저장하지 않습니다. 선택 handle 등이
  필요하면 document helper 또는 자식 scene bounds로 계산해야 합니다.
