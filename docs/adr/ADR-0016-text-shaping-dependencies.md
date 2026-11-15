# ADR-0016: 텍스트 shaping·BiDi·줄바꿈 외부 의존성

## Status

Accepted (2026-11-14, 제품 소유자 외부 crate 승인에 따른 구현 기록)

## Context

기술 아키텍처 §11.1은 디자인 결과 텍스트에 "HarfBuzz 기반 shaping, 공통 Unicode BiDi 처리, 공통
줄바꿈, 명시적 fallback, 공통 폰트 메트릭"을 요구하고, §3.1·§11.1은 동일 폰트·설정에서 플랫폼 간
glyph 배치와 줄바꿈이 일치해야 한다고 정합니다. 구현 계획 X.9.1은 "승인된 HarfBuzz binding을
dependencies policy에 등록하고 `text` crate에 안전 wrapper를 구현합니다. raw pointer는 wrapper 밖에
노출하지 않습니다"를, X.9.3·X.9.4는 Unicode BiDi(UAX #9)와 줄바꿈 기회(UAX #14)를 요구합니다.

제약:

- 의존성 정책(I.3.2·I.3.3): 직접 의존성은 `config/dependencies.yaml` 승인 목록에 있어야 하고,
  MIT·Apache-2.0·BSD·ISC는 자동 허용, GPL 계열은 거부, 그 밖은 별도 승인이 필요합니다.
- crate 정책(II.2.8): `stellar-text`는 내부 crate 중 `stellar-core-types`에만 의존합니다.
- 결정론: OS 글꼴·shaping API(CoreText, DirectWrite)는 플랫폼마다 결과가 달라 쓸 수 없습니다.
- 2026-11-14 제품 소유자는 잘 관리되는 MIT/Apache 외부 crate 추가를 승인하고, 순수 Rust
  HarfBuzz 호환 shaper(결정론, C toolchain 불필요)를 선호한다고 정했습니다.

## Decision

1. **shaping: `harfrust` `=0.15.0` (MIT)**. HarfBuzz 프로젝트가 관리하는 HarfBuzz shaping
   알고리즘의 순수 Rust 포트입니다. crate가 `#![forbid(unsafe_code)]`이고 C 라이브러리가 없어
   raw pointer 경계 자체가 생기지 않습니다. `default-features = false, features = ["std"]`로
   `icu`·`libm` 선택 기능은 끕니다.
   - 폰트 파싱·cmap·line metric·variation instance는 `harfrust`가 re-export하는 `read-fonts`
     model(`harfrust::font`)로 접근합니다. `read-fonts`를 직접 의존성으로 추가하지 않습니다.
   - 문자 script 속성은 shaping과 같은 데이터인 `harfrust::unicode::script`를 씁니다(별도
     `unicode-script` crate 없음).
2. **BiDi: `unicode-bidi` `=0.3.18` (MIT OR Apache-2.0)**. Servo 프로젝트의 UAX #9 구현(Unicode
   16.0)입니다. `default-features = false, features = ["hardcoded-data", "std"]`로 serde·flame
   기능은 끕니다. X.9.2(방향 itemization)에서 도입합니다.
3. **줄바꿈: `unicode-linebreak` `=0.1.5` (Apache-2.0)**. UAX #14 기본 알고리즘(Unicode 15.0)이며
   SA(복합 문맥) 문자는 AL로 해석합니다(사전 기반 태국어 등 분절 없음). locale별 tailoring은
   `stellar-text`가 결과 위에 명시적으로 적용합니다. X.9.4에서 도입합니다.
4. **grapheme cluster: `unicode-segmentation` `=1.13.3` (MIT OR Apache-2.0)**. UAX #29 extended
   grapheme cluster(Unicode 17.0, 2026-06-01 공개, crate가 `deny(unsafe_code)`)로 BiDi 재배치·
   fallback·줄바꿈·caret이 cluster를 쪼개지 않게 합니다. X.9.3에서 도입합니다.
5. **wrapper 경계**: `stellar-text` 공개 API는 위 crate의 타입을 노출하지 않습니다(`GlyphId`,
   `ScriptTag`, `LanguageTag`, `TextDirection` 등 자체 타입). `stellar-text`도
   `#![forbid(unsafe_code)]`입니다. 엔진을 바꿔도 호출자 API는 유지됩니다.
6. **버전 고정**: Cargo에 `=` 정확 버전을 쓰고, 도입 시점에 공개 후 2주 이상 지난 최신 안정
   버전을 고릅니다(예: `harfrust` 0.16.0은 2026-11-12 공개라 2026-10-22의 0.15.0을 씀).
7. **결정론 계약**: shaping 결과는 font unit 정수 advance·offset과 glyph id이고, px 변환은
   `stellar-text`가 정수 산술(1/64 px subpixel, 레이아웃 `LayoutUnit`과 같은 격자)로 합니다. 각
   crate의 Unicode 데이터 버전(BiDi 16.0, 줄바꿈 15.0, grapheme 17.0)은 테스트로 고정해, 갱신이
   golden 변경과 함께 의도적으로만 일어나게 합니다.
8. **glyph outline**: 캔버스 그리기용 unhinted 외곽선은 shaping과 같은 face instance에서
   `harfrust::font`의 outline 그리기로 얻습니다(`stellar_text::outline`). 새 의존성을 추가하지 않습니다.

