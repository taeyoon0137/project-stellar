# 작업 보고서: VI.5.1 ShapeKind

- WBS: VI.5.1
- 작업명: ShapeKind(Rectangle, Ellipse, Line, Polygon) 정의 — 별도 node kind를 늘리지 않음
- 상태: 완료 (`[x]`)
- 선행: VI.2.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/shape.rs`(신규) — `ShapeKind`(tag 1~4), `ShapeError`.
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.5.1.md`(신규), 계획서(VI.5.1 `[ ]`→`[x]`).

## 구현 요약

- **단일 node kind**: 모든 파라메트릭 도형은 `NodeKind::Shape` 하나 안에서
  `ShapeKind`로 구분합니다. node kind 목록에 도형 종류가 없음을 테스트로
  고정했습니다.
- **tag**: Rectangle=1, Ellipse=2, Line=3, Polygon=4(`repr(u8)`, 저장 포맷의 일부).
  알 수 없는 tag는 `UnknownKind`로 거부합니다. 도형별 파라미터는 VI.5.2~VI.5.5에서
  정의합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 82 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| kind variant round-trip(`kind_variants_round_trip`) | 통과 |
| golden tag·이름 고정, 알 수 없는 tag 거부, node kind 미증가 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 없음.
