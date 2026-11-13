# 작업 보고서: X.10.5 shaping·줄 레이아웃 cache key

- WBS: X.10.5
- 작업명: shaping·line layout cache key에 text hash, style hash, font face hash, width, locale, target profile 포함
- 상태: 완료 (`[x]`)
- 선행: X.10.1(이 lane `[x]`), X.8.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon arm64, 순수 Rust 로컬 검증).

## 변경 파일 목록

- `crates/text/src/cache.rs`(신규) — `ShapingCacheKey`, `LineLayoutCacheKey`, `StableHasher`(FNV-1a 128),
  `Hash128`, `TargetProfileKey`, `text_hash`, `style_hash`, `font_face_hash`, `LayoutCache`(hit·miss 집계, 검증 값
  비교).
- `crates/text/src/lib.rs` — 모듈 선언.
- `docs/task-reports/X.10.5.md`(신규), 계획서(X.10.5 `[ ]`→`[x]`).

## 구현 요약

- **shaping key**: text hash(paragraph 텍스트), style hash(기준 방향, span 범위·style index·언어, style별 글꼴
  크기·feature·variation), font face hash(style별 chain 항목의 출처·file hash·face index·axes와 항목별 사용 가능
  여부), locale, target profile(호출자 제공 `TargetProfileKey`).
- **줄 레이아웃 key**: shaping key + 너비 제약 + paragraph 레이아웃 속성(정렬·줄 높이·wrap)과 줄바꿈 옵션
  (word-break·엄격도·locale) hash.
- **hash**: 길이 접두 필드를 쓰는 FNV-1a 128-bit(정수 연산만, 실행·플랫폼 독립). key는 저장하지 않는 런타임 값입니다.
- **cache**: `LayoutCache`는 `BTreeMap` 기반이고 key와 함께 원본 검증 값을 보관해, hash가 같아도 원본이 다르면
  miss로 처리합니다(충돌 방어). 사용 가능 여부가 key에 있어 누락 폰트가 등록·로드되면 다시 shaping합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 / 결과 |
|---|---|
| `cargo fmt -p stellar-text` | 0 |
| `cargo test -p stellar-text` | 0 (101 unit passed, 0 failed) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 (경고 0) |
| `cargo fmt -p stellar-text -- --check` | 0 |

| 검사 | 결과 |
|---|---|
| **한 입력 변경마다 예상 cache miss**(`each_input_change_causes_expected_cache_miss`): 같은 입력은 둘 다 hit. text·글꼴 크기·feature·span 언어·기준 방향·font face(chain 항목 추가)·locale·target profile 변경은 shaping·줄 key 모두 miss, width·정렬·word-break 변경은 shaping hit·줄 miss. hit 집계(shaping 3, 줄 1) | 통과 |
| font 사용 가능 여부: 등록만 된 face는 hash 불변, 로드되어 사용 가능해지면 hash 변경 | 통과 |
| 줄 높이 모드(Auto·Multiplier·Fixed)가 줄 key에 반영 | 통과 |
| FNV-1a 128 golden(빈 입력 = offset basis, "a" 공개 test vector), 길이 접두 구분, 검증 값 불일치 miss | 통과 |

## 차이(작업 지시와 다른 점)

- target profile은 `target-profile` crate 타입(IX.10.1, 미구현)에 의존하지 않고 호출자가 profile id·고정 version
  으로 만든 `TargetProfileKey`(128-bit)를 받습니다(`text`는 core-types만 의존).

## 남은 위험 / 한계

- 128-bit 비암호 hash라 의도적 충돌을 만들 수 있지만, cache가 원본 검증 값을 비교하므로 잘못된 결과를 쓰지
  않습니다(X.10.6 엔진이 검증 값으로 원본 입력을 보관).
- font face hash는 폰트 bytes가 아니라 file hash로 정체성을 봅니다(X.8.1). 같은 hash에 다른 bytes를 연결하는
  잘못된 registry 사용은 막지 못합니다.
