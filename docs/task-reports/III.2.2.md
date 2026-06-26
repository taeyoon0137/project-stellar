# 작업 보고서: III.2.2 PropertyPath

- WBS: III.2.2
- 작업명: `PropertyPath = StableId + Vec<PropertyKey> + optional index/key segment`, canonical 비교·정렬
- 상태: 완료 (`[x]`)
- 선행: III.2.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/core-types/src/property.rs` (`PropertyPath`, `PathSegment`, 단위테스트)
- `docs/task-reports/III.2.2.md`, 계획서(III.2.2 `[ ]`→`[x]`)

## 구현 요약

- `PropertyPath { root: StableId, keys: Vec<PropertyKey>, tail: Option<PathSegment> }`. `PathSegment`는 `Index(u32)` 또는 `Key(String)`.
- `Eq`/`Ord`/`Hash` 파생으로 canonical 비교·정렬 가능.
- `to_canonical_bytes()`: root(16) ++ keys_len(u32 BE) ++ keys(u32 BE) ++ tail(discriminant + payload). 길이 prefix로 모호성 없는 canonical 직렬화 — 동일 의미 경로는 동일 바이트열.
- 빌더(`new`/`with_keys`/`push`/`with_index`/`with_key_segment`) + 접근자(`root`/`keys`/`tail`).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 동일 의미 경로(두 빌더 경로) → 동일 bytes + Eq | 통과 |
| 서로 다른 경로(길이/index/key 차이) → 다른 bytes | 통과 |
| canonical 정렬 결정성 | 통과 |
| 접근자 컴포넌트 반환 | 통과 |
| `cargo test -p stellar-core-types`(28 unit + 3 doctest) / fmt / clippy `-D warnings` | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 없음.
