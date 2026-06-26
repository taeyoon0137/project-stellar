# 작업 보고서: III.1.4 StableId↔RuntimeEntityId registry

- WBS: III.1.4
- 작업명: `StableId ↔ RuntimeEntityId` 양방향 registry(중복 StableId 삽입은 오류)
- 상태: 완료 (`[x]`)
- 선행: III.1.3 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/ids.rs` (`IdRegistry`, `RegistryError`, 단위테스트)
- `docs/task-reports/III.1.4.md`, 계획서(III.1.4 `[ ]`→`[x]`)

## 구현 요약

- `IdRegistry`가 `by_stable`/`by_runtime` 두 `HashMap`으로 양방향 lookup을 제공하고 일관성을 유지.
- `insert`는 어느 한쪽이라도 이미 있으면 `RegistryError`(DuplicateStableId / DuplicateRuntimeId) 반환, 실패 시 registry 불변.
- `runtime_for`/`stable_for`/`remove_by_stable`(양방향 제거)/`len`/`is_empty`. 삭제 후 같은 StableId 재삽입 허용.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 삽입 + 양방향 lookup | 통과 |
| 중복 StableId 삽입 오류(registry 불변) | 통과 |
| 중복 RuntimeEntityId 삽입 오류 | 통과 |
| 삭제(양방향 제거) 후 재삽입(새 RuntimeEntityId) | 통과 |
| `cargo test -p stellar-core-types`(20 unit + 3 doctest) / fmt / clippy `-D warnings` | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- plan은 "duplicate StableId 삽입은 오류"만 명시하나, 양방향 일관성을 위해 duplicate RuntimeEntityId 삽입도 오류로 처리했습니다.
