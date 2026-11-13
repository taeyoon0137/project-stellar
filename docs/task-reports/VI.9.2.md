# 작업 보고서: VI.9.2 초기 modifier kind

- WBS: VI.9.2
- 작업명: 초기 modifier kind(Transform, Boolean, StrokeExpand, Corner, Mask, Clip, Fill, Stroke, Shadow, Blur, Blend, Opacity) 정의
- 상태: 완료 (`[x]`)
- 선행: VI.9.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/modifier.rs` — `ModifierKind`(12종, `repr(u16)` 1~12),
  `ModifierEntry.kind`, `push_back(id, kind)`, 인코딩에 kind(u16) 추가.
- `docs/task-reports/VI.9.2.md`(신규), 계획서(VI.9.2 `[ ]`→`[x]`).

## 구현 요약

- **numeric 할당(golden 고정)**: Transform=1, Boolean=2, StrokeExpand=3, Corner=4,
  Mask=5, Clip=6, Fill=7, Stroke=8, Shadow=9, Blur=10, Blend=11, Opacity=12. 알 수 없는
  값은 디코딩에서 거부합니다.
- 각 항목이 kind를 가지며 stack 인코딩에 u16으로 포함됩니다(VI.9.1 형식 확장 — 아직
  저장된 파일이 없는 단계라 호환성 영향 없음).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 133 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| numeric kind golden test(12종 값·이름·round-trip, 0·13 거부)(`golden_numeric_modifier_kinds`) | 통과 |
| bytes의 알 수 없는 kind 거부, 기존 VI.9.1 round-trip(kind 포함) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- kind별 typed payload와 enabled flag(VI.9.3)가 후속입니다.
