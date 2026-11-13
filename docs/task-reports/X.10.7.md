# 작업 보고서: X.10.7 glyph ID·advance·줄바꿈 golden 비교

- WBS: X.10.7
- 작업명: 동일 font files로 glyph IDs, advances, line breaks golden 비교를 검증 경로에 추가
- 상태: macOS 부분 완료, Windows 비교 보류 (`[~]`, ADR-0006 규칙 4)
- 선행: X.10.6(이 lane `[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon arm64). `CI-WIN`·`REF-WIN` 비교는 ADR-0006으로 보류.

## 변경 파일 목록

- `crates/text/src/text_layout.rs` — `TextLayout::encode_logical`(논리 레이아웃 canonical LE bytes, `STXL` v1).
- `crates/text/src/golden.rs`(신규, `#[cfg(test)]`) — golden corpus 10개, dump·digest, 비교·독립 실행 동일성·encoding
  header 테스트.
- `crates/text/tests/golden/logical_layout_v1.txt`(신규) — golden(폰트 fingerprint, canonical bytes FNV-1a 128 digest,
  줄·glyph dump). `STELLAR_TEXT_UPDATE_GOLDEN=1 cargo test -p stellar-text golden`으로 생성.
- `crates/text/src/lib.rs` — 테스트 모듈 선언.
- `docs/task-reports/X.10.7.md`(신규), 계획서(X.10.7 `[ ]`→`[~]`).

## 구현 요약

- **동일 font files**: 합성 fixture 폰트(코드로 결정론 생성)의 bytes FNV-1a 128 fingerprint를 golden에 기록합니다. 폰트
  bytes가 바뀌면 golden이 깨집니다.
- **corpus**(10): Latin ligature 좁은 폭, 일본어 금칙(。·작은 가나), 한글 Normal·keep-all(+Strict), LTR 안 히브리어·숫자,
  RTL 아랍어·히브리어 두 paragraph(간격), emoji·CJK fallback, variable axis(wght 900)+feature(liga 끔, 20px), 양쪽·가운데
  정렬·배율·고정 줄 높이·빈 paragraph·세로 가운데 box, missing glyph + U+2028 강제 줄바꿈.
- **비교 대상**: 줄 범위(줄바꿈)·위치·baseline과 glyph id·cluster·x·advance·양쪽 정렬 추가분·offset(전체 pipeline,
  `IncrementalTextLayout` 경유). golden 파일은 사람이 읽는 dump와 canonical bytes digest를 함께 담아 byte 단위 일치를
  확인합니다.
- **검증 경로**: 계획서 2026-11-14 개정("제품 crate 테스트는 CI에 추가하지 않고 통합 시 로컬 `cargo xtask check-core`
  로 검증")에 따라 `cargo test --workspace`(check-core test 단계)에 포함됩니다. CI workflow는 수정하지 않았습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 / 결과 |
|---|---|
| `STELLAR_TEXT_UPDATE_GOLDEN=1 cargo test -p stellar-text golden` | 0 (golden 생성, 286줄) |
| `cargo test -p stellar-text` | 0 (107 unit passed, 0 failed, golden 비교 포함) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 (경고 0) |
| `cargo fmt -p stellar-text -- --check` | 0 |
| golden 한 값(w=3481→3482)을 바꿔 실행 | 101 (첫 차이 줄을 보고하며 실패) → 원복 후 0 |
| `cargo xtask check-core` | **65** — fmt·clippy 통과, test 단계에서 `xtask` `dependencies::tests::real_dependencies_file_parses` 1건 실패(아래 남은 위험) |
| `cargo test --workspace --locked --no-fail-fast` | **101** — 위 xtask 테스트 1건만 실패, 나머지 76개 test binary 통과(`stellar-text` 107개 포함) |

| 검사 | 결과 |
|---|---|
| **golden 일치**(`logical_layout_matches_golden`): 10개 fixture의 dump·digest가 golden 파일과 정확히 같음 | 통과 |
| **byte-identical**(`logical_layout_bytes_are_identical_across_independent_runs`): 폰트·registry·엔진을 새로 만들어 두 번 실행한 canonical bytes 동일 | 통과 |
| canonical encoding header·길이(`STXL`, v1, 필드 크기) | 통과 |

## 차이(작업 지시와 다른 점)

- macOS·Windows 교차 비교는 ADR-0006으로 보류했습니다. golden은 플랫폼 독립 입력(합성 폰트 bytes)과 정수 출력만 쓰므로
  Windows 재개 시 같은 golden을 그대로 통과시켜야 합니다.
- golden 비교는 CI workflow가 아니라 workspace 테스트(check-core)에 넣었습니다(계획서 2026-11-14 개정).

## 남은 위험 / 한계

- **Windows 미검증**(ADR-0006 규칙 4): shaping은 OS API가 아니라 같은 순수 Rust 코드지만, `harfrust`·`read-fonts` 일부
  경로가 f32 연산(`round` 등 IEEE 기본 연산)을 쓰므로 macOS 증거만으로 `[x]`로 올리지 않았습니다. shaping·metric 경로에서
  libm 초월함수 사용은 찾지 못했습니다(초월함수는 outline·COLR 변환 경로에만 있음). Windows 재개 시 `cargo test -p
  stellar-text golden`을 `CI-WIN`에서 통과시키면 `[x]`입니다.
- **check-core 실패(통합 차단)**: `tools/xtask/src/dependencies.rs`의 `real_dependencies_file_parses`는 실제
  `config/dependencies.yaml`을 직접 의존성 `["yaml-rust2"]`만으로 검증하고, `validate-dependencies` 명령도 `cargo tree -p
  xtask`만 수집합니다. 이 lane이 추가한 제품 crate 의존성 승인 항목(harfrust, unicode-bidi, unicode-segmentation,
  unicode-linebreak)이 `unused-approval`이 되어 둘 다 실패합니다. workspace 전체 직접 의존성을 수집하도록 validator와
  테스트를 고치는 `Fix:`가 `tools/xtask`(이 lane 허용 경로 밖)에 필요합니다.
- 합성 폰트는 GPOS(kerning·mark 위치)·복합 script GSUB(아랍어 연결형 등)를 담지 않습니다. 실제 폰트 golden은 라이선스상
  저장 가능한 폰트를 프로젝트 fixture로 들일 때 추가해야 합니다.
