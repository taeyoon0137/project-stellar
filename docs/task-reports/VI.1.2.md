# 작업 보고서: VI.1.2 소유 관계와 View 단일 root Frame

- WBS: VI.1.2
- 작업명: Document→Page→View 소유 관계 명시, View당 정확히 하나의 root Frame
- 상태: 완료 (`[x]`)
- 선행: VI.1.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/definitions.rs` — `ViewDefinition.root_frame` 필드,
  `ViewDefinition::with_roots`, `ProjectDocumentIndex::validate_ownership`,
  오류 3종(`MissingRootFrame`/`MultipleRootFrames`/`SharedRootFrame`), binary
  인코딩에 root frame id 포함.
- `docs/task-reports/VI.1.2.md`(신규), 계획서(VI.1.2 `[ ]`→`[x]`).

## 구현 요약

- **정확히 하나의 root Frame**: `ViewDefinition.root_frame`이 단일 `StableId`라
  타입 수준에서 root 없음/복수를 표현할 수 없습니다. 외부 입력(파일·병합)의
  root 목록 경로는 `with_roots`가 0개(`MissingRootFrame`)·2개 이상
  (`MultipleRootFrames`)을 거부합니다.
- **소유 관계 검증**: `validate_ownership`이 문서·페이지·View id의 전역 유일성
  (= 각 항목이 정확히 하나의 부모에 속함)과 View 간 root Frame 공유 금지를
  검사합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 18 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| root 없음·root 두 개 fixture 거부(`view_requires_exactly_one_root_frame`) | 통과 |
| root 공유·전역 id 중복 거부(`ownership_validation_rejects_shared_roots_and_duplicate_ids`) | 통과 |
| root frame 포함 binary round-trip(기존 round-trip 테스트 갱신) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- root_frame이 실제 Frame kind 노드인지의 검사는 node tree(VI.2)와 결합하는
  전체 document validator(VI.10.1)에서 수행합니다.
