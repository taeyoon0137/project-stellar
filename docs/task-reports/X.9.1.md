# 작업 보고서: X.9.1 HarfBuzz 호환 shaping 엔진 등록과 안전 wrapper

- WBS: X.9.1
- 작업명: 승인 HarfBuzz 계열 shaping 엔진을 dependencies policy에 등록하고 `text` crate 안전 wrapper 구현
- 상태: 완료 (`[x]`)
- 선행: I.3.2(`[x]`), X.8.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (Apple Silicon arm64, 순수 Rust 로컬 검증).

## 변경 파일 목록

- `crates/text/src/shaping.rs`(신규) — `FontFace`, `ShapeRequest`, `shape`, `ShapedRun`/`ShapedGlyph`,
  `GlyphId`, `ScriptTag`, `LanguageTag`, `TextDirection`, `LineMetrics`, 오류 타입, 단위 테스트.
- `crates/text/src/test_fonts.rs`(신규, `#[cfg(test)]`) — 합성 OpenType 폰트 생성기(head·hhea·maxp·
  hmtx·cmap format 12, 선택 GSUB `liga`, `fvar`/`HVAR`).
- `crates/text/src/lib.rs` — `#![forbid(unsafe_code)]`, 모듈 선언.
- `crates/text/Cargo.toml` — `harfrust = "=0.15.0"`(default features 끔, `std`만).
- `config/dependencies.yaml` — `harfrust` 승인 항목.
- `docs/adr/ADR-0016-text-shaping-dependencies.md`(신규, Accepted).
- `Cargo.lock` — `harfrust`와 전이 의존성 자동 갱신(`cargo test`가 생성).
- `docs/task-reports/X.9.1.md`(신규), 계획서(X.9.1 `[ ]`→`[x]`).

## 구현 요약

- **엔진**: HarfBuzz 프로젝트의 순수 Rust 포트 `harfrust` 0.15.0(MIT). C 라이브러리와 FFI가 없어
  raw pointer 경계가 없습니다. `stellar-text`는 `#![forbid(unsafe_code)]`이고 공개 API에 `harfrust`
  타입을 노출하지 않습니다(ADR-0016 Decision 5).
- **`FontFace::from_bytes(bytes, face_index)`**: SFNT만 허용하고, 형식 오류·없는 face index는
  `FontLoadError`로 반환합니다(panic 없음). units per em, glyph 수, 줄 metric(USE_TYPO_METRICS →
  typo, 아니면 hhea → typo → win → 0.8/-0.2em 기본값, 16.16 값은 정수 반올림), cmap 조회를 제공합니다.
- **`shape(face, &ShapeRequest)`**: paragraph 전체 `text`와 byte `range`, 방향, script(ISO 15924
  `ScriptTag`), 언어(BCP 47 `LanguageTag`, canonical 대소문자)를 받습니다. 결과 cluster는 paragraph
  byte offset이고 glyph는 visual 순서, 값은 font unit 정수입니다. 앞뒤 8문자를 shaping 문맥으로
  넘기고, paragraph 시작·끝이면 BOT/EOT flag를 켭니다. 범위 오류는 `ShapeError`로 구분합니다.
- **테스트 폰트**: 저장소에 외부 폰트 파일을 넣지 않고 합성 폰트를 코드로 결정론적으로 만듭니다
  (같은 정의 → 같은 bytes).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 / 결과 |
|---|---|
| `cargo fmt -p stellar-text` | 0 |
| `cargo test -p stellar-text` | 0 (43 unit passed, 0 failed) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 0 (경고 0) |
| `cargo fmt -p stellar-text -- --check` | 0 |
| `MallocStackLogging=1 leaks --atExit -- target/debug/deps/stellar_text-<hash> shaping --test-threads=1` | 0 (shaping 테스트 9개 통과, "0 leaks for 0 total leaked bytes") |
| `cargo run -q -p xtask -- validate-dependencies` | **65 (`unused-approval`)** — 아래 남은 위험 참조 |
| workspace 직접 외부 의존성 대조(scratch 스크립트, `cargo metadata --no-deps`) | 직접 의존 {harfrust, yaml-rust2} = 승인 목록 {harfrust, yaml-rust2} |

