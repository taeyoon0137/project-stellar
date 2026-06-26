# 작업 보고서: III.4.2 선형 색공간 변환 함수와 reference fixture

- WBS: III.4.2
- 작업명: sRGB 계열 transfer 함수(encode/decode) 정의 + reference fixture + 색상 정확도 예산
- 상태: 완료 (`[x]`)
- 선행: III.4.1(`ColorSpace`·`Color`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/color.rs` (`srgb_decode`/`srgb_encode`, `ColorSpace::decode_channel`/`encode_channel`, reference·round-trip 테스트)
- `config/performance-budgets.yaml` (신규: `color.transfer_abs_tolerance`)
- `docs/adr/ADR-0002-color-transfer-tolerance.md` (신규: transfer 곡선·정확도 예산 결정)
- `docs/task-reports/III.4.2.md`(신규), 계획서(III.4.2 `[ ]`→`[x]`)

## 구현 요약

- IEC 61966-2-1 sRGB transfer 곡선으로 `srgb_decode`(EOTF, 감마→선형)/`srgb_encode`(OETF,
  선형→감마)를 구현했습니다.
- sRGB와 Display P3는 **동일한 sRGB transfer 곡선을 공유**합니다(차이는 primaries이며 본
  작업 범위 밖). `ColorSpace::decode_channel`/`encode_channel`이 색공간별 진입점입니다.
- 색상 정확도 예산 `color.transfer_abs_tolerance = 1.0e-5`를 `config/performance-budgets.yaml`
  에 신설했습니다. 이 파일은 ADR 보호 경로(I.3.5)라 결정 근거를 ADR-0002로 남겼습니다.
- core-types는 std 전용이므로 YAML 의존성 없이 테스트가 해당 키 줄만 파싱해 예산값을 읽고,
  reference·round-trip 오차가 예산 이하임을 강제합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-core-types` (76 unit + 8 doctest) | 0 |
| `cargo fmt -p stellar-core-types -- --check` | 0 |
| `cargo clippy -p stellar-core-types --all-targets -- -D warnings` | 0 |
| `cargo run -p xtask -- validate-adr crates/core-types/src/color.rs config/performance-budgets.yaml docs/adr/ADR-0002-color-transfer-tolerance.md` | 0 |

| 검사 | 결과 |
|---|---|
| sRGB reference fixture(임계점·0.5 표준값) decode/encode 예산 이하 | 통과 |
| encode∘decode round-trip(256단계) 예산 이하 | 통과 |
| sRGB·P3 transfer 곡선 동일성, 색공간 메서드 ↔ standalone 함수 일치 | 통과 |
| 예산값이 8-bit 양자화(1/255)보다 작은 의미 있는 상한 | 통과 |
| 보호 경로(`config/performance-budgets.yaml`, core-types) + ADR 동반 시 ADR 정책 통과 | 통과 |

## 차이(작업 지시와 다른 점)

- `config/performance-budgets.yaml`이 존재하지 않아 신규 생성했습니다. 이 파일은 ADR 보호
  경로이므로 색상 정확도 예산 결정을 ADR-0002로 함께 기록했습니다. 같은 PR 범위에 포함될
  core-types 변경(III.3.5·III.4.1 포함)도 이 ADR이 ADR 정책상 커버합니다.
- "선형 색공간 변환"의 범위를 transfer(encode/decode)로 한정하고 색역(primary) 행렬 변환은
  후속 작업으로 분리했습니다("encode/decode reference fixture" 기준에 부합). ADR-0002에 명시.

## 남은 위험 / 한계

- 색역(sRGB↔P3 primary) 변환과 그 정확도 예산은 아직 없습니다. 추가될 때 ADR-0002를
  보강하거나 새 ADR로 기록합니다.
- transfer 함수는 `[0,1]` 정규화 채널을 가정합니다. 확장 범위(음수/>1) 입력에 대한 mirror
  처리는 구현하지 않았습니다(현재 reference·round-trip 검증 범위 밖).
- 타이밍 예산(T0~T5)은 XVI.4.1에서 같은 파일에 추가될 예정입니다.
