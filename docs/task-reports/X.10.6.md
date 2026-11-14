# 작업 보고서: X.10.6 paragraph 단위 증분 텍스트 레이아웃

- WBS: X.10.6
- 작업명: 텍스트 일부 편집 시 영향 paragraph와 이후 line만 재배치하는 incremental API
- 상태: 완료 (`[x]`)
- 선행: VI.7.5(`[x]`), X.10.5(이 lane `[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon arm64, 순수 Rust 로컬 검증).

## 변경 파일 목록

- `crates/text/src/incremental.rs`(신규) — `IncrementalTextLayout`, `LayoutRequest`, `ParagraphSpec`, `LayoutReport`,
  `affected_paragraphs`, `AffectedParagraphs`, `paragraph_ranges`.
- `crates/text/src/cache.rs` — `LayoutCache::peek`(집계 없는 조회).
- `crates/text/src/paragraph.rs` — `TextLayoutError::CacheEntryMissing`(내부 불변식 위반을 panic 대신 오류로).
- `crates/text/src/lib.rs` — 모듈 선언.
- `docs/task-reports/X.10.6.md`(신규), 계획서(X.10.6 `[ ]`→`[x]`).

## 구현 요약

- **cache 재사용**: paragraph마다 X.10.5 shaping key·줄 레이아웃 key로 `ShapedParagraph`·`ParagraphLines`를
  cache합니다. 내용·style·font·locale·target이 같은 paragraph는 다시 shaping하지 않고, 너비·paragraph 속성까지
  같으면 줄도 다시 나누지 않습니다. key가 paragraph 내용 기반이라 paragraph 삽입·삭제로 index가 밀려도 재사용됩니다.
- **영향 범위**: 편집된 paragraph만 다시 shaping·줄 나누기 하고, 그 뒤 모든 줄의 위치(y·정렬)는 `compose_text`
  (X.10.2)로 다시 계산합니다(glyph 수에 비례하는 저비용 단계). 이후 paragraph 줄은 glyph·x가 같고 y만 밀립니다.
- **충돌 방어·메모리**: cache는 원본 paragraph 텍스트·span·기준 방향을 검증 값으로 비교하고, 요청이 끝나면 이번에
  쓰지 않은 항목을 지웁니다(cache 크기 ≤ paragraph 수).
- **편집 helper**: `affected_paragraphs(old_text, edit, replacement)`가 편집 전·후 paragraph index 구간을 돌려줍니다
  (문서 편집 helper VI.7.5 결과를 layout 무효화 범위로 옮길 때 사용).
- `shape_calls`·`line_layout_calls`와 요청별 `LayoutReport { reshaped, relaid }`로 재계산을 관찰합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 / 결과 |
|---|---|
| `cargo fmt -p stellar-text` | 0 |
| `cargo test -p stellar-text` | 0 (104 unit passed, 0 failed) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 (경고 0) |
| `cargo fmt -p stellar-text -- --check` | 0 |

| 검사 | 결과 |
|---|---|
| **첫 paragraph 편집이 독립 둘째 paragraph를 재shape하지 않음**(`first_paragraph_edit_does_not_reshape_independent_second_paragraph`): 첫 레이아웃 reshaped `[0,1]`, 첫 paragraph 편집 후 reshaped·relaid `[0]`만, 누적 shaping 3회·줄 3회. 첫 paragraph가 1줄 → 2줄이 되자 둘째 paragraph 줄은 glyph·x 동일, top만 +1024 | 통과 |
| 둘째 paragraph 편집 → `[1]`만, 같은 입력 재요청 → 재계산 0, 너비만 변경 → shaping 0·줄 `[0,1,2]`, 앞에 paragraph 삽입 → 새 paragraph `[0]`만, 정리 후 cache 크기 4 | 통과 |
| `affected_paragraphs`: paragraph 안 편집, 줄바꿈 삽입(0 → 0..2), 경계 삭제 병합(0..2 → 0..1), 마지막 paragraph, 범위·문자 경계 오류 / `paragraph_ranges` | 통과 |

## 차이(작업 지시와 다른 점)

- "영향 paragraph와 이후 line만 재배치"를 paragraph 단위 cache로 구현했습니다. 이후 paragraph의 줄은 다시 나누지
  않고 위치만 다시 계산하며, 편집된 paragraph 안에서는 줄 전체를 다시 나눕니다(같은 paragraph 안 앞줄 재사용 없음).

## 남은 위험 / 한계

- 아주 긴 단일 paragraph의 편집은 paragraph 전체를 다시 shaping합니다. paragraph 안 부분 재shaping(unsafe-to-break
  경계 활용)은 후속 최적화입니다.
- 문서 모델(scalar index)과의 연결(문서 Text payload → `ParagraphSpec`)은 상위 계층 몫입니다(`text`는 `document`에
  의존하지 않음).

## 후속 수정 (posthoc 리뷰 TS-4: cache 검증 값이 hash로만 비교하던 입력)

- **결함**: 모듈 문서와 X.10.5 보고서는 cache가 원본 입력을 다시 비교해 hash 충돌로 잘못된 결과를 돌려주지 않는다고
  했지만, shaping 검증 값은 paragraph 텍스트·span·기준 방향만 담았습니다. style 값(글꼴 크기·feature·variation),
  fallback chain 항목·사용 가능 여부는 FNV-1a hash로만 비교했고, 줄 검증 값 `(ShapingCheck, ParagraphLayoutStyle)`에는
  줄바꿈 옵션(word-break·엄격도)이 없었습니다. FNV-1a는 충돌 저항성이 없으므로 key가 충돌하면 다른 입력의 결과를
  돌려줄 수 있었습니다.
- **수정**: `ShapingCheck`가 paragraph가 쓰는 style(`paragraph_styles`)마다 글꼴 크기·`ShapingStyle`·fallback chain·
  항목별 사용 가능 handle을 원본으로 담습니다. 결과 run이 face handle을 담으므로 handle 정체성도 비교합니다. 줄 검증
  값은 `LineCheck`(shaping 검증 값, paragraph 속성, `LineBreakOptions`)입니다. locale·target profile·너비는 key에 원본
  그대로 있어 key 비교로 충분합니다. 레이아웃 결과는 같습니다(golden 불변).
- **회귀 테스트**: `incremental::tests::hash_collision_between_different_inputs_is_a_miss`는 cache 항목을 다른 입력의
  key로 옮겨(`LayoutCache::rekey`, 테스트 전용) 충돌을 재현합니다. 16px 항목에 20px 요청이, Normal 줄 항목에 KeepAll
  요청이 들어오면 miss로 다시 계산하고 결과가 새 엔진과 같은지 봅니다. 수정 전 검증 값으로는 두 경우 모두 앞 입력의
  결과를 그대로 돌려줘 실패합니다(shaping 검증 값만 고친 상태에서는 KeepAll 경우가 실패).

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-text` | 0 (115 unit passed, golden 포함) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 |
| `cargo clippy -p stellar-text --release -- -D warnings` | 0 |
| `cargo fmt -p stellar-text -- --check` | 0 |
