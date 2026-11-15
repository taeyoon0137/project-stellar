# 작업 보고서: XII.2.2 macOS 창 controller의 ProjectWindowSession handle 소유

> 아래 초기 구현·검증 기록은 선별 통합 원본 범위 `3892c5e3..2636e8ce`에서 가져왔습니다.
> 현재 기준 `eac4531b` 위의 재검증·수정·남은 범위는 [세션 통합 보고서](../status/session-takeover-integration.md)를 따릅니다.

## WBS

- WBS: XII.2.2
- 작업명: macOS `NSWindowController`가 한 ProjectWindowSession handle을 소유하도록 연결합니다.
- 상태: 완료 (`[x]`)
- 선행: II.5.3 (`[x]`), XII.2.1 (`[x]`, 이 lane `6d0785e`)

## 변경 파일

- `crates/runtime/src/app.rs` (수정) — 창 session 표(`WindowSessionHandle`, 프로젝트별 창 집합, UUIDv7 창 id 공급기),
  `open_window_session`·`close_window_session`, event `WindowSessionOpened`(0x0106)·`WindowSessionClosed`(0x0107), 실패 코드
  `UnknownWindowSession`(4)·`WindowSessionLoadFailed`(18), 상태 snapshot `windows`(`WindowSessionSummary`), 프로젝트 닫기 시
  창 session 폐기.
- `crates/runtime/src/app_window_tests.rs` (신규) — 앱 runtime 경계 창 session 테스트 3개.
- `crates/runtime/src/window_session.rs` (수정) — 저장 프로젝트 fixture golden 테스트(`committed_fixture_project_matches_the_sample_project`).
- `crates/runtime/src/lib.rs` (수정) — 테스트 모듈 연결.
- `crates/runtime/tests/fixtures/window-session-project/` (신규) — 저장 프로젝트 fixture(`stellar.yaml`, lockfile, generation 1
  manifest·chunk 5개, 바이트 보존 `.gitattributes`). 셸 검사와 FFI 테스트가 복사해 씁니다.
- `crates/ffi/src/window_session.rs` (신규) — `stellar_window_session_open`·`_close`·`_info`, `StellarWindowSessionInfo`(88 byte),
  C ABI 테스트 3개.
- `crates/ffi/src/lib.rs` (수정) — 모듈 연결, `StellarRuntimeState` version 2(`window_session_count`, 48 byte), 헤더 일치 테스트 갱신.
- `tools/xtask/src/ffi_header.rs`, `crates/ffi/include/stellar_ffi.h` (수정·재생성) — 새 함수·구조체·event·실패 코드.
- `docs/adr/ADR-0028-window-session-shell-surface.md` (신규) — C ABI 확장 기록(ADR 보호 경로 `crates/ffi/`).
- `apps/macos/Stellar/CoreRuntime.swift` (수정) — 창 session 호출·정보 조회, event 해석.
- `apps/macos/Stellar/UI/Shell/ShellRuntimeEventPump.swift` (수정) — 창 session 요청 등록.
- `apps/macos/Stellar/UI/Shell/ShellProjectWindow.swift` (수정) — `ShellWindowSessionHandle`, 창 open 시 창 controller에 handle
  지정, 닫기 순서(`closeSessions`).
- `apps/macos/Stellar/UI/Shell/ShellWindowController.swift` (수정) — `windowSession` 소유 property.
- `apps/macos/Stellar/UI/Shell/ShellAppCoordinator.swift` (수정) — 프로젝트 열기 → 창 session 열기 → 창 표시(첫 열기·복원·다시
  열기), 창 session 실패 시 프로젝트 session 닫기·안내, 창 닫기 시 두 session 닫기.
- `apps/macos/Stellar/UI/Shell/ShellProjectOpenFailure.swift`, `ShellProjectOpenCheck.swift` (수정) — `designUnavailable`
  안내(`WINDOW_SESSION_LOAD_FAILED`)와 대응표 검사.
- `apps/macos/Stellar/UI/Shell/ShellWindowSessionCheck.swift` (신규), `apps/macos/Stellar/main.swift`,
  `scripts/macos/smoke.sh` (수정) — `--window-session-check <fixture>`.
- `docs/design/ui/shell_frame.md` (수정) — 13장 남은 일에서 창 session 연결을 끝난 일로 갱신.
- `docs/task-reports/XII.2.2.md`(신규), 계획서(XII.2.2 `[ ]`→`[x]`)

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `scripts/macos/smoke.sh` | 0 (`BUILD SUCCEEDED`, Swift warning 0, 기존 검사 전부와 `window-session ok: 68 checks` 포함 `smoke OK`) |
| `cargo test -p stellar-runtime -p stellar-ffi -p xtask` | 0 (runtime 51 + 8 + 2 + 11, ffi 60, xtask 169 등 전부 통과) |
| `cargo clippy -p stellar-runtime -p stellar-ffi -p xtask --all-targets -- -D warnings` | 0 |
| `cargo fmt --all -- --check` | 0 |
| `cargo run -q -p xtask -- validate-adr crates/ffi/src/lib.rs docs/adr/ADR-0028-window-session-shell-surface.md` | 0 |
| `cargo run -q -p xtask -- generate-ffi-header` 후 `committed_header_matches_template` | 통과 |
| 변이: `didOpen`에서 창 controller에 handle을 넣지 않게 바꾼 빌드의 `--window-session-check` | 1 (기대대로 실패, 원복함) |
| `git diff --check` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC (Xcode 27.2, macOS 27.2, 화면 밖 창 검사)
- 환경별 증거: DEV-MAC만이므로 없음

