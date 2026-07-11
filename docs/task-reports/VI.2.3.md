# 작업 보고서: VI.2.3 parent-child 삽입 validator

- WBS: VI.2.3
- 작업명: 자기 자신·자손 부모 지정과 다른 View root로의 무단 이동 거부
- 상태: 완료 (`[x]`)
- 선행: VI.2.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/tree_ops.rs`(신규) — `validate_reparent`, `reparent`,
  `ReparentError`.
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.2.3.md`(신규), 계획서(VI.2.3 `[ ]`→`[x]`).

## 구현 요약

- **검증 규칙**: ① stale 핸들 거부 ② 자기 자신 부모 지정 거부(`SelfParent`)
  ③ 자기 자손을 부모로 지정 거부(`DescendantParent`, parent chain 상향 탐색으로
  cycle 차단) ④ 다른 View tree(다른 root)로의 이동 거부(`CrossViewMove` —
  View 간 이동은 별도 명시 command의 몫) ⑤ View root 자체의 reparent 거부
  (`ViewRootMove`, VI.1.2의 단일 root 보장).
- **`reparent`**: 검증 통과 후에만 `set_parent`를 적용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 40 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| cycle fixture 실패(자손·간접 자손)(`descendant_parent_is_rejected`) | 통과 |
| cross-view fixture 실패(`cross_view_move_is_rejected`) | 통과 |
| self-parent·root 이동·stale 거부, 유효 이동 적용 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- OrderKey 정렬 자식 조회(VI.2.4)와 ReferenceIndex(VI.2.5)가 후속입니다.
