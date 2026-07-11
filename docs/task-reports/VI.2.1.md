# 작업 보고서: VI.2.1 NodeKind

- WBS: VI.2.1
- 작업명: `NodeKind`(Frame, Fragment, Shape, Path, Text, Image, ComponentInstance) 정의
- 상태: 완료 (`[x]`)
- 선행: VI.1.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/node_kind.rs`(신규) — `NodeKind` enum(7종),
  `as_u16`/`from_u16`/`name`/`ALL`.
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.2.1.md`(신규), 계획서(VI.2.1 `[ ]`→`[x]`).

## 구현 요약

- **숫자 할당(golden 고정)**: Frame=1, Fragment=2, Shape=3, Path=4, Text=5,
  Image=6, ComponentInstance=7 (`repr(u16)`). 알 수 없는 값은 `from_u16`이
  `None`으로 거부합니다.
- **ComponentDefinition 제외**: 컴포넌트 정의는 scene node가 아니라 문서
  resource(ComponentLibrary chunk, V.6.1)로 유지되며 scene에는
  ComponentInstance만 배치됩니다 — enum에 없음을 테스트로 고정했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 32 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| numeric kind golden test(7종 값·이름·round-trip)(`golden_numeric_kinds_are_stable`) | 통과 |
| 알 수 없는 값 거부, ComponentDefinition 부재 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 공통 Node column에 kind 추가(VI.2.2)가 후속입니다.
