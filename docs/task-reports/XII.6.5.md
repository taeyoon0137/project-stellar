# 작업 보고서: XII.6.5 앱 메뉴 단축키와 코어 공유 command ID

> 아래 초기 구현·검증 기록은 선별 통합 원본 범위 `3892c5e3..2636e8ce`에서 가져왔습니다.
> 현재 기준 `eac4531b` 위의 재검증·수정·남은 범위는 [세션 통합 보고서](../status/session-takeover-integration.md)를 따릅니다.

## WBS

- WBS: XII.6.5
- 작업명: 앱 메뉴 단축키를 macOS·Windows native convention으로 연결하되 command ID는 core에서 공유합니다.
- 상태: 부분 완료 (`[~]`) — macOS 셸 연결과 응답자 체인 기반 key mapping smoke는 통과했습니다.
  Windows 셸 단축키(Ctrl+Z·Ctrl+Y·Ctrl+S)는 Windows 지원 보류(ADR-0006)로 구현하지 않았습니다.
- 선행: XII.6.4 (`[~]`, 이 lane `64153be`)

## 변경 파일

- `crates/runtime/src/app_command.rs` (수정) — 공유 이름(`edit.undo`·`edit.redo`·`file.save_now`), `name`·`from_name`, 테스트.
- `crates/ffi/src/window_session.rs` (수정) — `stellar_command_code`(공유 이름 → 코드, handle 불필요), C ABI 테스트.
- `crates/ffi/src/lib.rs`, `tools/xtask/src/ffi_header.rs`, `crates/ffi/include/stellar_ffi.h` (수정·재생성) — 함수 노출과 헤더 일치 테스트.
- `docs/adr/ADR-0028-window-session-shell-surface.md` (수정) — XII.6.5 절과 대안.
- `apps/macos/StellarMac/UI/Tools/ToolsMenuModel.swift` (수정) — `ShellCommandCoreIDStub`(임시 문자열)을 `ShellCommandCoreIDs`
  (코어가 공유 이름으로 알려 준 ID 표)로 교체.
- `apps/macos/StellarMac/UI/Tools/ToolsUICheck.swift`, `UI/Tools/ShellCommand.swift` (수정) — 표 규칙 검사, 문서 주석.
- `apps/macos/StellarMac/CoreRuntime.swift` (수정) — `commandCode(name:)`.
- `apps/macos/StellarMac/UI/Shell/ShellProjectWindow.swift`, `ShellAppCoordinator.swift` (수정) — 앱 구성 때 코어에서 표를
  만들어(`ShellCommandCoreIDs.fromCore`) 프로젝트 창에 넘기고, 셸 고정 코드 대응(`ShellWindowSessionCommand`)을 표 조회로 교체.
- `apps/macos/StellarMac/UI/Shell/ShellSessionCommandsCheck.swift` (수정) — 표 기반 코드 확인.
- `apps/macos/StellarMac/UI/Shell/ShellKeyMappingCheck.swift` (신규), `apps/macos/StellarMac/main.swift`, `scripts/macos/smoke.sh`
  (수정) — `--key-mapping-check <fixture>`.
- `docs/design/ui/tools_menus.md` (수정) — 파일 표·Stub 설명·남은 일 갱신.
- `docs/task-reports/XII.6.5.md`(신규), 계획서(XII.6.5 `[ ]`→`[~]`)

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `scripts/macos/smoke.sh` | 0 (`BUILD SUCCEEDED`, Swift warning 0, `tools-menus ok`(shortcut audit 0 issues), `key-mapping ok: 57 checks` 포함 `smoke OK`) |
| `cargo test -p stellar-ffi -p stellar-runtime` | 0 (ffi 62, runtime 59 + 8 + 2 + 11) |
| `cargo clippy -p stellar-runtime -p stellar-ffi -p xtask --all-targets -- -D warnings` | 0 |
| `cargo fmt --all -- --check` | 0 |
| `cargo run -q -p xtask -- generate-ffi-header` 후 헤더 일치 테스트 | 통과 |
| 변이: Redo 단축키를 ⌘Y로 바꾼 빌드의 `--key-mapping-check` | 1 (기대대로 3건 실패, 원복함) |

## 환경과 증거

- 필수 실행 환경: DEV-MAC(macOS 부분). Windows 증거는 ADR-0006으로 보류.
- 환경별 증거: DEV-MAC만이므로 없음

## 테스트

- 검증 기준 "key mapping smoke test"(`--key-mapping-check`, 실제 코어 runtime·메인 메뉴):
  - 코어 표: Undo·Redo·Save Now ID = `STELLAR_COMMAND_UNDO`·`_REDO`·`_SAVE_NOW`, 역 조회, 공유 이름 조회 일치, ID 중복 없음, 셸
    전용 명령 전부 코어 명령 아님.
  - 메뉴 단축키: `ShortcutPolicy` 감사 0건, Undo ⌘Z·Redo ⇧⌘Z·Save Now ⌘S(macOS HIG 관례), 키 조합 → 메뉴 항목 → 셸 명령 → 코어
    ID 대응, 대문자 입력도 같은 항목, ⌥⌘Z는 코어 명령 아님.
  - 저장 프로젝트 fixture 창: 같은 메뉴 항목 action을 그 창 router로 보내면 Undo·Redo는 `HISTORY_REPLAYED`, Save Now는
    `WINDOW_SESSION_SAVED`가 그 창 session에서 옴. 57 checks 통과.
- 코어·C ABI: `shared_names_are_stable_and_unique`, `command_codes_are_looked_up_by_shared_name`(셸 전용 이름 →
  `NO_ERROR`·0, 잘못된 UTF-8·null 포인터 거부). 통과.
- `--tools-menus-ui-check`: 표 규칙(셸 전용 명령은 ID 없음, 미구현 코어 명령은 ID 없음, 0 거부, 역 조회). 통과.
- 남은 경고 0.

## 성능

- 해당 없음. 표는 앱 구성 때 한 번 만들고(코어 소유 메뉴 명령 수만큼 순수 조회), 메뉴 검증 경로는 바뀌지 않았습니다.

## 차이

- 계획 1.2 규칙 5: 새 검사라 구현 전 실행할 수 없어, 구현 뒤 변이(Redo ⌘Y)로 실패를 확인했습니다.
- 코어가 아직 구현하지 않은 코어 소유 메뉴 명령(편집·객체·줌 등)은 ID가 없어 비활성입니다. 예전 Stub은 모든 코어 소유 명령에 임시
  문자열을 줬지만, 이제 표에는 코어가 실제로 아는 명령만 있습니다.
- 메뉴 막대 key equivalent 처리는 화면 밖 창이 key 창이 될 수 없어 AppKit `performKeyEquivalent` 대신 같은 규칙(문자·수식키,
  대문자 = Shift)으로 메뉴 항목을 찾고 그 action을 응답자 체인(창 router)으로 보냈습니다.

## 남은 위험

- Windows: Windows 셸이 생기면 같은 공유 이름·코드에 Ctrl+Z·Ctrl+Y(보조 Ctrl+Shift+Z)·Ctrl+S를 두고 같은 key mapping smoke를
  `LAB-WIN`에서 실행해야 `[x]`가 됩니다(ADR-0006).
- 실제 키보드 입력이 메뉴 막대를 거쳐 key 창에 가는 경로는 화면에 띄운 창에서 사람이 확인하지 않았습니다.
