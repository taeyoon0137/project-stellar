# 작업 보고서: IV.4.2 filesystem transaction helper

- WBS: IV.4.2
- 작업명: 폴더 생성 후 실패 시 롤백하는 RAII 트랜잭션 헬퍼
- 상태: 완료 (`[x]`)
- 선행: IV.4.1(사전 검증)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/fs_transaction.rs` (신규: `DirTransaction`, fault injection 테스트)
- `crates/project/src/lib.rs` (`pub mod fs_transaction;`)
- `docs/task-reports/IV.4.2.md`(신규), 계획서(IV.4.2 `[ ]`→`[x]`)

## 구현 요약

- `DirTransaction::create(target)`는 `create_dir`로 새 폴더를 만듭니다(이미 존재하거나 부모가
  없으면 실패). 따라서 트랜잭션이 잡는 폴더는 항상 **이 호출이 새로 만든 것**입니다.
- `commit()` 없이 드롭되면 `Drop`이 **생성한 폴더와 그 안에 만든 내용을 모두 제거**합니다
  (롤백). 생성 이후 어느 단계에서 `?`로 일찍 반환해도 가드가 드롭되며 롤백됩니다.
- `commit()`하면 드롭 시 롤백하지 않고 폴더를 유지합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (전체) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| commit → 폴더·scaffold 유지 | 통과 |
| **실패(fault injection) → 롤백, 부모 entry 0(미오염)** | 통과 |
| commit 없는 스코프 종료 → 롤백 | 통과 |
| **이미 존재하는 target은 create 실패 + 기존 데이터 보존**(타인 데이터 미삭제) | 통과 |
| 부모 없음 → create 실패(부모 미생성) | 통과 |

## 차이(작업 지시와 다른 점)

- "빈 폴더 제거"를 넘어, 롤백 시 트랜잭션이 만든 폴더의 **부분 scaffold 내용까지** 제거해 부모
  오염을 완전히 막습니다(`remove_dir_all`로 우리가 만든 서브트리만 제거). `create_dir`로
  배타 생성하므로 타인의 기존 폴더는 절대 롤백 대상이 되지 않습니다.

## 남은 위험 / 한계

- scaffold 파일 생성(기본 stellar.yaml 등)은 IV.4.3, git init은 IV.4.4, 단계별 rollback 정책
  종합은 IV.4.7에서 다룹니다. 본 작업은 폴더 생성/롤백 가드에 한정합니다.
