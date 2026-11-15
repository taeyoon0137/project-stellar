# 작업 보고서: XII.6.4 Undo/Redo/Save Now를 활성 창 session에 연결

> 아래 초기 구현·검증 기록은 선별 통합 원본 범위 `3892c5e3..2636e8ce`에서 가져왔습니다.
> 현재 기준 `eac4531b` 위의 재검증·수정·남은 범위는 [세션 통합 보고서](../status/session-takeover-integration.md)를 따릅니다.

## WBS

- WBS: XII.6.4
- 작업명: Undo/Redo/Save Now command를 active WindowSession에 연결합니다.
- 상태: 완료 (`[x]`) — 창별 명령 fixture 통과. 근거는 아래 "2026-11-15 재검증" 절입니다.
- 선행: VII.5.3 (`[x]`), V.5.4 (`[x]`)

## 변경 파일

- `crates/runtime/src/window_session.rs` (수정) — `undo`·`redo`(이 창 history만 재생), `save_now`(작업 문서 직렬화 → loaded
  generation 위 SavePlan, 다른 문서 chunk는 Clean으로 이어 쓰기, 쓰기 전 optimistic check, 바뀐 것이 없으면 쓰지 않음, 성공 시
  loaded generation 갱신), `SaveOutcome`·`WindowSessionSaveError`, 테스트 3개와 fixture(`saved_project_with`, `other_document`).
- `crates/runtime/src/app_command.rs` (신규) — `AppCommand`(Undo 1·Redo 2·SaveNow 3).
- `crates/runtime/src/app.rs` (수정) — `perform`·`undo`·`redo`·`save_now`·`set_selection`, event `HistoryReplayed`(0x0109)·
  `WindowSessionSaved`(0x010A), 실패 코드 `SaveStale`(21)·`SaveFailed`(22)·`HistoryUnavailable`(23).
- `crates/runtime/src/app_window_tests.rs`, `crates/runtime/src/lib.rs` (수정) — runtime 경계 명령 테스트, 모듈 연결.
- `crates/ffi/src/window_session.rs` (수정) — `stellar_window_session_perform`·`stellar_window_session_select`, C ABI 테스트.
- `crates/ffi/src/lib.rs`, `tools/xtask/src/ffi_header.rs`, `crates/ffi/include/stellar_ffi.h` (수정·재생성) — `STELLAR_COMMAND_*`,
  새 event·실패 코드·함수 노출과 헤더 일치 테스트.
- `docs/adr/ADR-0028-window-session-shell-surface.md` (수정) — XII.6.4 절.
- `apps/macos/Stellar/CoreRuntime.swift`, `UI/Shell/ShellRuntimeEventPump.swift` (수정) — 명령·선택 호출, event 해석.
- `apps/macos/Stellar/UI/Shell/ShellProjectWindow.swift` (수정) — `ShellWindowSessionCommand`(코어 명령 코드),
  `performSessionCommand`(이 창 session으로), 명령 활성 상태(`setSessionCommands`), 저장 실패 alert, `selectNodes`.
- `apps/macos/Stellar/UI/Shell/ShellAppCoordinator.swift` (수정) — 창 도구 모델 명령을 창 session으로, alert 경로 연결.
- `apps/macos/Stellar/UI/Shell/ShellIntegrationCheck.swift` (수정) — 비활성 명령 설명 갱신(문서·history 없는 창).
- `apps/macos/Stellar/UI/Shell/ShellSessionCommandsCheck.swift` (신규), `apps/macos/Stellar/main.swift`,
  `scripts/macos/smoke.sh` (수정) — `--session-commands-check <fixture>`.
- `docs/design/ui/tools_menus.md`, `docs/design/ui/shell_frame.md` (수정) — 남은 일·Stub 설명 갱신.
- `docs/task-reports/XII.6.4.md`(신규), 계획서(XII.6.4 `[ ]`→`[~]`)

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `scripts/macos/smoke.sh` | 0 (`BUILD SUCCEEDED`, Swift warning 0, `session-commands ok: 31 checks` 포함 `smoke OK`) |
| `cargo test -p stellar-runtime -p stellar-ffi` | 0 (runtime 58 + 8 + 2 + 11, ffi 61) |
| `cargo clippy -p stellar-runtime -p stellar-ffi -p xtask --all-targets -- -D warnings` | 0 |
| `cargo fmt --all -- --check` | 0 |
| `cargo run -q -p xtask -- generate-ffi-header` 후 헤더 일치 테스트 | 통과 |
| 변이: 셸 Undo 명령 코드를 REDO로 바꾼 빌드의 `--session-commands-check` | 1 (기대대로 4건 실패, 원복함) |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC만이므로 없음

## 테스트

