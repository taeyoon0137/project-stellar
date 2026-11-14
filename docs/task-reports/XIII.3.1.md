# 작업 보고서: XIII.3.1 clean 창의 외부 generation 자동 reload

- WBS: XIII.3.1
- 작업명: 창에 로컬 미저장 변경이 없으면 외부의 새 정상 generation을 자동 reload하고 history를 폐기
- 상태: 완료 (`[x]`)
- 선행: XIII.2.5(`[x]`), VII.5.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/window_session.rs`(신규): `WindowDocumentSession`, `ExternalReloadOutcome`, `SessionSave`,
  `WindowSessionError`, 테스트.
- `crates/project/src/external_change.rs`: `WindowExternalState::adopt_reloaded`(clean 상태에서만 base 교체),
  `SnapshotError::Serialize`, 모듈 문서의 XIII.3.1 연결 설명.
- `crates/project/src/autosave/dirty.rs`: 테스트 fixture(`document`·`sid`·`tx`·`select`)를 crate 테스트에서
  함께 쓰도록 `pub(crate)`로 열고 문서 id를 offset으로 만들 수 있게 일반화(테스트 전용).
- `crates/project/src/lib.rs`: `pub mod window_session;`.
- `docs/task-reports/XIII.3.1.md`(신규), 계획서(XIII.3.1 `[ ]`→`[x]`).

## 구현 요약

- **세션 소유**: `WindowDocumentSession`이 창 하나의 문서 상태(`DocumentState`), dirty 판정
  (`DirtyTrackedHistory`, XIII.1.2), 디스크 동기 상태(`WindowExternalState`, XIII.3.2), 마지막 저장 상태
  (`SavedGenerationState`), 자기 저장 기록(`OwnSaveLedger`)을 함께 소유합니다. 문서·선택을 바꾸는 경로는 이
  type의 method뿐이고 편집 뒤 dirty를 디스크 동기 상태에 반영하므로 두 판정이 어긋나지 않습니다.
- **`on_external_generation`**(coalescer `ExternalChange::Generation` 입력):
  1. preview 진행 중이면 `Deferred`(진행 중 조작을 빼앗지 않음). 미룬 hash는 `take_deferred_external`.
  2. 로컬 미저장 변경이 있으면 XIII.3.2 대기 상태(`Pending`, 세 snapshot 보존). 메모리 snapshot은 이 창 문서만
     새로 직렬화하고 같은 generation의 다른 문서 chunk는 base pointer(`Clean`)로 둡니다.
  3. 로컬 변경이 없으면 프로젝트를 다시 열고(`SavedProjectLoader`, 저장소 검사·V.4.4) 이 창 문서를 읽은 뒤(chunk
     내용 검증) **모두 성공했을 때만** 문서를 교체하고 history(undo·redo)와 선택을 폐기합니다(`Reloaded`).
     열기·읽기 실패는 아무 상태도 바꾸지 않습니다. 결정한 generation과 실제 current가 다르면 `Superseded`.
  4. 갱신한 generation을 base·마지막 저장 상태·자기 저장 기록의 최신 값으로 삼습니다. 문서 revision은 이어서
     증가합니다(재사용 없음).
- **자기 저장 기록 계약 연결**(리뷰 minor `ledger-latest-contract-unwired`): reload가 `OwnSaveLedger::record`를
  호출하므로 pointer가 예전 자기 저장 generation으로 되돌아가면(Git checkout 등) 자기 저장으로 오판하지 않고
  외부 변경으로 다시 갱신합니다.
- **동기 저장 경로**: `save_blocking`은 이 창 문서 chunk만 새로 쓰고 다른 문서 chunk는 유지하는 저장 계획으로
  `run_guarded_save`(대기 중 거부)를 실행하고 기록을 갱신합니다. worker 분리·stale 결과 폐기는 XIII.1.4
  범위입니다(이 행의 fixture에 자기 저장이 필요해 둔 경로).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 354 passed, 1 ignored(수동 system font scan); 통합 테스트 전부 통과) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **clean window reload fixture**: 실제 저장 파이프라인으로 저장한 프로젝트, 선택 3회(clean) → 외부 writer 저장 → coalescer `Generation` → `Reloaded{2, 폐기 3}`, 문서 = 외부 문서, history·redo·선택 비어 있음, base·저장 상태·ledger = g2, revision 증가, 같은 hash 재입력 `Unchanged`, reload 뒤 undo는 reload 이전으로 가지 않음(`clean_window_reloads_the_new_generation_and_discards_history`) | 통과 |
| 로컬 변경 창은 reload하지 않고 `Pending`(세 hash), 메모리·history 유지, 저장 거부(`window_with_local_changes_is_not_reloaded`) | 통과 |
| preview 중 `Deferred`, 취소 뒤 미룬 hash로 `Reloaded`(`reload_waits_for_an_in_progress_preview`) | 통과 |
| 결정한 g2보다 g3가 current면 `Superseded`(상태 불변), g3로 `Reloaded`(`superseded_generation_is_not_applied`) | 통과 |
| 새 generation chunk 내용 손상 → 오류, 문서·base·저장 상태·ledger 불변(`invalid_external_generation_is_not_partially_applied`) | 통과 |
| 자기 저장 B → 외부 X reload → pointer가 B로 되돌아감 → coalescer가 외부 변경으로 보고 B로 `Reloaded`(`pointer_reverted_to_an_earlier_own_save_reloads_after_an_external_reload`). reload의 `ledger.record`를 주석 처리하면 이 테스트가 실패함을 확인(mutation 확인 후 원복) | 통과 |
| 두 문서 프로젝트에서 창 저장이 다른 문서 chunk를 유지, 다른 문서만 바뀐 외부 generation도 reload(`other_documents_keep_their_chunks_through_session_saves`) | 통과 |

## 차이(작업 지시와 다른 점)

- preview 진행 중 외부 generation은 바로 reload하지 않고 미룹니다(로컬 미완료 조작 보호). 작업 지시는 "로컬
  미저장 변경이 없으면"이며, 진행 중 preview는 메모리에 미완료 변경이 있는 상태로 해석했습니다.
- `save_blocking`(동기 저장)은 작업 지시 밖이지만 자기 저장 → reload → pointer 되돌림 fixture에 필요해 두었습니다.
- runtime 창 session을 이 type으로 바꾸고 셸에 reload 알림을 보내는 연결은 runtime crate 범위라 하지 않았습니다.

## 남은 위험 / 한계

- 미룬 외부 generation은 preview가 끝난 뒤 호출자가 `take_deferred_external`로 다시 넘겨야 합니다(자동 재시도
  없음).
- `stellar.yaml`의 project id가 바뀐 외부 변경은 `ProjectChanged` 오류로만 보고합니다(창 재열기 정책 미정).
- 창 문서가 새 generation에서 삭제된 경우 `Document(UnknownDocument)` 오류이며 창 처리 정책은 셸 범위입니다.
