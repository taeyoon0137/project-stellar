# 작업 보고서: XI.6.5 atlas page 할당·eviction

- WBS: XI.6.5
- 작업명: glyph atlas page allocation·eviction 구현, 현재 frame 사용 glyph 비제거
- 상태: 완료 (`[x]`)
- 선행: XI.6.4 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/glyph/atlas.rs`(신규) — `ShelfGlyphAtlas`(`GlyphAtlas` 구현), `AtlasConfig`,
  `AtlasStats`, `TrimOutcome`, `MAX_ATLAS_PAGES`.
- `crates/renderer-core/src/glyph/atlas/tests.rs`(신규) — 할당·eviction·memory pressure simulation 테스트.
- `crates/renderer-core/src/glyph/mod.rs` — `atlas` 모듈·재노출, `AtlasError::FormatMismatch`.
- `docs/task-reports/XI.6.5.md`(신규), 계획서(XI.6.5 `[ ]`→`[x]`).

## 구현 요약

- **page·할당**: 한 변 `page_size`(64..=8192) 정사각 page, page마다 format 하나(Coverage8→`R8Unorm`,
  Rgba8Premultiplied→`Rgba8UnormSrgb`). glyph는 사방 gutter(0..=8 px)를 더한 cell로 shelf packing합니다(남는
  높이가 가장 작은 shelf, 같으면 앞 shelf → 새 shelf → 같은 format 다음 page → 예산 안 새 page → LRU page 비움).
  결과는 입력 순서로만 정해집니다.
- **eviction**: page 단위 LRU(마지막 사용 frame, 같으면 id 작은 쪽)입니다. **현재 frame에서 조회·배치한 page**와
  **마지막 사용 frame을 GPU가 아직 끝내지 않은 page**는 비우지 않습니다(`gpu_completed` 알림 기준). 비울 page가
  없으면 `Full`입니다. page를 없애면 slot 세대가 올라가 이전 `AtlasPlacement`는 `is_valid`가 거짓이 됩니다.
- **memory pressure**: `apply_memory_pressure(max_pages)`가 예산을 바꾸고 넘는 page를 LRU로 없앱니다. 보호된
  page는 남기고 `protected_over_budget`으로 보고하며, 이후 frame에서 보호가 풀리면 예산으로 수렴합니다.
- **upload**: `upload_request`가 gutter까지 0으로 채운 cell 전체를 `UploadRequest`(XI.5.3 upload queue)로 만들어,
  재사용 page의 이전 pixel이 bilinear sampling으로 번지지 않게 합니다. `page_texture_desc`는 page texture
  descriptor를 줍니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-renderer-core` | 종료 코드 0 |
| `cargo test -p stellar-renderer-core` | 125 unit passed(신규 7) + 3 doctest passed, 0 failed |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 40 glyph shelf 배치: cell(gutter 포함) 겹침 0·page 안, 같은 입력 같은 배치, 기존 key 재사용(`shelf_allocation_is_disjoint_and_deterministic`) | 통과 |
| format별 별도 page·메모리 합, format 불일치·page 초과 bitmap·잘못된 설정·frame 역행 거부(`formats_use_separate_pages_and_validate`) | 통과 |
| 예산 2장에서 LRU page 비움, 그 glyph 조회 miss, 이전 placement 무효(`lru_page_is_evicted_and_placements_invalidated`) | 통과 |
| 현재 frame 사용 page·GPU 미완료 frame 사용 page는 비우지 않고 `Full`, GPU 완료 후 비움(`current_frame_and_in_flight_pages_are_never_evicted`) | 통과 |
| 예산 1장 압박에서 현재 frame page 2장 보호·나머지 제거, 다음 frame에 1장으로 수렴, 예산 해제 후 증가(`memory_pressure_keeps_protected_pages_then_converges`) | 통과 |
| memory pressure simulation 400 frame(GPU 2 frame 지연, 예산 1~6 무작위 변경): 매 조작 뒤 현재 frame이 받은 모든 placement 유효, frame 안 배치 겹침 0, eviction 발생(`memory_pressure_simulation_never_evicts_current_frame_glyphs`) | 통과 |
| gutter 0 채움 upload 영역·bytes golden, upload queue 길이 검증 통과(`upload_request_fills_gutter_with_zeros`) | 통과 |

## 차이(작업 지시와 다른 점)

- eviction 단위를 glyph가 아니라 page로 정했습니다(아키텍처 §10.4 "사용 빈도가 낮은 glyph atlas 페이지",
  shelf packing의 단편화 회피).

## 남은 위험 / 한계

- shelf packing은 높이가 크게 다른 glyph가 섞이면 공간 효율이 떨어집니다. 실제 텍스트 benchmark(XVI)로 page 크기·
  예산을 조정해야 합니다.
- cache priority 정책(XVI.5.2)과 시스템 메모리 압박 감지 연결은 후속 작업입니다. 현재는 호출자가 예산을 넘깁니다.
- page texture 생성·파괴와 `DeferredRelease` 연결은 backend 연결 작업(XI.8)에서 합니다.

## 후속 수정 (리뷰 반영)

`Fix: color glyph pixel 규약과 atlas page texture 수명 규약 명시` commit으로 리뷰 지적(minor)을 반영했습니다
(문서 규약 명시와 테스트 추가, 동작 코드 변경 없음).

- **color page pixel 규약**: `GlyphBitmapFormat::Rgba8Premultiplied`가 어느 공간에서 premultiply했는지 정의되어
  있지 않았습니다. 이미지 mip chain(XI.7.2)과 같은 규약으로 명시했습니다. RGB는 선형광에서 premultiply한 값을 sRGB
  transfer 곡선으로 인코딩한 8-bit, 알파는 선형 8-bit이며 texture는 `Rgba8UnormSrgb`입니다. 인코딩된 값에 알파를
  곱한 bitmap(일반적인 bitmap context 출력)은 이 규약이 아니므로 rasterizer 연결 계층이 변환해야 한다고 적었습니다.
  변환 helper는 넣지 않았습니다. 실제 color glyph rasterizer와 그 출력 형식이 아직 없어(XI.8.x 이후) 입력 규약을
  확정할 수 없기 때문입니다.
- **page texture 수명 규약**: 비운 slot이 다른 format page로 다시 쓰일 수 있는데 renderer가 texture를 다시 만들
  시점을 알리는 규약이 없었습니다. 동작은 이미 이를 구분할 수 있었습니다. slot을 비우면 세대가 오르고, placement는
  `(page, generation, format)`을 담습니다. 그래서 renderer가 page texture를 `(page id, 세대)`마다 두고, 맞는 texture가
  없으면 placement의 `format`으로 `page_texture_desc`를 불러 새로 만든다는 규약을 모듈 문서에 적었습니다. 별도
  이벤트 API는 두지 않았습니다.
- 동작 코드가 그대로이므로 실패를 먼저 재현하는 테스트는 없습니다. 대신 renderer가 기대는 보장을
  `reused_slot_bumps_generation_and_can_change_format` 테스트로 고정했습니다. 검증 기준 "memory pressure
  simulation 통과"에는 영향이 없습니다.

| 검사 | 결과 |
|---|---|
| `reused_slot_bumps_generation_and_can_change_format`(예산 1장, Coverage page를 비우고 같은 slot에 color glyph) | 통과: 같은 page id·다른 세대, `format`·`page_format` = `Rgba8Premultiplied`, 이전 placement 무효, texture format `R8Unorm` → `Rgba8UnormSrgb` |
