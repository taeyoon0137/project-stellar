# 작업 보고서: III.1.5 IdRemapTable(복제용)

- WBS: III.1.5
- 작업명: 복제용 `IdRemapTable` — 하위 그래프 StableId를 새 ID로 바꾸고 내부 참조 재매핑(외부 참조 유지)
- 상태: 완료 (`[x]`)
- 선행: III.1.2, III.1.4 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/ids.rs` (`IdRemapTable`, 단위테스트)
- `docs/task-reports/III.1.5.md`, 계획서(III.1.5 `[ ]`→`[x]`)

## 구현 요약

- `IdRemapTable`이 old `StableId` → new `StableId` 매핑을 보관.
- `clone_ids(olds, generator, clock)`로 하위 그래프 old id들에 `StableIdGenerator`(III.1.2)로 새 id를 생성·등록. `insert`로 직접 지정도 가능.
- `remap(id)`: 테이블에 있으면(내부 참조) 새 id, 없으면(외부 참조) 원본 그대로. `mapped`/`len`/`is_empty`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 하위 그래프(a,b) 새 id 생성(서로 다르고 원본과 다름) | 통과 |
| 내부 참조(a,b) → 새 id 재매핑 | 통과 |
| 외부 참조(x) → 원본 유지(mapped None) | 통과 |
| 직접 insert 후 remap | 통과 |
| `cargo test -p stellar-core-types`(22 unit + 3 doctest) / fmt / clippy `-D warnings` | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 없음.
