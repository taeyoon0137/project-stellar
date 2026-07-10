# 작업 보고서: V.6.6 지연 로딩 loader

- WBS: V.6.6
- 작업명: 열기에서 DocumentManifest까지만 읽고 ViewTree subtree는 요청 시 읽는 lazy loader
- 상태: 완료 (`[x]`)
- 선행: V.4.4, V.6.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/generation/lazy.rs`(신규) — `LazyGeneration`,
  `ChunkReadRecorder`, `LazyLoadError`.
- `crates/storage/src/generation/mod.rs` — `lazy` 모듈 등록.
- `docs/task-reports/V.6.6.md`(신규), 계획서(V.6.6 `[ ]`→`[x]`).

## 구현 요약

- **`LazyGeneration::open`**: 구조 검증(V.4.4, chunk는 존재 확인만) 후
  **DocumentManifest kind chunk의 payload만** 전체 검증으로 읽습니다. ViewTree
  subtree 등 다른 kind는 열기에서 읽지 않습니다.
- **`read_chunk_payload(chunk_id)`**: 요청 시 pointer를 찾아 전체 검증(V.2.6)으로
  읽습니다. manifest에 없는 id는 거부합니다.
- **계측**: 모든 payload 읽기가 단일 경로를 지나며 `ChunkReadRecorder`가 파일
  읽기 직전에 호출됩니다. 테스트 recorder가 "어떤 chunk 파일이 읽혔는가"를
  집합으로 검증합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 158 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 열기에서 DocumentManifest만 읽힘 — view chunk 3개 미읽음(`open_reads_only_document_manifest`) | 통과 |
| 요청한 view만 추가로 읽힘(`view_chunk_is_read_only_on_request`) | 통과 |
| 손상된 비활성 view가 열기를 막지 않음(열기에서 안 읽는 간접 증거) | 통과 |

## 차이(작업 지시와 다른 점)

- "read syscall 미발생" 계측은 OS syscall 추적 대신 chunk payload 읽기의 단일
  경로에 recorder를 끼우는 방식으로 구현했습니다. 저장 계층의 chunk 읽기가 전부
  이 경로를 지나므로 동일한 보증이며, 손상된 비활성 chunk가 열기를 막지 않는
  테스트가 이를 교차 검증합니다.

## 남은 위험 / 한계

- ViewTree subtree 목록(V.6.2)과 결합한 상위 문서 로딩 흐름은 문서 도메인
  (Phase VI)에서 연결합니다.
