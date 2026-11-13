# 작업 보고서: VI.8.1 Image payload

- WBS: VI.8.1
- 작업명: Image payload — AssetRef, fit mode, crop rect, sampling mode, color space override
- 상태: 완료 (`[x]`)
- 선행: VI.2.2, III.2.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/image.rs`(신규) — `ImagePayload`, `FitMode`, `SamplingMode`,
  `CropRect`, `ImageError`, encode/decode.
- `crates/core-types/src/value.rs` — `FiniteF32::ONE` 상수 추가.
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.8.1.md`(신규), 계획서(VI.8.1 `[ ]`→`[x]`).

## 구현 요약

- **필드**: `asset: AssetRef`(III.2.3 강타입 참조), `size: Size2`(배치 상자 —
  VI.8.2의 placeholder bounds), `fit`(Fill/Contain/Cover/None, 기본 Contain),
  `crop: CropRect`(원본 기준 정규화 좌표, 기본 전체), `sampling`(Linear/Nearest),
  `color_space_override: Option<ColorSpace>`(III.4.1 tag 재사용).
- **인코딩**: 고정 43바이트(asset id 16 + size 8 + fit 1 + crop 16 + sampling 1 +
  색공간 1, 0 = override 없음). 알 수 없는 tag·NaN·음수 size·길이 불일치를 거부합니다.
  crop 범위 검증은 VI.8.3.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document -p stellar-core-types` | 126 + 116 passed, 0 failed |
| `cargo fmt -- --check`(두 crate) | 통과 |
| `cargo clippy --all-targets -- -D warnings`(두 crate) | 경고 0 |

| 검사 | 결과 |
|---|---|
| 기본·전체 필드 payload round-trip + 재인코딩 byte-identical(`default_and_full_payloads_round_trip`) | 통과 |
| malformed 거부(길이·fit·색공간·NaN) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 누락 asset 진단(VI.8.2)과 crop 범위 정책(VI.8.3)이 후속입니다.
