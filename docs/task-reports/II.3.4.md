# 작업 보고서: II.3.4 revision newtype

- WBS: II.3.4
- 작업명: DocumentRevision·SceneRevision·StorageGeneration newtype 분리
- 상태: 완료 (`[x]`)
- 선행: II.2.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/core-types/src/revision.rs` (신규)
- `crates/core-types/src/lib.rs` (`pub mod revision;`)
- `docs/task-reports/II.3.4.md`, 계획서(II.3.4 `[ ]`→`[x]`)

## 구현 요약

- 3개 newtype(서로 다른 타입)으로 혼용을 컴파일 단계에서 차단. 같은 타입은 비교·정렬 가능.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 같은 타입 비교·정렬 | 통과 |
| 다른 revision 비교 / 대입 `compile_fail` doctest | 통과(컴파일 실패 확인) |
| `cargo test -p stellar-core-types` / fmt / clippy --locked | 통과(6 unit + 2 doc) |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- compile-fail은 rustdoc `compile_fail` doctest로 구현했습니다.
