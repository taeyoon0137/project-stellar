# 작업 보고서: III.2.4 UnresolvedRef

- WBS: III.2.4
- 작업명: dangling 참조 `UnresolvedRef`(원래 ID·예상 타입·발견 위치 보존)
- 상태: 완료 (`[x]`)
- 선행: III.2.3 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/reference.rs` (`ExpectedRefType`, `UnresolvedRef`, 각 Ref의 `unresolved_at`, 단위테스트)
- `docs/task-reports/III.2.4.md`, 계획서(III.2.4 `[ ]`→`[x]`)

## 구현 요약

- `UnresolvedRef { original_id: StableId, expected: ExpectedRefType, location: PropertyPath }`. 접근자로 세 정보를 그대로 읽음.
- `ExpectedRefType { Node, Token, Component, Asset }`로 어떤 참조였는지 보존.
- 각 강한 타입 참조에 `unresolved_at(location)`을 추가해, resolve 실패 시 기대 타입과 위치를 붙여 `UnresolvedRef`로 변환.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| resolve 실패 → UnresolvedRef, 세 정보(id/expected/location) 보존 | 통과 |
| 보존 정보로 재구성 시 동일(정보 손실 없는 round-trip) | 통과 |
| 각 참조의 unresolved_at가 올바른 ExpectedRefType 부여 | 통과 |
| `cargo test -p stellar-core-types`(32 unit + 6 doctest) / fmt / clippy `-D warnings` | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- "발견 위치"는 `PropertyPath`(III.2.2)로 표현했습니다.
