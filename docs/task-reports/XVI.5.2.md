# 작업 보고서: XVI.5.2 cache priority

- WBS: XVI.5.2
- 작업명: cache priority를 Document/CurrentScene(비축출), VisibleGlyph/Geometry, NearViewport, BackgroundThumbnail 순으로 정의
- 상태: 완료 (`[x]`)
- 선행: XI.6.5, XI.7.3
- 결정 기록: ADR-0108 결정 4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust).

## 변경 파일 목록

- `crates/renderer-core/src/cache.rs`(신규) — `CachePriority`, `CacheKind`, `CacheUsage`, `BudgetedCache`, `ReportedUsage`,
  `CacheBudgetManager`(등록·사용량 합·제거 순서), `EvictionPass`·`ChunkOutcome`·`EvictionDeadline`(`UnitLimit`, `Unbounded`).
- `crates/renderer-core/src/cache/tests.rs`(신규), `crates/renderer-core/src/lib.rs`(module 등록).

## 구현 요약

- **priority**(제거 순서): `BackgroundThumbnail` → `NearViewport` → `VisibleGlyphGeometry`, `DocumentScene`(현재 문서 모델·현재
  Scene Snapshot)은 지우지 않습니다. manager는 `DocumentScene`에 대해 cache의 `evict_one`을 부르지 않습니다.
- **같은 priority 안 순서**: 아키텍처 §10.4 "우선 제거" 목록 순서(`CacheKind::eviction_rank`: 썸네일 → 이미지 mipmap →
  재생성 가능한 tessellation → 사용 빈도가 낮은 glyph atlas page), 같은 종류는 등록 순서, 한 cache 안에서는 cache가 정한
  LRU 순서입니다.
- **cache 계약**(`BudgetedCache`): priority별 실제 사용 byte를 보고하고, priority 하나의 가장 오래 쓰지 않은 단위 하나를
  지워 줄어든 byte를 돌려줍니다. 지금 지울 수 없는 단위(현재 frame·GPU 진행 중)만 남았으면 0을 돌려주고, manager는 그
  (cache, priority)를 이번 pass에서 건너뜁니다(무한 반복 없음).
- **제거 단위 실행**: `evict_chunk`는 전체 사용량이 목표 이하가 될 때까지 deadline이 허락하는 만큼 한 단위씩 지우고, 진행
  상태(`EvictionPass`)를 호출자가 들고 다음 chunk에서 이어 갑니다. 시간 예산과 압박 단계 정책은 XVI.5.4입니다.

## 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core --lib cache::` | 2 passed(이 행 시험), 같은 filter의 기존 시험 포함 18 passed |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| eviction order fixture: 문서·scene(비축출), §10.4 순서와 반대로 등록한 glyph·tessellation·mipmap·썸네일 cache 둘, 지울 수 없다고 답하는 cache를 목표 0으로 지우면 순서가 썸네일(등록 순) → NearViewport(mipmap → tessellation) → Visible(mipmap → tessellation → glyph)이고 Document/CurrentScene과 보호 단위만 남음(`eviction_order_fixture`) | 통과 |
| 목표에 닿으면 멈추고 chunk 사이에 이어 감(`eviction_stops_at_the_target_and_resumes_across_chunks`) | 통과 |

## 남은 한계

- 실제 cache 등록(XVI.5.3), 압박 단계와 2 ms chunk(XVI.5.4), GPU OOM purge(XVI.5.6)는 각 행 보고서에 있습니다.
- 앱 render thread가 manager를 소유하고 viewport 이동마다 entry priority를 바꾸는 연결은 UI 재설계 때 합니다(엔진 API만 있음).
