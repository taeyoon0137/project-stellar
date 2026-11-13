# 작업 보고서: VI.9.3 modifier enabled flag와 typed payload

- WBS: VI.9.3
- 작업명: 각 modifier에 enabled flag와 typed payload 정의 — disabled는 평가에서 skip, 저장에는 유지
- 상태: 완료 (`[x]`)
- 선행: VI.9.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/modifier.rs` — `ModifierPayload`(12종 typed), `BooleanOp`,
  `BlendMode`, `ModifierEntry.{enabled, payload}`, `set_enabled`/`set_payload`/
  `active_entries`/`effective_opacity`, payload 인코딩·검증,
  `ModifierError::{KindPayloadMismatch, InvalidPayload}`.
- `docs/task-reports/VI.9.3.md`(신규), 계획서(VI.9.3 `[ ]`→`[x]`).

## 구현 요약

- **typed payload**: Transform(행렬), Boolean(op + 피연산 NodeRef 목록), StrokeExpand
  (폭), Corner(radius), Mask/Clip(원본 NodeRef), Fill(색), Stroke(색·두께), Shadow
  (색·오프셋·blur), Blur(반경), Blend(mode), Opacity(0..=1). 모든 float는 finite 전용,
  폭·반경은 비음수, 불투명도는 범위 검증합니다. 항목 kind와 payload 종류 불일치는
  거부합니다. `default_for(kind)`는 새 modifier 기본값(참조형은 NIL 참조)입니다.
- **enabled**: 비활성 modifier는 `active_entries()` 평가에서 건너뛰지만 stack에서
  제거되지 않고 enabled=false로 저장·복원됩니다.
- **인코딩**: 항목마다 kind + enabled(u8) + 길이 프리픽스 payload. 디코딩은 kind별
  길이 일치와 값 범위를 검증합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 136 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| enable/disable 결과 fixture(불투명도 0.5×0.4 → 비활성 시 0.5 → 재활성 0.2, 저장 유지)(`disabled_modifier_is_skipped_but_kept`) | 통과 |
| 12종 payload round-trip + byte-identical(`every_kind_payload_round_trips`) | 통과 |
| 범위 밖 값·kind 불일치 거부(`payload_validation_and_kind_mismatch`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- Boolean·Mask·Clip 참조의 ReferenceIndex 등록(VI.9.4)과 cycle 검출(VI.9.5)이
  후속입니다. 실제 geometry/paint 평가는 renderer·scene(Phase XI) 소관입니다.
