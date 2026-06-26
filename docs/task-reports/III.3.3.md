# 작업 보고서: III.3.3 기하 타입

- WBS: III.3.3
- 작업명: `Vec2`, `Size2`, `Rect`, `Insets`, `Transform2D`를 finite 전용으로 구현
- 상태: 완료 (`[x]`)
- 선행: III.3.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/value.rs` (`Vec2`/`Size2`/`Rect`/`Insets`/`Transform2D`, `SizeError`, 단위테스트)
- `docs/task-reports/III.3.3.md`, 계획서(III.3.3 `[ ]`→`[x]`)

## 구현 요약

- 모든 성분은 `FiniteF32` — 생성 시 비유한 값 거부.
- `Size2`: 음수 크기 정책 — 음수는 `SizeError::Negative`, 비유한은 `NonFinite`로 거부. `is_empty`(면적 0).
- `Rect`(origin+size): `is_empty`는 size 면적 0 여부.
- `Insets`(top/right/bottom/left), `Vec2`(x/y).
- `Transform2D`(아핀 a,b,c,d,tx,ty): `determinant`, `apply`, `inverse`(det 0 또는 결과 비유한이면 `None` — 역행렬 불가).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| Vec2/Insets 비유한 거부 | 통과 |
| Size2 음수 크기 정책(Negative/NonFinite 거부) | 통과 |
| Rect 빈 사각형(면적 0) 판정 | 통과 |
| Transform2D 역행렬 가능(스케일 round-trip)·불가능(det 0 → None) | 통과 |
| `cargo test -p stellar-core-types`(44 unit + 8 doctest) / fmt / clippy `-D warnings` | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 음수 크기 정책은 "거부(Err)"로 정했습니다. `Transform2D::inverse`는 행렬식 0뿐 아니라 역변환 성분이 비유한(오버플로)일 때도 `None`을 반환합니다.
