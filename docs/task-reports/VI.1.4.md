# 작업 보고서: VI.1.4 OpenDocumentSet

- WBS: VI.1.4
- 작업명: 열려 있는 문서와 저장된 문서 index를 분리하는 `OpenDocumentSet` 런타임 타입
- 상태: 완료 (`[x]`)
- 선행: VI.1.1, III.6.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/open_set.rs`(신규) — `OpenDocumentSet`,
  `OpenDocumentHandle`, `OpenOutcome`.
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.1.4.md`(신규), 계획서(VI.1.4 `[ ]`→`[x]`).

## 구현 요약

- **분리**: 저장된 `ProjectDocumentIndex`(VI.1.1)와 별개로, 열려 있는 문서는
  런타임 `Document`(III.6.3) 인스턴스를 담는 `OpenDocumentSet`이 관리합니다.
  열기·닫기는 저장 index를 변경하지 않습니다(테스트에서 bytes 불변 확인).
- **중복 open**: 이미 열린 문서 id로 open하면 `OpenOutcome::Existing`으로
  **기존 handle**을 반환하고 런타임 문서를 중복 생성하지 않습니다.
- **handle**: slot+generation 불투명 handle. 닫으면 generation이 올라 옛
  handle이 무효가 되고(조회 None, 이중 close 불가), 재open은 새 handle입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 24 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 중복 open 시 기존 handle 반환(`duplicate_open_returns_existing_handle`) | 통과 |
| 저장 index 불변, close 후 handle 무효·재open, 다중 문서 격리 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 최대 열린 문서 수 제한(VI.1.5)이 후속입니다. 열린 문서에 저장된 내용을
  로드하는 연결은 VI.10 직렬화와 함께 구현됩니다.
