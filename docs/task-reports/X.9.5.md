# 작업 보고서: X.9.5 grapheme·script run 단위 font fallback과 missing glyph cluster

- WBS: X.9.5
- 작업명: font fallback을 grapheme/script run 단위로 적용하고 missing glyph cluster 정보를 보존
- 상태: 완료 (`[x]`)
- 선행: X.8.3(`[x]`), X.9.2(이 lane `[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon arm64, 순수 Rust 로컬 검증).

## 변경 파일 목록

- `crates/text/src/font_runs.rs`(신규) — `FaceSource`, `LoadedFaces`, `assign_fonts`, `shape_font_runs`,
  `notdef_clusters`, `FontRun`, `RunFace`, `MissingGlyphCluster`, `MissingReason`, `FontAssignment`,
  `ShapedFontRun`, `FontRunError`, `is_default_ignorable`, 단위 테스트.
- `crates/text/src/lib.rs` — 모듈 선언.
- `docs/task-reports/X.9.5.md`(신규), 계획서(X.9.5 `[ ]`→`[x]`).

## 구현 요약

- **단위**: itemization item(X.9.2, script·방향·언어 run) 안에서 UAX #29 grapheme cluster마다 style의 해석된
  fallback chain(X.8.3 `ResolvedFallbackChain::select`)을 기록 순서로 검사해, cluster의 비무시 문자를 모두
  cmap에 가진 첫 로드된 face를 고릅니다. 같은 face가 이어지면 하나의 `FontRun`으로 합치고 item 경계는 넘지
  않습니다. ZWJ emoji 시퀀스는 cluster 하나라 한 face를 씁니다.
- **missing 보존**: 체인 안 어느 face도 덮지 못한 cluster는 체인 밖 font로 바꾸지 않고(X.8.3) 체인의 첫 로드된
  face로 `.notdef` shaping하며 `MissingGlyphCluster { range, item, NoCoveringFace }`로 기록합니다. 로드된
  face가 하나도 없으면 run은 `RunFace::None`(shaping 없음)입니다. `shape_font_runs`는 shaping 결과의
  `.notdef` cluster도 `NotdefAfterShaping`으로 더하되 이미 기록된 범위는 중복하지 않습니다.
- **무시 문자**: Default_Ignorable 문자(ZWJ, VS, BiDi 제어, soft hyphen 등)는 coverage 판단에서 빼고, 무시
  문자만으로 된 cluster는 앞 cluster face를 따릅니다.
- **face 저장소**: `FaceSource` trait과 결정론적 `LoadedFaces`(handle → `FontFace`, `BTreeMap`). registry에
  등록됐지만 로드되지 않은 face는 사용 불가로 봅니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 / 결과 |
|---|---|
| `cargo fmt -p stellar-text` | 0 |
| `cargo test -p stellar-text` | 0 (71 unit passed, 0 failed) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 (경고 0, `sort_by_key` 지적 1건 수정 후) |
| `cargo fmt -p stellar-text -- --check` | 0 |

| 검사 | 결과 |
|---|---|
| **emoji fallback fixture**: chain Latin→CJK→emoji, "Hi 😀 你好 👩‍💻!" → `Hi `(Latin) `😀`(emoji) ` `(Latin) `你好`(CJK) ` `(Latin) `👩‍💻`(emoji, ZWJ 시퀀스 한 cluster) `!`(Latin), missing 0, emoji glyph advance 1200 | 통과 |
| **CJK fallback fixture**: chain Latin→CJK, "Hello 世界 안녕" → `Hello `·`世界`·` `·`안녕`; 순서를 CJK→Latin으로 바꾸면 CJK가 공백까지 덮고 item 경계는 유지 | 통과 |
| missing glyph cluster: Latin만 있는 chain에서 "a世b"의 `1..4`가 NoCoveringFace, shaping 결과에 `.notdef`, 중복 기록 없음 | 통과 |
| registry에 없는 face는 건너뛰고 체인 밖 face로 바꾸지 않음, 전부 없으면 `RunFace::None`·전 cluster missing | 통과 |
| RTL item과 무시 문자 단독 cluster(RLM) | 통과 |
| 없는 style index는 typed error | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- coverage는 cmap 기준입니다. GSUB 조합으로만 표현되는 glyph(예: 특정 emoji 시퀀스 ligature 부재)는 shaping 뒤
  `.notdef` 검사로만 드러납니다.
- Default_Ignorable 목록은 코드에 고정했습니다(Unicode DerivedCoreProperties 기준). Unicode 갱신 시 함께 검토해야
  합니다.
