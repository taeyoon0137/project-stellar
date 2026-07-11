# 작업 보고서: VI.3.1 Frame payload

- WBS: VI.3.1
- 작업명: Frame payload — size, local transform, clipping, background, border, corner radius, layout mode 참조
- 상태: 완료 (`[x]`)
- 선행: VI.2.2, III.3.3, III.4.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/frame.rs`(신규) — `FramePayload`, `Clipping`,
  `CornerRadius`, `Border`, `FrameError`, binary encode/decode.
- `crates/core-types/src/value.rs` — `Transform2D::components` 접근자 추가
  (직렬화용).
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.3.1.md`(신규), 계획서(VI.3.1 `[ ]`→`[x]`).

## 구현 요약

- **필드**: `size: Size2`(음수 불가 타입, III.3.3), `transform: Transform2D`
  (finite 전용), `clipping`(Disabled/FrameBounds — PathRef 형식은 VI.3.4),
  `background: Option<Color>`(III.4.1), `border: Option<Border>`(색+비음수
  두께), `corner_radius: CornerRadius`(개별 4귀퉁이, 비음수),
  `layout_mode: Option<StableId>`(layout 정의는 Phase X).
- **기본값**: 크기 0, 항등 변환, bounds clip, 배경·테두리·layout 없음, radius 0.
- **binary 인코딩**: canonical 규칙(LE 고정 폭 + presence flag). 디코딩은
  NaN·음수 size·음수 radius·알 수 없는 tag·절단·trailing을 전부 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 53 passed, 0 failed |
| `cargo test -p stellar-core-types` | 115 passed, 0 failed |
| `cargo fmt -- --check`(두 crate) | 통과 |
| `cargo clippy --all-targets -- -D warnings`(두 crate) | 경고 0 |

| 검사 | 결과 |
|---|---|
| 기본값 round-trip + 재인코딩 byte-identical(`default_payload_round_trips`) | 통과 |
| 전체 필드 round-trip, 음수 radius·두께 거부, malformed 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- containing block capability validator(VI.3.2), size/transform 구분 정책 검증
  (VI.3.3), PathRef clip(VI.3.4), property metadata registry(VI.3.5)가
  후속입니다.