### 전이 의존성과 라이선스(2026-11-14 `cargo metadata` 기준, `harfrust` 0.15.0)

| crate | 라이선스 |
|---|---|
| read-fonts 0.46.0, font-types 0.12.7, bitflags, smallvec, once_cell, stable_deref_trait, proc-macro2, quote, syn | MIT OR Apache-2.0 |
| bytemuck, bytemuck_derive | Zlib OR Apache-2.0 OR MIT |
| synstructure | MIT |
| foldhash 0.2.0 | Zlib |
| yoke, yoke-derive, zerofrom, zerofrom-derive | Unicode-3.0 |
| unicode-ident | (MIT OR Apache-2.0) AND Unicode-3.0 |

Zlib·Unicode-3.0은 허용형(permissive) 라이선스지만 I.3.3 자동 허용 목록 밖입니다. 현재 정책
validator는 직접 의존성만 검사하므로 이 표로 검토 기록을 남깁니다.

## Alternatives

- **C HarfBuzz binding(`harfbuzz-sys`, `harfbuzz_rs`)**: 기준 구현이지만 C/C++ toolchain과
  플랫폼별 빌드가 필요하고 `unsafe` FFI·raw pointer 경계를 직접 관리해야 합니다. 순수 Rust 포트가
  같은 알고리즘을 제공하므로 채택하지 않습니다.
- **`rustybuzz` 0.20.1 (MIT)**: 순수 Rust지만 유지보수 모드이고 후속 프로젝트가 `harfrust`입니다.
- **`swash`**: 자체 shaper로 HarfBuzz 호환을 목표로 하지 않습니다.
- **OS shaping(CoreText, DirectWrite)**: 플랫폼 간 결과가 달라 §11.1 결정론을 깨뜨립니다.
- **ICU4X(`icu_segmenter`, `icu_properties`)**: 기능은 넓지만 Unicode-3.0 직접 의존이라 별도
  승인이 필요하고 데이터 크기가 큽니다. 필요해지면 별도 ADR로 검토합니다.
- **자체 shaping·BiDi·UAX #14 구현**: 표준 적합성 검증 비용이 크고 오류 위험이 높습니다.

## Consequences

- 긍정: C toolchain 없이 macOS·Windows에서 같은 코드로 같은 결과를 냅니다. `unsafe`가 없습니다.
- 긍정: HarfBuzz와 같은 shaping 규칙(OpenType GSUB/GPOS, 복합 script shaper)을 씁니다.
- 부정: `harfrust`는 0.x이고 릴리스가 잦습니다. 갱신은 golden(X.10.7) 재검증과 함께 합니다.
- 부정: crate마다 Unicode 데이터 버전이 다릅니다(BiDi 16.0, 줄바꿈 15.0, grapheme 17.0). 최신
  문자의 줄바꿈 속성은 15.0 기준입니다.
- 해소된 부정(기록): 이 ADR 채택 당시 정책 validator(`cargo xtask validate-dependencies`)는
  `cargo tree -p xtask`의 직접 의존성만 수집해(`tools/xtask/src/commands.rs`), 제품 crate의 승인
  항목을 `unused-approval`로 보고했습니다. 실제 승인 목록을 `["yaml-rust2"]`만으로 검증하던 xtask
  단위 테스트(`real_dependencies_file_parses`)도 같은 이유로 실패했습니다. 둘 다 후속 수정으로
  해소되었습니다. `975bf0a`(XXIX.1.2)가 validator를 workspace 전체 직접 의존성 수집
  (`cargo tree --workspace`)으로 넓혔고, `24728e3`이 이 테스트가 실제 workspace 사용 목록을 쓰도록
  고쳤습니다. 2026-11-14 기준 두 검사 모두 종료 코드 0입니다.

## Compatibility

파일 포맷·저장 구조·FFI(C ABI)에 영향이 없습니다. shaping 결과는 저장하지 않는 런타임 값입니다.
공개 Rust API는 `stellar-text`의 자체 타입만 씁니다.

## Performance

shaping은 run 단위로 수행하고 결과를 캐시합니다(X.10.5·X.10.6). 폰트 데이터는 `Arc` 공유로
face당 한 번만 보관합니다. 세부 예산은 Phase X 레이아웃 benchmark에서 측정합니다.

## Security

폰트 파일은 신뢰할 수 없는 입력입니다. `read-fonts`는 bounds-checked 파싱을 하고 `harfrust`는
연산 budget으로 악성 폰트의 과도한 작업을 제한합니다. `stellar-text`는 잘못된 폰트를 오류로
반환하고 panic하지 않으며(잘린·임의 bytes fixture), `unsafe` 코드가 없습니다.

glyph outline도 같은 bounds-checked 파서를 씁니다. glyph 하나의 path 요소(65,536개)와 scratch
메모리(4 MiB)에 상한을 두고, 넘으면 `TooComplex` 오류로 그 glyph를 그리지 않습니다.

## Migration

기존 데이터 마이그레이션은 없습니다. crate를 갱신하거나 교체할 때는 이 ADR(또는 후속 ADR)의 버전
표를 고치고, `config/dependencies.yaml`을 갱신하고, shaping·줄바꿈 golden(X.10.7)을 다시 검증합니다.
