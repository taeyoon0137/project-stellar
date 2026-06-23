# 작업 보고서: I.2.4 제한 YAML loader

- WBS: I.2.4
- 작업명: yaml-rust2 event 기반 제한 loader (단일 document, mapping/sequence/scalar만)
- 상태: 완료 (`[x]`)
- 선행: I.2.3 (`[x]`)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/yaml/restricted_loader.rs` (신규)
- `tools/xtask/src/yaml/mod.rs` (신규)
- `tools/xtask/src/lib.rs` (`pub mod yaml` 추가)
- `docs/task-reports/I.2.4.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.2.4 `[ ]` → `[x]`)

## 구현 요약

- `MarkedEventReceiver`로 event 스트림을 받아 `YamlValue`(Scalar/Sequence/Mapping) 트리를 구성.
- 거부 구문과 고유 오류 코드(`LoaderError { kind, line, col, message }`):
  - anchor → `anchor`
  - custom tag → `custom-tag`
  - merge key(`<<`) → `merge-key`
  - 다중 document → `multiple-documents`
  - 빈 document → `empty-document`
  - parse 실패 → `parse`
  - alias → `alias` (핸들러 존재, 아래 동작 참고)
- TaskStatus 필드 해석은 하지 않음(트리 구성·구문 거부만).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| 허용(mapping+sequence+scalar) fixture | Ok, 트리 일치 |
| anchor / custom-tag / merge-key / multiple-documents / empty-document | 각각 고유 코드 + line/col |
| alias 사용 문서 | 거부됨(아래 참고) |
| `cargo fmt/clippy/test -p xtask` | fmt 0, clippy 0, test 15 passed |

## 작업 지시와 다른 점

- **alias 동작**: yaml-rust2는 정의된 anchor+alias를 anchor 이벤트로 먼저 거부하고(코드 `anchor`), 미정의 alias는 scanner parse 오류로 거부합니다(코드 `parse`). 따라서 alias 단독 이벤트를 격리한 `alias` 코드 fixture는 만들 수 없으나, alias 사용 문서는 어느 경로로든 항상 거부됩니다. `LoaderErrorKind::Alias` 핸들러는 방어적으로 유지했습니다.

## 남은 경고 수

- 0
