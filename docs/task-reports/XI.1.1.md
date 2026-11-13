# 작업 보고서: XI.1.1 SceneNode·SceneResourceRef·SceneSnapshot 정의

- WBS: XI.1.1
- 작업명: `scene`에 `SceneNode`, `SceneResourceRef`, `SceneSnapshot`과 canonical round-trip serialization 정의
- 상태: 완료 (`[x]`)
- 선행: Phase X (phase 수준 선행은 사용자 지시(2026-11-14)에 따라 완화 — 아래 "차이" 참조)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/scene/src/node.rs`(신규) — `SceneNode`, `SceneNodeSpec`, `SceneContent`(Rect/Path/Text/Image
  primitive), `SceneResourceRef`, `TextLayoutKey`, `CornerRadii`, `ImageFit`, `ImageSampling`,
  `ImageCrop`, `SceneValueError`.
- `crates/scene/src/snapshot.rs`(신규) — `SceneSnapshot`(document·scene revision, 최상위 node 목록),
  재귀 없는 pre-order 순회 `PreOrder`, `resource_refs`.
- `crates/scene/src/codec.rs`(신규) — canonical `encode`/`decode`, `SceneCodecError`, 내부 `ByteSink`.
- `crates/scene/src/geometry.rs` — `FillRule::tag`/`from_tag`(document `WindingRule`과 같은 0/1),
  sink 기반 `write_canonical`, document path 모델과의 대응표 문서화.
- `crates/scene/src/lib.rs` — 모듈 등록·재노출.
- `docs/task-reports/XI.1.1.md`(신규), 계획서(XI.1.1 `[ ]`→`[x]`).

## 구현 요약

- **Document Node 미포함**: scene node는 원본 document node를 `StableId` 값(`source`)으로만
  가리킵니다. scene crate는 core-types만 의존합니다(`cargo xtask validate-crate-graph` 통과).
- **GPU 객체 미포함**: 이미지 픽셀과 text layout 결과는 `SceneResourceRef::{Image(AssetRef),
  TextLayout(TextLayoutKey)}` 값 식별자로만 가리킵니다. 모든 타입이 내부 가변성 없는 값 타입이라
  `SceneSnapshot: Send + Sync + 'static`입니다(정적 검사 테스트).
- **primitive**: Frame 배경용 둥근 사각형(`RectPrimitive`), Shape·Path 공용 `PathPrimitive`
  (`PathGeometry` + `FillRule`), `TextPrimitive`(layout key + 상자), `ImagePrimitive`(asset, 상자,
  fit, crop, sampling, 색공간 override). 이미지 enum tag는 document `FitMode`·`SamplingMode`와 같습니다.
- **구조 공유 준비**: 자식은 `Arc<SceneNode>`로 보관하고 node마다 subtree node 수를 캐시합니다.
- **serialization**: magic `STSN` + 버전 1 + revision 2개 + pre-order node(source, content, 자식 수).
  프로젝트 파일 포맷이 아닌 테스트·golden·hash용 canonical 표현입니다. decode는 잘림·잉여 bytes·
  미지 tag·비유한·음수·잘못된 crop·subpath 규칙 위반을 구조화 오류로 거부하고, 깊은 tree를
  명시적 stack으로 처리합니다.
- **geometry 대응**: scene `PathGeometry`는 document VI.6과 같은 command 집합·subpath 규칙
  (첫 command MoveTo, Close 다음 MoveTo)·winding tag를 쓰며, 대응표를 모듈 문서에 적었습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-scene` | 16 passed, 0 failed |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-scene -- --check` | 통과 |
| `cargo test -p stellar-renderer-core` | 24 passed, 0 failed (scene 재노출 사용처 회귀 없음) |
| `cargo xtask validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| 모든 content 종류·중첩 snapshot encode→decode 값 동일, 재인코딩 bytes 동일(`snapshot_round_trips_through_canonical_bytes`) | 통과 |
| header·leaf node bytes 수기 확인(`header_bytes_are_hand_checked`) | 통과 |
| 깊이 1001 tree round-trip(`deep_tree_round_trips_without_recursion`) | 통과 |
| 모든 잘림 길이에서 오류·잉여 bytes 거부(`every_truncation_is_a_structured_error`) | 통과 |
| magic·버전·tag·NaN·음수·crop·subpath·거대 개수 거부(`malformed_values_are_rejected`) | 통과 |
| `-0.0` 정규화로 bytes 동일(`negative_zero_is_canonical`), FillRule tag = document WindingRule tag | 통과 |
| `SceneSnapshot`/`SceneNode` `Send + Sync + 'static`(`snapshot_is_send_sync_value_type`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 "Phase X"는 phase 수준 선행이라 사용자 지시(2026-11-14)에 따라 완화했습니다. 이 작업은
  core-types 값 타입만 필요하며 b703259에 모두 있습니다.
- world transform·clip·paint·z-order·hit-test metadata는 XI.1.2 범위라 이 작업에 넣지 않았습니다.

## 남은 위험 / 한계

- scene `PathGeometry`·이미지 enum과 document 원본의 의미 동등성은 두 crate를 모두 보는 crate에서만
  교차 테스트할 수 있습니다. 현재 crate graph에는 그런 crate가 없으므로 Scene 생성 통합 계층이
  생길 때 대응표 기준 교차 fixture를 추가해야 합니다.
- `TextLayoutKey`의 의미(입력 hash 구성)는 text layout cache(X.10.5, 미착수)가 정합니다.
- 매우 깊은(수만 단계) tree의 `Drop`·`PartialEq`는 재귀입니다. decode·encode·순회는 재귀가 없습니다.
