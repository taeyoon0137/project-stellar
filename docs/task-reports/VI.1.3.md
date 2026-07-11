# 작업 보고서: VI.1.3 표시 이름 API 분리

- WBS: VI.1.3
- 작업명: 표시 이름을 ID·저장 경로에 사용하지 않도록 API 분리
- 상태: 완료 (`[x]`)
- 선행: VI.1.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/definitions.rs` — `display_name` 필드를 비공개로 전환,
  생성자(`new`)·접근자(`display_name()`)·`rename()` API 추가.
- `docs/task-reports/VI.1.3.md`(신규), 계획서(VI.1.3 `[ ]`→`[x]`).

## 구현 요약

- **API 분리**: 표시 이름은 비공개 필드가 되어 읽기(`display_name()`)와 변경
  (`rename()`)으로만 접근합니다. 식별(`id`)·참조(`root_frame`)는 별도 공개
  필드로, 이름이 식별·경로 유도에 끼어들 수 있는 경로가 타입 수준에서
  차단됩니다.
- **경로 불변 근거**: chunk 저장 경로는 content hash(V.2.3)와 ChunkId로만
  유도되고 이름은 입력에 포함되지 않습니다. rename 후 id·root frame·id 기반
  조회가 전부 불변임을 테스트로 고정했습니다. 빈 이름도 허용됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 20 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| rename 후 id·참조·조회 불변 + 빈 이름 허용(`rename_does_not_change_ids_or_references`) | 통과 |
| 생성자 계층 구성(`constructors_build_hierarchy`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- `OpenDocumentSet`(VI.1.4)과 열린 문서 수 제한(VI.1.5)이 후속입니다.