## 테스트

- 검증 기준 "창 2개 open/close lifecycle 통과": `--window-session-check`가 실제 코어 runtime으로 fixture를 복사한 두 프로젝트를
  열어 창 controller마다 창 session handle 하나(서로 다른 handle·창 편집 id), 정보(소속 프로젝트, 작업 문서, generation 1,
  revision 0, 빈 history·선택)를 확인하고, 첫 창을 닫으면 그 창 session만 닫혀 코어 수가 (프로젝트 2, 창 2) → (1, 1), 둘째 창까지
  닫으면 (0, 0)이 되는지 확인합니다. 같은 cycle을 두 번 돌리고, 저장한 적 없는 프로젝트(작업 문서 없음)도 엽니다. 68 checks 통과.
- 코어: `two_windows_open_and_close_with_one_session_each`, `closing_a_project_closes_its_window_sessions`,
  `window_session_failures_have_fixed_codes`(없는 프로젝트 → `UnknownProject`, 손상 generation → `WindowSessionLoadFailed`, 빈
  프로젝트 → NIL 문서), event payload 고정 배치·창 id 1,000개 중복 없음, fixture golden. 통과.
- C ABI: `two_window_sessions_open_and_close_through_the_c_abi`, `info_layout_is_fixed`, `null_info_pointer_and_unknown_handle_are_rejected`. 통과.
- 기존 셸 검사(`--shell-integration-check`, `--project-open-check` 등)는 창 session 단계를 더한 열기 경로로 그대로 통과합니다.
- 남은 경고 0.

## 성능

- 해당 없음(성능 경로 변경 없음). 창 session 정보 조회는 상태 snapshot O(1) 읽기입니다.

## 차이

- 계획 1.2 규칙 5: 셸 검사는 새 기능이라 구현 전 실행할 수 없어, 구현 뒤 변이(handle 미지정)로 검사가 실패하는 것을 확인했습니다.
- 셸 검사의 저장 프로젝트는 Rust가 만든 fixture를 저장소에 두고 복사합니다(Swift에서 저장 형식을 쓰지 않기 위해). fixture는
  runtime golden 테스트가 sample 프로젝트 저장 결과와 같은지 확인합니다(`STELLAR_UPDATE_FIXTURES=1`로 재생성).
- 창 session을 열지 못하면(손상 generation) 창을 만들지 않고 프로젝트 session을 닫은 뒤 새 안내 `designUnavailable`로 알립니다.

## 남은 위험

- 같은 폴더를 두 창에서 여는 셸 정책은 그대로(같은 폴더면 기존 창을 앞으로)입니다. 같은 프로젝트 다중 창은 XII.2.4 범위입니다.
- 창 session은 작업 문서 하나(generation의 첫 root 문서)만 로드합니다.
- 캔버스는 아직 창 session의 문서를 그리지 않습니다(Metal 연결 XII.4.1).

## 2026-11-15 재검증

- Codex 작업 검토(계획 B9)에서 이 행의 상태를 다시 정했습니다. 위 기록은 Codex 작업 시간에 작성·통합되어 상태의 근거로 쓰지 않고, 아래 실행 결과를 근거로 씁니다.
- 논리 검토: 계획 U037 판정의 결함은 `--native-menus` 검사 모드뿐이며 B7(`e452f760`)에서 삭제했습니다.
- 환경: `DEV-MAC`(Apple M3 Max, macOS 27.2, rustc 1.96.0, Apple Git 2.54.0). 코드 기준은 `3463761b`이며 이 검토의 문서 커밋은 코드를 바꾸지 않습니다.

| 명령 | 결과 |
|---|---|
| `cargo test --workspace --no-fail-fast` | 0 — 4,039 통과, 0 실패, 10 ignored(수동 측정·장기 fuzz) |
| `Stellar.app --window-session-check <fixture>` (`scripts/macos/smoke.sh`와 같은 인자로 개별 실행) | 0 — `window-session ok: 68 checks (2 windows × 2 cycles open/close, one session handle per window controller)` |

- 검증 기준(창 2개 open/close lifecycle): 위 앱 검사와 `app_window_tests::two_windows_open_and_close_with_one_session_each`, `window_session::tests::two_window_sessions_open_and_close_through_the_c_abi` 통과.
- `scripts/macos/smoke.sh` 전체는 이 보고서를 처음 쓸 때 종료 1이었습니다(뒤 단계 `--frame-insert-check`, 두 번 재현). 그 뒤 검사는 위처럼 같은 인자로 따로 실행했습니다. 원인은 이 행과 무관한 정리 작업의 회귀였습니다. B3(`ed6bd994`)가 Git 패널 HEAD·변경 목록 표시(U136r)를 지운 뒤에도 검사가 그 표시를 단언했고, 그 실패에 가려 B4(`1fe6076c`, `cea6d84e`)가 바꾼 Layers 다음 페이지 행과 Shape 선택 Inspector의 낡은 단언도 남아 있었습니다. 실패하던 `--frame-insert-check`(`ShellFrameInsertCheck`)는 UX.2.3 속성 패널 연결(`587de247`)이 옛 인스펙터 틀과 함께 지웠고, 정리 쪽 조치 커밋 `1cdaee9f`는 그 삭제 뒤 빈 커밋이 되어 이 history에 들어오지 않았습니다. UX 1단계 통합 위로 정리를 옮긴 브랜치에서 로컬 `scripts/macos/smoke.sh` 전체가 종료 0입니다(2026-11-15 `DEV-MAC`, 교차 검토 반영 뒤 다시 실행).
- 판정: `[x]` 유지.
