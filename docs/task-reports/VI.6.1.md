# 작업 보고서: VI.6.1 Path command

- WBS: VI.6.1
- 작업명: Path command는 MoveTo, LineTo, CubicTo, Close만 정의 — Quadratic은 import에서 Cubic으로 정규화
- 상태: 완료 (`[x]`)
- 선행: III.3.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/path.rs`(신규) — `PathCommand`(4종), `quadratic_to_cubic`,
  `encode_commands`/`decode_commands`, `PathError`.
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.6.1.md`(신규), 계획서(VI.6.1 `[ ]`→`[x]`).

## 구현 요약

- **command 4종**: MoveTo, LineTo, CubicTo(c1, c2, to), Close. 좌표는 finite 전용
  `Vec2`(III.3.3)입니다. 문서 모델에 Quadratic은 존재하지 않습니다.
- **Quadratic 정규화**: `quadratic_to_cubic`은 degree elevation
  (`c1 = from + ⅔(control − from)`, `c2 = to + ⅔(control − to)`)으로 정확히 같은
  곡선의 Cubic을 만듭니다(import 단계 사용).
- **인코딩**: `u32` 개수 + (tag 1~4 + LE f32 좌표). 알 수 없는 tag·NaN·절단·
  trailing bytes를 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 87 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| command round-trip + 재인코딩 byte-identical(`command_round_trip`) | 통과 |
| Quadratic→Cubic 동치(11개 t 샘플 오차 < 1e-3) | 통과 |
| malformed 거부(절단·tag·NaN·trailing) | 통과 |

## 차이(작업 지시와 다른 점)

- 계획서 순서상 VI.5.3 이전에 수행했습니다. VI.6.1의 선행(III.3.3)은 충족되어
  있고, Shape path 평가(VI.5.3·VI.5.5·VI.5.6)가 이 command 타입을 필요로 합니다.

## 남은 위험 / 한계

- subpath·winding rule과 MoveTo 누락 검증(VI.6.2)이 후속입니다.
