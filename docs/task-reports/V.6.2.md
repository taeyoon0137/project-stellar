# 작업 보고서: V.6.2 ViewTree subtree 참조 스키마

- WBS: V.6.2
- 작업명: ViewTree가 subtree root 목록과 개별 subtree chunk 참조를 담는 스키마 정의
- 상태: 완료 (`[x]`)
- 선행: V.6.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/chunk/view_tree.rs`(신규) — `ViewTreeChunk`, `SubtreeRef`,
  `ViewTreeError`.
- `crates/storage/src/chunk/mod.rs` — `view_tree` 모듈 등록.
- `docs/task-reports/V.6.2.md`(신규), 계획서(V.6.2 `[ ]`→`[x]`).

## 구현 요약

- **payload 스키마**: view root node StableId(16바이트) + subtree count(`u32` LE) +
  각 subtree 항목(root node StableId 16바이트 + subtree `ChunkId` 16바이트).
  schema version은 payload가 아니라 chunk pointer(V.3.1)가 보관합니다.
- **의미**: ViewTree chunk는 노드 내용을 직접 담지 않고 subtree chunk를 ChunkId로
  참조합니다. 지연 로딩(V.6.6)이 이 목록에서 필요한 subtree만 열게 됩니다.
- **불변식**: subtree root node 중복 거부. subtree 순서는 자식 순서로서 보존됩니다.
  trailing bytes·절단은 canonical decoder가 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 154 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 2개 subtree fixture round-trip + 재인코딩 byte-identical(`two_subtree_fixture_round_trips`) | 통과 |
| 순서 보존, 빈 목록, 중복 root 거부, 절단·trailing 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 노드 내용(subtree chunk payload) 스키마는 문서 도메인 모델(Phase VI)에서
  정의됩니다. 이 작업은 참조 구조만 고정합니다.
