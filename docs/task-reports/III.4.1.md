# 작업 보고서: III.4.1 ColorSpace와 Color(비연관 알파)

- WBS: III.4.1
- 작업명: `ColorSpace`(sRGB·Display P3) 정의 + 비연관 알파 `Color` 저장·round-trip
- 상태: 완료 (`[x]`)
- 선행: III.3.1(`FiniteF32`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/color.rs` (신규: `ColorSpace`, `Color`, `ColorError`, round-trip, 단위테스트)
- `crates/core-types/src/typed_value.rs` (`TypedValue::Color` placeholder → 정식 `Color` 교체, `ValueDecodeError::Color`, Reader 제네릭 `take_array` 도입)
- `crates/core-types/src/lib.rs` (`pub mod color;`)
- `docs/task-reports/III.4.1.md`(신규), 계획서(III.4.1 `[ ]`→`[x]`)

## 구현 요약

- `ColorSpace`: `Srgb`(tag `0x01`), `DisplayP3`(tag `0x02`). tag는 저장 포맷 계약.
- `Color`: 색공간 + **비연관 알파(unassociated/straight)** RGBA. 각 채널은 `FiniteF32`(비유한 거부, `-0.0`→`0.0`). 알파는 `[0, 1]` 범위로 강제. RGB 채널은 wide-gamut·확장 범위 저작을 막지 않으려고 `[0, 1]`로 제한하지 않음(감만 매핑은 렌더 단계 소관).
- 직렬화: 고정 17바이트(`Color::BYTE_LEN`) = 색공간 tag 1 + 채널 4×(LE 4바이트). `to_bytes`/`from_bytes`로 정확히 round-trip. 디코드는 미지 색공간·비유한 채널·알파 범위 위반을 거부.
- `TypedValue::Color`를 III.3.5의 opaque placeholder에서 정식 `Color`로 교체. type tag `0x09`는 그대로 유지. 디코드 실패는 `ValueDecodeError::Color(ColorError)`로 전달.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-core-types` (72 unit + 8 doctest) | 0 |
| `cargo fmt -p stellar-core-types -- --check` | 0 |
| `cargo clippy -p stellar-core-types --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| alpha 0·1 경계 fixture round-trip | 통과 |
| gamut 모서리(채널 0/1, sRGB·P3) fixture round-trip | 통과 |
| 색공간 tag golden(`0x01`/`0x02`) + tag round-trip | 통과 |
| 확장 범위 RGB 채널 보존(클램프 없음) | 통과 |
| 비유한 채널·알파 범위 위반·미지 색공간 tag 거부 | 통과 |
| `-0.0` 채널 정규화 | 통과 |
| `TypedValue::Color` 정식 타입 round-trip(tag `0x09`, 18바이트) 및 잘못된 payload 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 지시는 `ColorSpace`·`Color` 정의만 요구하지만, III.3.5에서 `TypedValue::Color`를 "후속 작업이 정식 타입으로 교체"한다고 명시했으므로 이번 작업에서 placeholder를 실제 `Color`로 교체했습니다. type tag `0x09`는 유지되어 저장 포맷 계약을 깨지 않습니다. 남은 placeholder는 Gradient(III.4.3)·Shadow(III.4.4)·Typography(VI.7.2/X.8.1)입니다.
- RGB 채널 범위는 의도적으로 제한하지 않았습니다(확장 gamut 저작 보존). 알파만 `[0, 1]`로 강제했습니다.

## 남은 위험 / 한계

- 색공간 간 변환(sRGB↔P3, 선형화)은 III.4.2 소관이며 본 작업에 포함하지 않았습니다. 본 작업은 저장 표현과 round-trip만 다룹니다.
- `Gradient`/`Shadow`/`Typography`는 여전히 opaque placeholder입니다. 각 정식 작업에서 교체 예정.
