# 작업 보고서: XIII.5.4 asset write temp+fsync+atomic rename

- WBS: XIII.5.4
- 작업명: asset write도 temp+fsync+atomic rename 사용
- 상태: 완료 (`[x]`)
- 선행: XIII.5.2, V.2.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 파일 시스템 fault injection).

## 변경 파일 목록

- `crates/assets/src/atomic_write.rs`(신규) — `write_atomic_streamed`, `AtomicWriteError`,
  storage `WriteStage`·`WriteObserver`·`NoopObserver` 재노출.
- `crates/assets/src/store.rs` — copy pass를 `write_atomic_streamed`로 교체,
  `AssetStore::import_file_observed`, `ImportOperation::Write(WriteStage)`(기존 `Rename` 대체).
- `crates/assets/src/lib.rs` — `atomic_write` 모듈 등록.
- `docs/task-reports/XIII.5.4.md`(신규), 계획서(XIII.5.4 `[ ]`→`[x]`).

## 구현 요약

- **storage protocol 재사용**: storage V.2.5 `write_file_atomic`과 같은 6단계·순서
  (`CreateTemp` → `WriteBytes` → `Flush` → `SyncFile`(`sync_all`, macOS는 std가
  `F_FULLFSYNC`) → `Rename` → `SyncParentDir`(POSIX))와 같은 관찰자 타입(`WriteStage`·
  `WriteObserver`, storage에서 import해 재노출)을 씁니다. storage 함수는 `&[u8]` 전체를 받아
  대형 에셋을 메모리에 올려야 하므로 `WriteBytes` 단계만 caller body(64 KiB buffer streaming
  복사 + 재hash 검증)로 바꾼 `write_atomic_streamed`를 assets에 두었습니다.
- **불변식**: rename이 마지막 변경 단계이고 temp는 대상과 같은 디렉터리에 `create_new`로
  만듭니다. body 실패(원본 변경 `SourceChanged`, 읽기 실패)나 어느 단계 중단에서도 대상
  경로에 부분 파일이 생기지 않습니다. 우아한 실패는 temp를 지우고, 강제 종료로 남은 temp는
  이름이 `.`로 시작해 content 파일로 해석되지 않으므로 dedup·GC 대상이 아닙니다.
- **import 연결**: `import_file`은 `NoopObserver`로 `import_file_observed`를 호출합니다.
  단계 실패는 `ImportError::Io { operation: Write(stage) }`로 보고합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-assets` | lib 76 passed + integration 1 passed + doctest 2 passed(compile_fail), 0 failed |
| `cargo clippy -p stellar-assets --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-assets -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 단계 실행 순서 = storage `WriteStage::ALL`(`stages_run_in_storage_order`) | 통과 |
| 6단계 각각 중단 주입: rename 전 중단은 최종 파일 없음, `SyncParentDir` 중단은 완전한 내용, temp 정리(`interruption_before_rename_leaves_no_final_file`) | 통과 |
| 기존 대상이 있을 때 rename 전 중단 → 이전 내용 유지(`interruption_keeps_previous_target_content`) | 통과 |
| body 중간 실패(부분 기록 후 오류) → 최종 파일·temp 없음 | 통과 |
| 강제 종료 모사(panic, 정리 미실행) → 최종 이름 없음, `.`로 시작하는 temp만 남음 | 통과 |
| import 수준 6단계 중단 주입(3×64Ki+11 bytes): 최종 파일 없음 또는 완전, temp 없음, 재시도 성공(`Copied`/`Deduplicated`)(`interrupted_import_never_leaves_partial_final_file`) | 통과 |
| import 수준 Flush·SyncFile·Rename 직전 강제 종료: 최종 이름 없음, 남은 temp는 `locate_content`가 무시, 재import `Copied`(`forced_abort_after_copy_leaves_only_hidden_temp`) | 통과 |

## 차이(작업 지시와 다른 점)

- storage `write_file_atomic` 함수를 그대로 호출하지 않고 같은 단계·관찰자 타입을 쓰는
  streaming 버전을 assets에 두었습니다(전체 bytes를 받는 API로는 XIII.5.2의 "전체 메모리 로드
  없음"을 지킬 수 없음). temp 이름 규칙(`.<file>.tmp-<pid>-<n>`)과 부모 디렉터리 sync는
  storage 내부(private) 구현과 같은 규칙으로 중복 구현했습니다. storage가 streaming 쓰기를
  공개하면 그쪽으로 합칠 수 있습니다.

## 남은 위험 / 한계

- 강제 종료로 남은 temp 파일 정리는 XIII.5.6 GC에서 다룹니다(이 작업은 최종 경로 불변식만
  보장).
- 실제 전원 차단·커널 crash는 재현하지 않았고 단계 직전 중단 주입과 panic으로 모사했습니다
  (storage V.2.5와 같은 검증 수준).
- Windows의 부모 디렉터리 sync는 no-op입니다(Windows 지원은 현재 보류).

## 후속 수정(보안 리뷰 반영): 만들지 않은 temp 항목을 지우지 않음

- **문제**: `write_atomic_streamed`는 temp 생성(`create_new`)이 실패해도 실패 정리로 그 temp
  이름의 `remove_file`을 호출했습니다. 이름이 이미 있으면(다른 process의 temp, 미리 심어 둔 항목)
  이 호출이 만들지 않은 항목을 지울 수 있었습니다(출처 key 파일 리뷰 S2와 같은 유형).
- **수정**: temp 생성은 `create_new_temp`로 새 이름(같은 `.<file>.tmp-<pid>-<n>` 형식, GC 해석과
  호환)을 최대 16회 시도하고, 이미 있는 이름은 열거나 지우지 않습니다. 정리는 이 호출이 만든 temp에만,
  rename 전 실패에서만 합니다(rename 뒤 `SyncParentDir` 실패에서는 지우지 않음).
- **검증**: `existing_temp_entries_are_skipped_and_never_removed`(점유 파일·디렉터리·symlink 건너뛰기,
  모두 점유 시 `AlreadyExists`, symlink 대상 불변, body 실패 정리 후에도 점유 항목 보존). 기존 단계
  순서·중단 주입·강제 종료 테스트는 그대로 통과합니다.
