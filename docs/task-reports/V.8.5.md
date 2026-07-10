# 작업 보고서: V.8.5 migration Git working tree 통합 fixture

- WBS: V.8.5
- 작업명: migration 결과를 자동 commit하지 않고 Git working tree 변경으로 남기는 통합 fixture
- 상태: 완료 (`[x]`)
- 선행: V.8.4, IV.3.4

## 필수 실행 환경

- `DEV-MAC` (Rust + 시스템 Git, 로컬 실행).

## 변경 파일 목록

- `crates/storage/tests/migration_git_fixture.rs`(신규) — 통합 fixture.
- `docs/task-reports/V.8.5.md`(신규), 계획서(V.8.5 `[ ]`→`[x]`).

## 구현 요약

- **fixture 흐름**: 실제 Git repo에 v1 프로젝트 구성 → baseline commit →
  noop v1→v2 migration 실행 → 검증.
- **검증**: ① HEAD 불변(자동 commit 없음) ② `git status --porcelain` 결과가
  정확히 기대 파일 2개 — `M design/current`(pointer 전환)와
  `?? design/generations/<새 manifest>`(추가). 그 외 어떤 변경도 없어야 통과.
- noop migration이므로 chunk 파일 추가·수정이 없다는 점까지 status 라인 수(2)로
  함께 고정됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage --test migration_git_fixture` | 1 passed, 0 failed |
| `cargo test -p stellar-storage` (전체) | 192 passed + 통합 1 + doctest 2, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

## 차이(작업 지시와 다른 점)

- Git 실행은 IV.3의 `GitRunner` 대신 테스트 전용 직접 실행을 사용했습니다 —
  storage crate는 crate 그래프 정책상 git-porcelain에 의존할 수 없고, fixture의
  Git 호출은 검증 스크립트 역할이기 때문입니다.

## 남은 위험 / 한계

- golden fixture runner(`cargo xtask test-migrations`, V.8.6)는 후속입니다.
