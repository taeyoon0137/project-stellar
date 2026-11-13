# 작업 보고서: VI.4.1 Fragment zero-size marker

- WBS: VI.4.1
- 작업명: 별도 렌더·layout 필드 없는 zero-size marker Fragment payload
- 상태: 완료 (`[x]`)
- 선행: VI.2.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/fragment.rs`(신규) — `FragmentPayload`(unit struct),
  `FragmentError`.
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.4.1.md`(신규), 계획서(VI.4.1 `[ ]`→`[x]`).

## 구현 요약

- `FragmentPayload`는 필드 없는 unit struct(`size_of == 0`)입니다. 공통 node
  column(VI.2.2) 외에 저장하는 값이 없으므로 canonical bytes는 항상 비어 있고,
  디코딩은 비어 있지 않은 payload를 `UnexpectedPayload`로 거부합니다.
- 좌표계를 만들지 않는다는 성질은 VI.3.2 capability validator가 이미 강제합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 70 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| serialized bytes가 metadata 외 추가 필드 없음(빈 bytes, zero-size)(`serialized_payload_has_no_fields`) | 통과 |
| 비어 있지 않은 payload 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 금지 속성 validation(VI.4.2), bounds helper(VI.4.3), Frame 변환 계획(VI.4.4)이
  후속입니다.
