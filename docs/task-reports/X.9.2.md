# 작업 보고서: X.9.2 script·language·direction itemization

- WBS: X.9.2
- 작업명: 입력 문자열과 style run을 script·언어·BiDi 방향이 같은 shaping run으로 분할
- 상태: 완료 (`[x]`)
- 선행: X.9.1(이 lane `[x]`), VI.7.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon arm64, 순수 Rust 로컬 검증).

## 변경 파일 목록

- `crates/text/src/bidi.rs`(신규) — `BaseDirection`, `BidiLevel`, `BidiParagraph::analyze`(byte별 resolved level).
- `crates/text/src/itemize.rs`(신규) — `StyleSpan`, `TextItem`, `Itemization`, `itemize`, `resolve_scripts`,
  `char_script`, `ItemizeError`.
- `crates/text/src/lib.rs` — 모듈 선언.
- `crates/text/Cargo.toml` — `unicode-bidi = "=0.3.18"`(`hardcoded-data`, `std`만).
- `config/dependencies.yaml` — `unicode-bidi` 승인 항목(ADR-0016 Decision 2).
- `Cargo.lock` — `unicode-bidi` 추가(자동).
- `docs/task-reports/X.9.2.md`(신규), 계획서(X.9.2 `[ ]`→`[x]`).

## 구현 요약

- **style span 입력**: `StyleSpan { range(byte), style, language }`가 paragraph를 빈틈·겹침 없이 덮어야
  합니다(빈 텍스트는 span 없음 또는 단일 빈 span). 위반은 `ItemizeError`로 구분합니다. 문서 모델 style
  run(VI.7.2, scalar index)은 호출자가 byte 범위로 바꿔 넘깁니다(`text` crate는 `document`에 의존하지
  않음).
- **script**: shaping 엔진과 같은 Unicode 데이터(`harfrust::unicode::script`). Inherited는 앞 문자,
  Common은 앞 실제 script, 닫는 괄호는 짝 여는 괄호 시점 script(UAX #9 BidiBrackets, stack 깊이 63
  상한), 선행 Common은 뒤 첫 실제 script, 실제 script가 없으면 Common.
- **방향**: `unicode-bidi`로 계산한 byte별 resolved level(L1 이전)이 바뀌는 곳에서 나눕니다. 기준
  방향(`LeftToRight`·`RightToLeft`·`Auto`)은 명시 입력입니다.
- **언어·style**: span 경계와 언어 변경에서 나눕니다. 결과는 논리 순서이고 입력만으로 정해집니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 / 결과 |
|---|---|
| `cargo fmt -p stellar-text` | 0 |
| `cargo test -p stellar-text` | 0 (53 unit passed, 0 failed) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 (경고 0) |
| `cargo fmt -p stellar-text -- --check` | 0 |
| `cargo run -q -p xtask -- validate-crate-graph` | 0 |
| `cargo run -q -p xtask -- validate-dependencies` | **65 (`unused-approval` ×2)** — X.9.1 보고서의 validator 범위 문제와 같음 |
| workspace 직접 외부 의존성 대조(scratch 스크립트) | 직접 의존 = 승인 목록(harfrust, unicode-bidi, yaml-rust2) |

| 검사 | 결과 |
|---|---|
| **Latin+Arabic+한글 mixed fixture**(`latin_arabic_hangul_mixed_fixture`): "Hello مرحبا 안녕하세요!" → Latn/0 `0..6`, Arab/1 `6..16`, Arab/0 `16..17`(중립 공백), Hang/0 `17..33` | 통과 |
| RTL 문단의 아랍어·숫자(AN, level 2)·Latin(level 2) 분할 | 통과 |
| style·언어 경계에서 분할 | 통과 |
| Common·Inherited·짝 괄호·선행 Common 해석, 괄호 500단 중첩도 상한 안에서 처리 | 통과 |
| 결정론·전체 덮음·연속성 | 통과 |
| span 누락·빈틈·겹침·문자 경계 위반·미완 덮음·빈 텍스트 규칙 | 통과 |
| BiDi level 기본 규칙(L·R 사이 중립, AN, Auto, 문단 구분 문자), Unicode 데이터 버전 16.0 고정 | 통과 |

## 차이(작업 지시와 다른 점)

- 방향 itemization에 BiDi resolved level이 필요해 `unicode-bidi` 도입과 level 해석(`bidi.rs`)을 이 작업에서
  했습니다. 줄 단위 재배치(L1·L2)와 visual↔logical cluster mapping은 X.9.3 범위로 남겼습니다.

## 남은 위험 / 한계

- `cargo xtask validate-dependencies`는 xtask 직접 의존성만 보므로 제품 crate 승인 항목을
  `unused-approval`로 보고합니다(X.9.1 보고서·ADR-0016 Consequences). validator 범위 수정이 필요합니다.
- Han·Hiragana·Katakana는 서로 다른 item이 됩니다(일본어 혼합 문장은 item 수가 늘어남). shaping 결과에는
  영향이 없지만 성능 측정 대상입니다.
