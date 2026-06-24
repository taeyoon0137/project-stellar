# 작업 보고서: II.2.2 document·commands·history 골격

- WBS: II.2.2
- 작업명: `crates/document`·`commands`·`history` 생성, 의존 방향 고정
- 상태: 완료 (`[x]`)
- 선행: II.2.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/document`·`crates/commands`·`crates/history` (각 Cargo.toml + src/lib.rs)
- `Cargo.toml` (members 추가), `Cargo.lock`
- `docs/task-reports/II.2.2.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (II.2.2 `[ ]` → `[x]`)

## 구현 요약

- 의존 방향 `history -> commands -> document -> core-types`(path 의존). 역방향·순환 없음.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| `cargo tree -p stellar-history` | commands→document→core-types 선형 |
| `cargo check --workspace --locked` | OK |
| `cargo test` (3개) / fmt / clippy --locked | 통과 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 스켈레톤 crate 3개 생성으로 파일 수가 1.2.12의 5개 가이드를 넘으나, 계획서의 원자 작업 정의(3개 crate 묶음)를 따랐습니다.
