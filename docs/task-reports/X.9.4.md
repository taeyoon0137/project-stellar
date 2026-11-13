# 작업 보고서: X.9.4 공통 줄바꿈 기회 계산(locale 명시)

- WBS: X.9.4
- 작업명: line break opportunity를 공통 엔진으로 계산하고 locale 입력을 명시
- 상태: 완료 (`[x]`)
- 선행: X.9.2(이 lane `[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon arm64, 순수 Rust 로컬 검증).

## 변경 파일 목록

- `crates/text/src/line_break.rs`(신규) — `line_break_opportunities`, `LineBreakOptions`(locale 필수),
  `WordBreak`, `LineBreakStrictness`, `BreakOpportunity`, `BreakKind`, 단위 테스트.
- `crates/text/src/lib.rs` — 모듈 선언.
- `crates/text/Cargo.toml` — `unicode-linebreak = "=0.1.5"`.
- `config/dependencies.yaml` — `unicode-linebreak` 승인 항목(Apache-2.0, ADR-0016 Decision 3).
- `Cargo.lock` — 자동 갱신.
- `docs/task-reports/X.9.4.md`(신규), 계획서(X.9.4 `[ ]`→`[x]`).

## 구현 요약

- **기본 알고리즘**: UAX #14(Unicode 15.0)를 `unicode-linebreak`로 계산합니다. offset은 새 줄이 시작될
  byte 위치이고, 0은 제외, 텍스트 끝은 항상 Mandatory, 텍스트 안 강제 줄바꿈 문자(U+2028 등)도 Mandatory입니다.
- **locale 명시**: `LineBreakOptions::new(locale)`만 있고 기본값(`Default`)이 없어 실행 환경 locale을 암묵적으로
  쓸 수 없습니다. 결과는 locale·옵션·텍스트만으로 정해집니다.
- **tailoring**(CSS Text 3 의미): `Strict`(UAX #14 기본, CJ=NS) / `Normal`(기본, CJ를 ID로 → 작은 가나 앞 허용,
  `ja`·`zh` locale이면 U+301C·U+30A0 앞 허용) / `KeepAll`(글자 단위 사이 기회 제거, 공백·구두점 기회 유지).
  추가 기회는 앞 base 문자(CM·ZWJ 건너뜀, LB9)가 UAX #14에서 ID 앞 줄바꿈을 허용하는 클래스일 때만 넣습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 / 결과 |
|---|---|
| `cargo fmt -p stellar-text` | 0 |
| `cargo test -p stellar-text` | 0 (65 unit passed, 0 failed) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 (경고 0) |
| `cargo fmt -p stellar-text -- --check` | 0 |
| `cargo run -q -p xtask -- validate-dependencies` | **65 (`unused-approval` ×4)** — X.9.1 보고서의 validator 범위 문제 |
| workspace 직접 외부 의존성 대조(scratch 스크립트) | 직접 의존 = 승인 목록(harfrust, unicode-bidi, unicode-linebreak, unicode-segmentation, yaml-rust2) |

| 검사 | 결과 |
|---|---|
| **영문·공백 fixture**: 공백·하이픈 뒤 허용/공백 앞 금지, 연속 공백은 마지막 뒤 한 번, NBSP 금지, ZWSP 뒤 허용, 줄 끝 공백, U+2028 Mandatory, 빈 텍스트 | 통과 |
| **한글 fixture**: "한국어 텍스트" Normal → 음절 사이·공백 뒤 `[3,6,10,13,16]`, keep-all → `[10]`, keep-all에서도 쉼표 뒤 유지 | 통과 |
| **일본어 fixture**: "キャット。です" Strict → `[9,15,18]`(작은 가나·。 앞 금지), Normal → `[3,6,9,15,18]` | 통과 |
| **locale 차이**: "東京〜大阪"의 〜 앞은 `ja`·`zh-Hant` Normal에서만 허용, `ko`·`en-US`·Strict는 금지 | 통과 |
| 혼합 script, 결합 부호 앞 금지, 여는 괄호 뒤 작은 가나 앞 금지 | 통과 |
| 정렬·중복 없음·문자 경계·끝 Mandatory·결정론(3 locale) | 통과 |
| Unicode 데이터 버전 15.0 고정 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. (tailoring 범위는 CSS `line-break: strict|normal`과 `word-break: normal|keep-all`로 한정했고 `loose`·
  `break-all`은 넣지 않았습니다.)

## 남은 위험 / 한계

- 줄바꿈 데이터는 Unicode 15.0입니다(BiDi 16.0·grapheme 17.0과 다름). 15.1의 LB28a(브라흐미 계열 정자법
  음절) 등 최신 규칙은 반영되지 않습니다.
- SA(태국어 등 복합 문맥) 문자는 AL로 처리되어 단어 사전 기반 분절이 없습니다.
- `cargo xtask validate-dependencies` 범위 문제는 X.9.1 보고서와 같습니다.
