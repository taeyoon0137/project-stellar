# 작업 보고서: XXV.1.2 history journal 기록과 Git 추적 금지 검사

- WBS: XXV.1.2
- 작업명: completed transaction과 inverse를 project-local ignored path 또는 OS app data 정책에 따라 기록, Git 추적 금지 검사 추가
- 상태: 완료 (`[x]`)
- 선행: XXV.1.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증, 시스템 Git fixture).

## 변경 파일 목록

- `crates/history/src/journal_store.rs`(신규) — `JournalStore`, `JournalLocation`, `JournalProjectKey`, `JournalStoreError`, 원자적 기록, 테스트 3건.
- `crates/history/src/journal.rs` — header 고정 길이 `JOURNAL_HEADER_LEN`과 header만 읽는 `JournalHeader::decode_prefix`(덮어쓰기 판정용, 기존
  디코딩도 같은 함수 사용).
- `crates/history/src/test_fs.rs`(신규, `#![cfg(test)]`) — 임시 디렉터리, hermetic 시스템 Git 실행기, 파일 목록.
- `crates/history/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XXV.1.2.md`(신규), 계획서(XXV.1.2 `[ ]`→`[x]`).

## 구현 요약

- **위치 정책**(`JournalLocation`):
  - `AppData`(권장, 복구 스냅샷 §19과 같은 정책): `<app-data>/history-journal/<project-key>/<window>.stlhjnl`. `JournalProjectKey`는 소문자 영숫자·`-`
    1~128자만 허용해 경로에 프로젝트 이름·경로 텍스트가 들어가지 않습니다(V.9.1 규칙). 값은 project crate가 ProjectIdentity hash로 만드는 것을
    전제로 합니다(history는 storage hash에 의존할 수 없음).
  - `ProjectLocal`: `<project>/.stellar/cache/history/<window>.stlhjnl`. 디렉터리 안에 `*` guard `.gitignore`를 만들어 프로젝트 `.gitignore`가
    `.stellar`를 다시 포함시켜도(하위 `.gitignore` 우선) guard 자신을 포함한 모든 파일이 Git에서 제외됩니다.
- **Git 추적 금지 검사**(`check_untracked`, 열 때와 기록할 때마다): 앱 데이터 디렉터리는 symlink 해석 후 프로젝트 밖이어야 하고 프로젝트를 품어도
  안 됩니다. 프로젝트 내부 디렉터리는 symlink로 프로젝트 밖을 가리키면 거부하고, guard 내용이 정확해야 합니다(바뀌거나 사라지면 기록 거부).
  위치 검사는 디렉터리를 만들기 전에 합니다.
- **기록**: `record_save_point(window_history, base)`가 창 history의 완료된 항목 전체(undo·redo, 정방향·역방향 transaction, 선택 항목, 다음
  sequence)를 XXV.1.1 journal로 캡처해 같은 디렉터리 임시 파일(`.`로 시작) → `sync_all` → rename으로 교체합니다. preview 중(완료되지 않은 변경)은
  거부합니다. 기록 시점 계약은 "문서를 base 세대로 저장한 직후"입니다(undo stack 맨 위 = base 내용, XXV.1.1).
- **기존 journal 보호**(V.8.1): 기존 파일 header를 읽어 이 writer가 편집 가능일 때만 덮어씁니다. 최소 writer version이 더 높으면 `WriterTooOld`,
  다른 형식·읽을 수 없는 선언이면 `NotWritable`로 거부하고 파일을 보존합니다. magic·길이 위반 잔여물은 덮어씁니다.
- `read`(없으면 `None`, 파일 이름과 내용의 창 id가 다르면 `WindowMismatch`), `remove`(없어도 성공).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands -p stellar-history` | 완료 |
| `cargo test -p stellar-commands -p stellar-history` | commands unit 82 passed·1 ignored·doctest 3 passed, history unit 32 passed, 0 failed |
| `cargo clippy -p stellar-commands -p stellar-history --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -p stellar-history -- --check` | 통과 |
| `cargo test -p stellar-document workspace_production_code_does_not_use_test_support`(VI.10.4 검사) | 통과 |

| 검사 | 결과 |
|---|---|
| 실제 Git 저장소(사용자 `.gitignore`에 `!.stellar/`·`!.stellar/**` 재포함 규칙)에서 앱 데이터·프로젝트 내부 정책으로 각각 기록·재기록·읽기: journal = capture 결과(undo 4·redo 2), 임시 파일 잔여 0, 앱 데이터 경로에 프로젝트 경로 텍스트 없음, 프로젝트 내부 guard 내용 정확·`git check-ignore` 제외, `git status --porcelain=v1 --untracked-files=all` 기록 전과 동일. 앱 데이터만 쓴 별도 저장소는 파일 목록과 `--ignored` 포함 status 모두 불변(빈 status)(`recording_journals_leaves_git_status_unchanged`) | 통과 |
| 앱 데이터가 프로젝트 안·프로젝트를 품음·상대/`..` 경로·잘못된 project key 거부, guard 변경·삭제 뒤 기록 거부와 파일 미생성, `.stellar` symlink가 밖을 가리키면 거부하고 밖에 아무것도 만들지 않음(`git_tracking_guards_reject_unsafe_locations`) | 통과 |
| preview 중 기록 거부, 미래 writer(최소 writer 2) journal `WriterTooOld`로 보존, 다른 형식 `NotWritable`, 깨진 잔여물 덮어씀, 창 id 불일치 읽기 거부(`preview_and_newer_writer_journals_are_not_overwritten`) | 통과 |
| 검사 민감도 확인(커밋하지 않은 임시 변경): guard에서 `*` 줄을 빼면 첫 test가 실패함 | 확인 |

## 차이(작업 지시와 다른 점)

- 두 정책을 모두 구현하고 선택은 caller에 맡겼습니다(작업 지시가 "또는"). 앱 데이터를 권장으로 문서화했습니다.
- "Git 추적 금지 검사"는 경로·guard 검사입니다. 그 경로에 이미 Git이 추적 중인 파일이 있는지는 Git index를 읽어야 해 git-porcelain을 쓰는 상위
  계층(project)의 몫으로 남겼습니다(crate graph상 history는 git-porcelain에 의존할 수 없음). 실제 Git 동작은 test에서 시스템 Git으로 확인했습니다.
- 시스템 Git이 없는 환경에서는 test가 Git status 비교만 건너뜁니다(이번 실행은 Git 있음).

## 남은 위험 / 한계

- 기록은 저장 시점마다 history 전체를 다시 씁니다(O(history 크기)). 크기 상한·compaction은 XXV.1.5입니다.
- 기록 시점 계약(문서 = base 세대)은 caller가 지켜야 합니다. history는 저장 여부를 알 수 없습니다.
- 앱 데이터 정책에서 같은 project key를 다른 프로젝트가 쓰면 디렉터리를 공유합니다. key 유일성은 project crate 계약입니다.
