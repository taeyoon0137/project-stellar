# ADR-0116: 렌더 효과 pass, 문서 scene 변환 위치, PNG 내보내기 명령과 Metal offscreen target

## Status

Accepted (2026-11-23).

- ADR-0103 결정 8(CLI·MCP 연결 보류)을 결정 1·5로 풉니다.
- ADR-0019·ADR-0028의 crate 의존 표에 `stellar-scene → stellar-document`, `stellar-scene → stellar-commands`(feature
  한정)를 더합니다.

## Context

- 저장 문서 → scene 변환(canvas scene source, ADR-0084)은 앱 runtime(`crates/runtime/src/canvas/scene_source.rs`)에
  있었습니다. headless 도구(ops·CLI·MCP)는 runtime에 닿지 않으므로(runtime → ops 방향, ADR-0005 §8) PNG 내보내기
  엔진(ADR-0103)을 명령으로 연결하지 못했습니다.
- 같은 변환을 ops에 다시 만들면 이중 경로입니다(ADR-0103 Alternatives).

## Decision

1. **문서 → scene 변환은 scene crate의 `document` 모듈에 둡니다.** `stellar_scene::document`(feature `document`)가
   `document_scene_source`·`scene_edits`·`scene_invalidations`와 node 입력 함수(`inputs::frame_input`·`shape_source`·
   `shape_style`·`image_input`·`scene_command`·`scene_fit`)를 소유합니다. 앱 runtime은 이 모듈을 다시 노출하고
   (`stellar_runtime::canvas::scene_source`), 캔버스 행도 같은 입력 함수를 씁니다. ops는 같은 모듈로 저장 문서의
   scene을 만듭니다.
   - feature 한정 의존: `stellar-scene → stellar-document`, `stellar-scene → stellar-commands`(change set 입력). renderer-core·
     export는 feature를 켜지 않으므로 renderer는 문서 계층을 빌드하지 않습니다. 순환은 없습니다(document·commands는 scene에
     의존하지 않음).
   - 변환 규칙(숨김 유지, layout Frame·Path clip skip, instance 펼침, Boolean host, View Variant 표시 patch)은 ADR-0084 그대로입니다.
2. **문서 효과 modifier를 scene 합성 효과로 평가합니다(`stellar_scene::document::effects`).** Shadow·Blur·Blend·Opacity
   modifier는 node 합성 결과에 걸리는 CSS 효과(`box-shadow`·`filter: drop-shadow()`, `filter: blur()`, `mix-blend-mode`,
   `opacity`)라 내용 모양(transform·fill·stroke)과 따로 평가합니다. 변환은 이 종류를 stack에서 떼어 scene
   `ModifierPaint`의 `shadows`·`opacity`·`blend`·`layer_blur`에 넣고, 나머지 stack은 기존 입력 함수가 그대로 평가합니다.
   도형 modifier 규칙(`evaluate_shape_modifiers`)과 캔버스 행의 동작은 바뀌지 않습니다.
   - Opacity는 활성 항목의 곱(`ModifierStack::effective_opacity`, 문서 규칙)입니다. Shadow는 활성 항목 전부이고 OrderKey
     순서가 scene 목록 순서(앞이 아래)입니다. Blur·Blend는 CSS 속성 하나에 대응하므로 활성 항목이 둘 이상이면 평가하지
     않고 `SkipReason::UnsupportedModifiers`로 남깁니다(근사하지 않음). blend tag는 문서·scene이 같습니다.
   - Frame은 효과 modifier만 가진 stack을 평가합니다. 다른 종류가 있으면 전처럼 skip합니다. Boolean operand의 효과는
     결과에 흡수되어 쓰지 않습니다. 효과 적용 순서는 renderer가 정합니다(ADR-0102 결정 5).

## Alternatives

- ops에 별도 변환: 이중 경로라 탈락했습니다.
- 새 crate(`stellar-scene-source`): crate 하나와 정책 행이 더 생기지만 얻는 경계가 feature 한정 의존과 같아 탈락했습니다.
- scene이 무조건 document에 의존: renderer-core와 GPU backend가 문서 계층을 빌드하게 되어 탈락했습니다.

## Consequences

- 앱 runtime, xtask replay, headless 도구가 같은 변환 함수를 씁니다.

## Compatibility

- 파일 포맷·FFI 변경 없음. 공개 경로 `stellar_runtime::canvas::scene_source`는 re-export로 유지됩니다.
- crate 의존 표(`tools/xtask/src/crate_graph.rs`, `docs/architecture/crate-dependency-graph.md`)가 바뀌었습니다.

## Performance

- 변환 코드는 그대로 옮겼습니다. 성능 변화는 없습니다.

## Security

- 변화 없음.

## Migration

- 없음.
