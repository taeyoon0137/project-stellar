# 작업 보고서: VII.4.1 HistoryEntry

- WBS: VII.4.1
- 작업명: `HistoryEntry`에 origin, forward/inverse transaction, grouping key, timestamp 대신 monotonic sequence 정의
- 상태: 완료 (`[x]`)
- 선행: VII.1.6, II.3.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/history/Cargo.toml` — `stellar-core-types` path 의존성 추가(2e06c27이 허용한 `history -> core-types` edge).
- `crates/history/src/lib.rs` — crate 문서, 모듈 등록.
- `crates/history/src/entry.rs`(신규) — `HistorySequence`, `SequenceAllocator`, `GroupingKey`, `TextInputKind`, `CommandSignature`,
  `HistoryPayload`, `HistoryEntry`, `HistoryError`.
- `crates/history/src/history.rs`(신규) — `History`(확정 항목 stack, `record_document`), 테스트 fixture, 테스트 4건.
- `crates/commands/src/lib.rs` — `pub use stellar_document as document;` 재수출(commands 공개 API가 노출하는 문서 타입을 history가
  직접 edge 없이 이름 붙이도록).
- `Cargo.lock` — history 의존성 추가로 cargo가 자동 갱신(`stellar-history` dependency 목록 1줄).
- `docs/task-reports/VII.4.1.md`(신규), 계획서(VII.4.1 `[ ]`→`[x]`).

## 구현 요약

- **`HistoryEntry`**: `sequence`(창 안 단조 `HistorySequence`, wall clock 미사용), `origin`(`CommandOrigin`, agent provenance 보존),
  `grouping`(`Option<GroupingKey>`), `payload`(문서: 정방향·역방향 `CommandTransaction`, 선택: 정방향·역방향 `SetSelection`).
- **`GroupingKey`**: pointer gesture, 텍스트 입력(node·입력 종류·편집 전후 caret), 키보드 반복(`CommandSignature`), scrub, 영역
  선택. 병합 규칙은 VII.4.2~VII.4.5에서 구현합니다.
- **`SequenceAllocator`**: 1부터 단조 증가, 한계에서 오류(되돌아가지 않음). 실패한 기록은 sequence를 소비하지 않습니다.
- **`History::record_document`**: transaction을 실행하고 성공하면 정방향·역방향을 한 항목으로 기록합니다. 실패하면 문서·history 모두
  실행 전 상태입니다.
- **세션 메모리 전용**: 항목에는 저장 형식이 없고 project·storage·document crate는 history에 의존하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-history` | 완료 |
| `cargo test -p stellar-history` | unit 4 passed, 0 failed |
| `cargo clippy -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-history -- --check` | 통과 |
| `cargo test -p stellar-commands`, clippy, fmt --check(재수출 추가 확인) | unit 62 passed, 경고 0, 통과 |
| `cargo xtask validate-crate-graph` | exit 0(의존 방향 위반 없음) |

| 검사 | 결과 |
|---|---|
| 기록 5건의 sequence 1..5, origin·grouping·정방향/역방향 id·역 command 정확, agent provenance 보존(`entries_have_origin_transactions_grouping_and_monotonic_sequence`) | 통과 |
| 실패 transaction은 기록·sequence 소비 없음(`failed_transaction_records_nothing`) | 통과 |
| sequence 한계 오류(`sequence_allocator_refuses_overflow`) | 통과 |
| project·storage·document manifest에 history 의존 없음, history가 문서 저장 레코드에 흔적을 남기지 않음(`history_is_excluded_from_project_serializer`) | 통과 |

## 차이(작업 지시와 다른 점)

- history 테스트가 문서 모델을 만들려면 document 타입이 필요해, 새 의존 edge 대신 commands가 `stellar_document`를 재수출하게 했습니다
  (scene crate의 core-types 재수출과 같은 방식). `validate-crate-graph`는 통과합니다.

## 남은 위험 / 한계

- undo·redo, 병합 규칙, 창별 분리는 VII.4.2~VII.5.5에서 이어집니다.
