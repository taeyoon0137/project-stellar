# 작업 보고서: II.2.6 semantic-diff·semantic-merge·git-porcelain 골격

- WBS: II.2.6
- 작업명: 의미 diff/merge·Git porcelain crate 골격 생성
- 상태: 완료 (`[x]`)
- 선행: II.2.2, II.2.5 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/{semantic-diff,semantic-merge,git-porcelain}` (각 Cargo.toml + src/lib.rs)
- `Cargo.toml` (members), `Cargo.lock`
- `docs/task-reports/II.2.6.md`, 계획서(II.2.6 `[ ]`→`[x]`)

## 구현 요약

- semantic-diff → document, core-types. semantic-merge → semantic-diff.
- git-porcelain → core-types만. document를 직접 수정하지 않고 빈 `SnapshotSource` trait에만 의존.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| `cargo tree -p stellar-git-porcelain` | core-types만(document 비의존) |
| `cargo check --workspace --locked` | OK |
| fmt / clippy --locked | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 없음.