- 검증 기준 "창별 history와 save 호출 fixture":
  - `window_session::tests::undo_and_redo_replay_only_their_own_window_history`: 같은 프로젝트의 두 창에서 각자 편집·선택 후
    undo·redo가 자기 창 문서·선택·stack만 바꿈. 통과.
  - `save_now_writes_this_windows_document_as_a_new_generation`: 바뀐 것이 없으면 current pointer 그대로(written false), A 편집
    저장 → generation 2, 새로 연 창이 A 편집만 봄(B의 저장 안 한 편집 없음), 오래된 base의 B 저장은 Stale로 current pointer 불변,
    A undo 후 저장 → generation 3. 통과.
  - `save_now_keeps_other_documents_of_the_generation`: root 문서 2개 generation에서 첫 문서만 편집·저장해도 둘째 문서 chunk
    3개를 그대로 이어 쓰고 둘째 문서가 그대로 로드됨. 통과.
  - `app_window_tests::undo_redo_and_save_now_reach_only_the_target_window`: `AppRuntime` 요청·event로 창별 undo·redo·Save Now,
    같은 프로젝트 둘째 창의 Stale(`SaveStale`), 없는 창(`UnknownWindowSession`). 통과.
  - C ABI `undo_redo_and_save_now_go_to_the_given_window_session`: 선택 → undo·redo·Save Now가 지정한 창 session에만, 모르는
    명령 코드는 요청 없음. 통과.
  - 셸 `--session-commands-check`: 메뉴 항목으로 보낸 Undo·Redo·Save Now가 그 창 창 session에만 가고, 비활성 명령은 코어 요청을
    만들지 않으며, current pointer가 사라진 프로젝트의 Save Now는 Stale로 거부·alert 한 번·디스크 불변. 31 checks 통과.
- 남은 경고 0.

## 성능

- 해당 없음(성능 경로 변경 없음). Save Now 직렬화·기록은 document thread에서 실행되고 UI thread는 완료 event만 받습니다. 자동
  저장 debounce·worker 직렬화(XIII.1)는 이 행 범위 밖입니다.

## 차이

- 계획 1.2 규칙 5: 새 API라 구현 전 테스트는 컴파일 실패로만 재현됩니다. 셸 검사는 구현 뒤 변이로 실패를 확인했습니다.
- 셸에는 아직 캔버스 편집 도구가 없어, 셸 검사는 Undo 대상 history를 선택 변경(`stellar_window_session_select`)으로 만들고
  Save Now는 쓸 변경이 없는 경로와 Stale 경로를 확인합니다. 실제 generation 쓰기는 Rust 테스트가 확인합니다.
- Stale 저장은 아무것도 쓰지 않고 실패로 알립니다. 두 선택지(현재 창 저장 / 디스크 최신 불러오기) UI는 XII.7 범위입니다.
- Undo 메뉴 제목의 동작 이름("Undo Move")은 코어가 아직 주지 않아 "Undo"로 둡니다.

## 남은 위험

- Save Now는 document thread에서 동기로 직렬화·기록하므로 큰 문서에서는 그동안 다른 요청이 기다립니다(worker 직렬화 XIII.1.4).
- 자동 저장(XIII.1)과 앱 종료·비활성화 flush는 아직 연결하지 않았습니다. 정상 macOS 창 닫기의 편집 폐기는
  [ADR-0028 정상 창 닫기 저장 보존](../adr/ADR-0028-window-session-shell-surface.md)의 좁은 저장 확인 경로로 보완했습니다. XIII.1.6 전체 완료는 아닙니다.

## 2026-11-15 재검증

- Codex 작업 검토(계획 B9)에서 이 행의 상태를 다시 정했습니다. 위 기록은 Codex 작업 시간에 작성·통합되어 상태의 근거로 쓰지 않고, 아래 실행 결과를 근거로 씁니다.
- 논리 검토: 계획 U037 판정의 결함은 `--native-menus` 검사 모드뿐이며 B7(`e452f760`)에서 삭제했습니다.
- 환경: `DEV-MAC`(Apple M3 Max, macOS 27.2, rustc 1.96.0, Apple Git 2.54.0). 코드 기준은 `3463761b`이며 이 검토의 문서 커밋은 코드를 바꾸지 않습니다.

| 명령 | 결과 |
|---|---|
| `cargo test --workspace --no-fail-fast` | 0 — 4,039 통과, 0 실패, 10 ignored(수동 측정·장기 fuzz) |
| `Stellar.app --session-commands-check <fixture>` (`scripts/macos/smoke.sh`와 같은 인자로 개별 실행) | 0 — `session-commands ok: 56 checks (undo/redo/save now per window session via menu items)` |

- 검증 기준(창별 history와 save 호출 fixture): `window_session::tests::undo_and_redo_replay_only_their_own_window_history`, `save_now_writes_this_windows_document_as_a_new_generation`, `save_now_keeps_other_documents_of_the_generation`, `app_window_tests::undo_redo_and_save_now_reach_only_the_target_window`, `window_session::tests::undo_redo_and_save_now_go_to_the_given_window_session`과 위 앱 검사 통과.
- `scripts/macos/smoke.sh` 전체는 이 보고서를 처음 쓸 때 종료 1이었습니다(뒤 단계 `--frame-insert-check`, 두 번 재현). 그 뒤 검사는 위처럼 같은 인자로 따로 실행했습니다. 원인은 이 행과 무관한 정리 작업의 회귀였습니다. B3(`ed6bd994`)가 Git 패널 HEAD·변경 목록 표시(U136r)를 지운 뒤에도 검사가 그 표시를 단언했고, 그 실패에 가려 B4(`1fe6076c`, `cea6d84e`)가 바꾼 Layers 다음 페이지 행과 Shape 선택 Inspector의 낡은 단언도 남아 있었습니다. 조치 커밋 `1cdaee9f` 뒤 로컬 smoke 전체가 종료 0입니다(2026-11-15 `DEV-MAC`).
- 판정: `[~]` → `[x]`. 처음 구현(`64153be6`)은 `[x]`였고 2026-11-14 통합에서 사유 없이 `[~]`로 낮아졌습니다. 검증 기준을 모두 다시 실행해 통과했고 구현 커밋 `101bcd1d`가 있습니다.
