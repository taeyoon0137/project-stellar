# 작업 보고서: V.6.1 초기 chunk kind 정의

- WBS: V.6.1
- 작업명: 초기 chunk kind(ProjectMetadata, DocumentManifest, ViewTree, ComponentLibrary, TokenSet, AssetMetadata) 정의
- 상태: 완료 (`[x]`)
- 선행: V.2.7

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/chunk/kinds.rs`(신규) — kind 상수 6종, `INITIAL_KINDS`,
  `initial_kind_registry`.
- `crates/storage/src/chunk/mod.rs` — `kinds` 모듈 등록.
- `docs/task-reports/V.6.1.md`(신규), 계획서(V.6.1 `[ ]`→`[x]`).

## 구현 요약

- **code 할당(golden 고정)**: ProjectMetadata `0x80000001`, DocumentManifest
  `0x80000002`, ViewTree `0x80000003`, ComponentLibrary `0x80000004`, TokenSet
  `0x80000005`(이상 required bit 설정), AssetMetadata `0x00000006`(optional).
- **requiredness 기준**: 모르면 문서 구조·의미 해석이 깨지는 kind는 required,
  없이도 안전하게 열 수 있는 부가 정보(AssetMetadata)는 optional.
- **`initial_kind_registry()`**: V.2.7 registry에 6종을 등록해 반환합니다. unknown
  kind version 정책(optional skip/required 오류)이 그대로 적용됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 148 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| kind registry golden test(숫자 code·이름 6종 고정) | 통과 |
| required bit 정책 일치, 초기 kind 전부 resolve, unknown 정책 유지 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- AssetMetadata의 optional 분류는 asset 파이프라인(Phase VIII) 구체화 시 재검토가
  필요할 수 있습니다. required로 승격하려면 새 code 발급이 필요합니다(bit가 code의
  일부).
