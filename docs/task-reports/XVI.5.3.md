# 작업 보고서: XVI.5.3 cache 사용량 공통 budget manager 등록

- WBS: XVI.5.3
- 작업명: glyph, tessellation, image mipmap, thumbnail cache가 공통 budget manager에 usage를 등록
- 상태: 완료 (`[x]`)
- 선행: XVI.5.1, XVI.5.2
- 결정 기록: ADR-0108 결정 5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust).

## 변경 파일 목록

- `crates/renderer-core/src/cache_adapters.rs`(신규) — `LruBudget`(key별 priority·byte·LRU 순번), `BudgetedTessellationCache`,
  `BudgetedMipmapCache`, `BudgetedGlyphAtlas`, `mesh_bytes`.
- `crates/renderer-core/src/cache_adapters/tests.rs`(신규), `crates/renderer-core/src/lib.rs`(module 등록).
- `crates/runtime/src/cache/mod.rs`(신규), `crates/runtime/src/cache/thumbnail.rs`(신규) — `ThumbnailCache`, `Thumbnail`,
  `ThumbnailKey`. `crates/runtime/src/lib.rs`(module 등록).

## 구현 요약

- 기존 cache(`TessellationCache`, `TextureCache`, `ShelfGlyphAtlas`) 파일은 바꾸지 않고 감싸는 타입을 두었습니다. 감싼 타입은
  원래 cache의 조회·삽입 API(`fill`·`stroke`, `get_or_create`, `GlyphAtlas`)를 그대로 거치면서 entry마다 priority·byte·마지막
  사용 순번을 기록합니다. manager가 보는 사용량은 cache가 실제로 든 resource byte의 합입니다.
  - tessellation: mesh byte = vertex(`Vec2`, 8 byte) + index(`u32`, 4 byte). entry priority는 호출자가 정합니다(보이는 것
    `VisibleGlyphGeometry`, viewport 밖 `NearViewport`), 다시 쓰면 최근 사용으로 바뀌고 priority가 다르면 옮깁니다.
  - 이미지 mipmap: `MipChain::total_bytes`. priority는 tessellation과 같은 규칙입니다.
  - glyph atlas: page texture byte(`memory_bytes`), 모두 `VisibleGlyphGeometry`. 제거는 atlas의 LRU page 제거
    (`apply_memory_pressure`)를 한 page씩 쓰고, 현재 frame·GPU 진행 중 page와 마지막 page 하나는 atlas 규칙대로 남깁니다.
    `apply_memory_pressure`는 page 상한도 낮추므로, page 하나를 지운 직후 상한을 처음 설정으로 되돌립니다(검토 수정:
    처음 구현은 상한을 낮춘 채 두고 되돌리는 호출을 앱 연결에 미뤄, 압박 pass·GPU OOM purge 뒤 atlas가 page 한 장 상한에
    묶였습니다. `glyph_page_eviction_keeps_the_configured_page_budget`가 확인합니다).
  - 썸네일(runtime): 문서·크기별 RGBA pixel, 모두 `BackgroundThumbnail`, LRU.
- LRU 기록(`LruBudget`)은 한 구현을 renderer-core와 runtime 썸네일이 함께 씁니다.
- Document/CurrentScene은 `ReportedUsage`(XVI.5.2)로 사용량만 등록합니다. 문서 모델·scene 크기를 재는 일은 연결하는 쪽이
  넘깁니다.

## 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core --lib cache` | 이 행 시험 3 passed(같은 filter 전체 26 passed) |
| `cargo test -p stellar-runtime --lib cache::` | 1 passed |
| `cargo clippy -p stellar-runtime -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-runtime -p stellar-renderer-core -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| total usage 일치: 문서·scene 보고값 + glyph atlas page byte(3 × 64²) + tessellation mesh byte(같은 tessellator로 다시 계산) + mip chain byte(같은 이미지로 다시 생성) + 썸네일 byte가 manager 합계·cache별 합계와 같고, priority 이동·삭제 뒤에도 따라감(`total_usage_matches_registered_caches`) | 통과 |
| 실제 cache를 등록해도 XVI.5.2 순서로 지우고 Document/CurrentScene과 atlas 마지막 page만 남음(`registered_caches_follow_the_eviction_order`) | 통과 |
| tessellation 안 LRU: 다시 쓴 entry가 남고 목표에서 멈춤(`eviction_stops_at_target_and_keeps_lru_order_within_a_cache`) | 통과 |
| 썸네일 cache 사용량 등록·같은 key 교체·LRU 제거(`thumbnail_cache_registers_usage_and_evicts_least_recent_first`) | 통과 |

## 남은 한계

- 썸네일을 그리는 경로(비활성 문서 render)와, 앱 render thread가 감싼 cache를 실제로 쓰도록 바꾸는 연결은 아직 없습니다.
  지금 제품 경로의 cache 사용처는 UI 재설계 때 이 타입으로 연결합니다(ADR-0108 결정 5).