| 검사 | 결과 |
|---|---|
| **simple Latin shaping fixture**(`simple_latin_shaping_fixture`): "Hello, wi"의 glyph id·cluster·advance가 합성 폰트 정의와 정확히 일치, offset 0, advance 합 3750 | 통과 |
| 결정론: 폰트를 두 번 로드해 같은 입력을 shaping하면 결과 동일 | 통과 |
| 부분 범위 shaping이 paragraph cluster offset 유지, 빈 범위는 glyph 0개 | 통과 |
| RTL(히브리어) run이 visual 순서(cluster 내림차순) | 통과 |
| 없는 문자는 `.notdef`(glyph 0), cmap 조회 | 통과 |
| 범위 밖·문자 경계 위반·역순 범위가 typed error | 통과 |
| 빈 입력·임의 bytes·없는 face index·잘린 폰트가 panic 없이 오류 | 통과 |
| ScriptTag·LanguageTag 검증과 canonical 형식(`ZH-hant-tw` → `zh-Hant-TW`, 확장 subtag 소문자) | 통과 |
| 합성 폰트 6종 로드·coverage | 통과 |

## 차이(작업 지시와 다른 점)

- **"HarfBuzz binding"**: C HarfBuzz binding 대신 HarfBuzz 알고리즘의 순수 Rust 포트를 썼습니다(제품
  소유자 선호, ADR-0016). raw pointer가 wrapper 밖에 노출되지 않는다는 요구는 `unsafe` 금지와 엔진
  타입 비노출로 구조적으로 충족합니다.
- **leak sanitizer**: Rust LeakSanitizer(`-Zsanitizer=leak`)는 nightly 전용이고 고정 toolchain
  (1.96.0 stable)에서 쓸 수 없어, macOS 기본 누수 검사기 `leaks --atExit`로 shaping 테스트 프로세스를
  검사했습니다(누수 0).

## 남은 위험 / 한계

- **dependency validator 범위**: `cargo xtask validate-dependencies`는 `cargo tree -p xtask`의 직접
  의존성만 수집해(`tools/xtask/src/commands.rs` `direct_dependencies`), 제품 crate의 승인 항목
  `harfrust`를 `unused-approval`로 보고하고 종료 코드 65로 실패합니다. CI `bootstrap.yml`도 이 명령을
  실행합니다. 승인 목록은 workspace 실제 직접 의존성과 정확히 일치하므로, validator가 workspace 전체
  (`cargo tree --workspace` 또는 `cargo metadata`)를 수집하도록 고치는 `Fix:`가 필요합니다. 이 lane의
  허용 경로 밖이라 수정하지 않았습니다. 같은 원인으로 `tools/xtask/src/dependencies.rs`의 단위 테스트
  `real_dependencies_file_parses`(실제 승인 목록을 직접 의존성 `["yaml-rust2"]`만으로 검증)도 실패해 `cargo xtask
  check-core`의 test 단계가 실패합니다(X.10.7 보고서에 실행 결과). 두 곳을 함께 고쳐야 합니다.
- 전이 의존성 중 Zlib(foldhash)·Unicode-3.0(yoke, zerofrom, unicode-ident) 라이선스는 자동 허용
  목록 밖이며 ADR-0016 표에 기록만 했습니다.
- `harfrust`는 0.x입니다. 갱신 시 X.10.7 golden을 다시 검증해야 합니다.

## 후속 확인 (posthoc 리뷰 TS-5: dependency validator 범위 문제 해소)

- 위 "dependency validator 범위" 항목은 main에서 해소되었습니다. `975bf0a`(XXIX.1.2)가 `validate-dependencies`를
  `cargo tree --workspace` 기반 workspace 전체 직접 의존성 검사로 바꿨고, `24728e3`이
  `real_dependencies_file_parses`가 실제 workspace 사용 목록을 쓰도록 고쳤습니다. 이 보고서와 X.9.2·X.9.3·X.9.4
  보고서의 종료 코드 65 기록은 당시 상태입니다.
- ADR-0016 Consequences의 같은 내용("validator 수정이 필요합니다")은 처음 이 후속 확인 때는 고치지 않았고, 이후
  posthoc 수정 리뷰 지적에 따라 해소 기록으로 고쳤습니다(위 두 commit과 현재 종료 코드 0을 적음).

| 명령 (기준 `5d0f2ce` + 이 lane 수정, xtask·의존성 변경 없음) | 결과 |
|---|---|
| `cargo run -q -p xtask -- validate-dependencies` | 0 |
| `cargo test -q -p xtask real_dependencies_file_parses` | 0 (1 passed) |
