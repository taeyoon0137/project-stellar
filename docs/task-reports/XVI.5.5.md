# 작업 보고서: XVI.5.5 비활성 문서 chunk unload와 재로드

- WBS: XVI.5.5
- 작업명: 비활성 document chunk unload·재로드, document index와 미저장 dirty chunk는 unload 금지
- 상태: 완료 (`[x]`)
- 선행: V.6.6, VI.1.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증. 실제 저장 파이프라인과 temp dir 사용).

## 변경 파일 목록

- `crates/project/src/document_io/residency.rs`(신규) — `ResidentDocuments`, `DirtyState`,
  `UnloadReport`, `ResidencyError`.
- `crates/project/src/document_io/mod.rs` — `residency` 모듈 등록.
- `crates/project/tests/document_residency.rs`(신규).
- `docs/task-reports/XVI.5.5.md`(신규), 계획서(XVI.5.5 `[ ]`→`[x]`).

## 구현 요약

- **index 분리**: `ResidentDocuments::open`은 generation 열기 때 이미 읽힌 DocumentManifest(V.6.6)
  만으로 문서 index를 만들고 node chunk는 읽지 않습니다. index는 unload 대상이 아니며
  `document_index()`는 chunk 읽기 없이 동작합니다.
- **로드/재로드**: `ensure_loaded`가 메모리에 없는 문서만 `load_document_from_generation`(VI.10.3)
  으로 그 문서의 ViewTree·subtree chunk를 storage lazy loader(`LazyGeneration::read_chunk_payload`)
  로 읽어 모델을 재구축합니다. 이미 올라온 문서는 다시 읽지 않습니다.
- **unload 정책**(`unload_inactive`): `OpenDocumentSet`(VI.1.4)에 열린 문서는 유지, 나머지는
  dirty 판정이 `Clean`일 때만 내립니다.
- **dirty 판정**(`dirty_state`): 현재 모델을 직렬화한 각 chunk의 content hash를 기준 generation의
  pointer hash와 비교합니다(flag가 아닌 내용 비교라 편집 경로가 표시를 빠뜨려도 미저장 내용을
  잃지 않음). 다른 chunk 목록을 `Dirty`로 보고하고, 직렬화가 실패하는 문서(편집 중 불변식
  위반)는 `Unserializable`로 보수적으로 유지합니다. 저장 후 `adopt_generation`으로 새 generation을
  기준으로 삼으면 clean이 됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project` | unit 138, `document_residency` 3, `document_round_trip` 4, `document_serialize` 4 passed, 0 failed |
| `cargo test -p stellar-document` | 189 passed + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-project -- --check`, `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | 통과(exit 0) |

| 검사 | 결과 |
|---|---|
| 문서 3개 저장(실제 V.5 파이프라인) → 로드 → 비활성 2개 unload, 활성 1개 유지 → index 3개 유지·조회 중 읽기 0 → 재로드 시 그 문서의 ViewTree+subtree chunk만 1회씩 읽고 semantic snapshot(VI.10.4) 동일(`inactive_document_reload_has_identical_semantic_snapshot`) | 통과 |
| 비활성 문서 미저장 편집 → 정확히 그 subtree chunk가 dirty, unload 대상에서 제외·편집 보존; 직렬화 불가 상태도 유지; 저장 후 새 generation 채택 → clean → unload → 재로드 snapshot = 편집본(`unsaved_dirty_document_is_never_unloaded`) | 통과 |
| index에 없는 문서 요청 거부(`unknown_document_is_rejected`) | 통과 |

## 차이(작업 지시와 다른 점)

- dirty 단위는 chunk로 판정하되 unload 단위는 문서입니다: 문서의 chunk 중 하나라도 미저장이면
  그 문서 전체를 메모리에 둡니다(미저장 chunk를 절대 내리지 않는 보수적 해석). View 단위
  부분 residency는 모델 분할이 필요해 범위 밖입니다.

## 남은 위험 / 한계

- 메모리 예산 provider·eviction 우선순위(XVI.5.1~XVI.5.4)와 아직 연결하지 않았습니다. 이 작업은
  호출 시 unload·재로드 동작과 보호 규칙을 고정합니다.
- dirty 판정은 문서를 직렬화하므로 문서 크기에 비례하는 비용이 듭니다. 큰 문서의 증분 dirty
  추적은 command 계층(Phase VII)의 변경 집합과 연결해 최적화할 수 있습니다.
