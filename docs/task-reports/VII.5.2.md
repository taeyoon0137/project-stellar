# 작업 보고서: VII.5.2 창의 inverse transaction을 현재 창 문서에 적용

- WBS: VII.5.2
- 작업명: Undo는 해당 창의 inverse transaction을 현재 창의 in-memory document state에 적용(다른 창과 모델 비공유 정책 반영)
- 상태: 완료 (`[x]`)
- 선행: VII.5.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/history/src/window.rs` — `WindowHistory::undo` 정책 문서화, 창별 독립 snapshot 테스트.
- `docs/task-reports/VII.5.2.md`(신규), 계획서(VII.5.2 `[ ]`→`[x]`).

## 구현 요약

- `WindowHistory::undo(doc)`는 이 창 stack의 마지막 항목 역 transaction을 **이 창의 인메모리 문서**(`DocumentState`)에만 적용합니다. 창끼리
  모델을 공유하지 않으므로(§22.2) 다른 창의 문서·revision과 무관하며, 역 transaction은 이 창 문서의 현재 revision으로 rebase해 원자
  실행합니다(VII.4.6 경로).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-history` | 완료 |
| `cargo test -p stellar-history` | unit 18 passed, 0 failed |
| `cargo clippy -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-history -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 같은 초기 문서의 두 창: A(크기 변경 → subtree 삭제), B(이동). 창별 revision 분기, A undo 2회가 A snapshot만 단계별 원복(B 불변), B undo가 B만 원복(A 불변), 빈 stack undo `None`(`undo_applies_to_each_windows_own_document`) | 통과 |

## 차이(작업 지시와 다른 점)

- 동작은 VII.4.6·VII.5.1에서 만든 경로를 그대로 쓰며, 이번 작업은 창별 독립 적용 정책을 문서화하고 fixture로 고정했습니다.

## 남은 위험 / 한계

- 없음.
