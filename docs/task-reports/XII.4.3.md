# 작업 보고서: XII.4.3 viewport pan/zoom을 scene camera 입력으로 연결

> 아래 초기 구현·검증 기록은 선별 통합 원본 범위 `3892c5e3..2636e8ce`에서 가져왔습니다.
> 현재 기준 `eac4531b` 위의 재검증·수정·남은 범위는 [세션 통합 보고서](../status/session-takeover-integration.md)를 따릅니다.

## WBS

- WBS: XII.4.3
- 작업명: viewport pan/zoom matrix를 scene camera 입력으로 연결하고 document geometry는 변경하지 않습니다.
- 상태: `[x]` 판정 — 통합 때 계획서 표기를 `[x]`로 바꿉니다(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — 코어 계산·scene layer 생성까지 통합. 실제 CanvasHost 렌더 연결 미완료.
- 선행: XII.2.5 (`[x]`, 이 lane `59f275d`), XI.2.2 (`[x]`)

## 변경 파일

- `crates/commands/src/view_tabs.rs` (수정) — `CanvasViewport::world_to_view`(문서 → 화면 아핀 변환), `panned`, `zoomed`(anchor
  고정), `with_zoom`, `ZOOM_STEP`(×2), 테스트.
- `crates/runtime/src/window_session.rs` (수정) — `camera`(scene `Viewport`), `pan`, `zoom`, `zoom_to_actual_size`,
  `WindowSessionEditError::Viewport`, 검증 테스트와 테스트용 frame scene helper.
- `crates/runtime/src/app_command.rs`, `crates/runtime/src/app.rs` (수정) — `ZoomIn`(4)·`ZoomOut`(5)·`ZoomToActualSize`(6)와 공유
  이름, `pan_viewport`, 실패 코드 `InvalidViewport`(24).
- `crates/runtime/src/app_window_tests.rs` (수정) — runtime 경계 줌·이동 테스트.
- `crates/runtime/Cargo.toml`, `tools/xtask/src/crate_graph.rs`, `docs/architecture/crate-dependency-graph.md` (수정) —
  `stellar-runtime → stellar-scene` 허용 의존.
- `crates/ffi/src/window_session.rs`, `crates/ffi/src/lib.rs`, `tools/xtask/src/ffi_header.rs`, `crates/ffi/include/stellar_ffi.h`
  (수정·재생성) — `stellar_window_view_info`, `StellarWindowViewInfo`(56 byte), 줌 명령 코드·실패 코드, C ABI 테스트.
- `docs/adr/ADR-0028-window-session-shell-surface.md` (수정) — XII.4.3 절.
- `apps/macos/Stellar/CoreRuntime.swift`, `UI/Shell/ShellProjectWindow.swift` (수정) — view 정보 조회, 줌 명령 활성 상태
  (active view·배율 경계)와 툴바 배율 표시.
- `apps/macos/Stellar/UI/Shell/ShellSessionCommandsCheck.swift`, `ShellKeyMappingCheck.swift`, `ShellIntegrationCheck.swift`
  (수정) — 메뉴 줌·배율 표시·창 격리·줌 뒤 저장 bytes 검사, ⌘+·⌘0·⌘- 대응, 비활성 설명.
- `docs/design/ui/tools_menus.md` (수정) — 줌 남은 일 갱신.
- `docs/task-reports/XII.4.3.md`(신규), 계획서(XII.4.3 `[ ]`→`[~]`)

## 명령과 종료 코드

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-commands --lib view_tabs` | 0 (6 passed) |
| `cargo test -p stellar-runtime --lib` | 0 (61 passed) |
| `cargo test -p stellar-ffi` | 0 (63 passed) |
| `cargo run -q -p xtask -- validate-crate-graph` | 0 |
| `scripts/macos/smoke.sh` | 0 (`session-commands ok: 56 checks`, `key-mapping ok: 90 checks` 포함 `smoke OK`, Swift warning 0) |
| `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo fmt --all -- --check` | 0 (마지막 `check-core`에서 확인) |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: DEV-MAC만이므로 없음

## 테스트

- 검증 기준 "pan/zoom 후 save bytes 동일":
  - `window_session::tests::pan_and_zoom_move_the_scene_camera_without_changing_saved_bytes`: 기본 camera(문서 원점 → 화면 중심
    400, 300), 확대 ×2·이동·anchor 축소·100% 뒤 camera 변환 확인, 같은 scene에서 camera에 따라 가시 후보가 Frame 11 → Frame 12로
    바뀜(scene camera 입력 연결), 작업 문서 revision 0·history 없음·직렬화 chunk 동일, Save Now `written == false`, `design/` 아래
    파일 bytes 동일, 유한하지 않은 이동·0 배율은 viewport 불변. 통과.
  - `app_window_tests::zoom_commands_and_pan_change_only_the_target_viewport`: `AppRuntime` 줌 명령·`pan_viewport`가 그 창
    viewport만 바꾸고 history 없음, 저장 시 current pointer 불변, 무한 이동 `InvalidViewport`. 통과.
  - C ABI `zoom_commands_update_the_view_info_of_their_window`: 줌 명령 뒤 view 정보 배율, 다른 창 불변. 통과.
  - 셸 `--session-commands-check`: 메뉴 Zoom In·Out·Actual Size가 그 창 코어 배율과 툴바 배율 표시만 바꾸고, 줌 뒤 Save Now도
    디자인 폴더 bytes를 바꾸지 않음, 맞춤 줌은 비활성. 통과.
- 남은 경고 0.

## 성능

- 해당 없음(camera 계산은 상수 시간, 줌 명령은 document thread의 viewport 갱신 한 번).

## 차이

- 계획 1.2 규칙 5: 새 API라 구현 전 테스트는 컴파일 실패로만 재현됩니다.
- camera는 scene `Viewport`까지 연결했고 이를 쓰는 Metal 렌더는 XII.4.1(차단)이 연결되면 같은 값을 씁니다. 이 행 검증은 scene 가시
  후보 계산으로 camera 입력을 확인했습니다.
- 줌 단계는 ×2·÷2(화면 중심 고정)로 정했습니다(셸 Stub과 같은 단계, ADR-0028). Zoom to Fit·Zoom to Selection은 내용·선택 bounds
  계산이 필요해 비활성으로 두었습니다.
- 트랙패드·스크롤 입력 → 이동·확대 연결은 입력 파이프라인(XII.3.x) 범위라 runtime `pan_viewport`·`zoom`까지만 두었습니다.

## 남은 위험

- 이후 통합(8b0ac615)에서 macOS 프로젝트 창이 저장된 고정 Free Frame을 표시하게 됐습니다. 다만 그 렌더·clip·hit test·선택
  외곽선은 Swift 셸의 임시 Core Graphics 경로(`CanvasFrameScene`)가 코어 표시 목록을 다시 해석하는 구조라 아키텍처 §3.5·§10.1과
  다릅니다. 코어 SceneSnapshot을 Metal로 그리고 hit test를 코어 질의로 바꾸는 일(XI.8.2 다음 XII.4.1)에서 이 경로를 지웁니다.
  그 전까지 CG 경로에는 기능을 더하지 않습니다.
- 그 통합이 넣은 앱 bitmap 검사(저장 Frame 픽셀·hit·팬·선택 외곽선)는 2026-11-15 검토에서 삭제했습니다. 저장 Frame 표시의
  픽셀 단위 자동 검사는 없으며, 알려진 공백입니다. 순수 Rust 표시 목록 테스트(`canvas::tests`)만 남습니다.
- 큰 문서에서는 revision이 바뀔 때마다 active View의 표시 목록을 다시 평가하고, hit 후보를 painter 역순으로 확인하는 비용이
  남습니다.
- viewport는 저장하지 않으므로 창을 닫으면 사라집니다(복구 snapshot 연결은 V.9 범위).

## 2026-11-15 재검증

- Codex 작업 검토(계획 B9)에서 이 행의 상태를 다시 정했습니다. 위 기록은 Codex 작업 시간에 작성·통합되어 상태의 근거로 쓰지 않고, 아래 실행 결과를 근거로 씁니다.
- 논리 검토: 계획 U037 판정의 결함은 `--native-menus` 검사 모드뿐입니다. 스크롤 해석은 B8(`d98d583b`)에서 코어로 옮겼습니다.
- 환경: `DEV-MAC`(Apple M3 Max, macOS 27.2, rustc 1.96.0, Apple Git 2.54.0). 코드 기준은 `3463761b`이며 이 검토의 문서 커밋은 코드를 바꾸지 않습니다.

| 명령 | 결과 |
|---|---|
| `cargo test --workspace --no-fail-fast` | 0 — 4,039 통과, 0 실패, 10 ignored(수동 측정·장기 fuzz) |

- 검증 기준(pan/zoom 후 save bytes 동일): `window_session::tests::pan_and_zoom_move_the_scene_camera_without_changing_saved_bytes` 통과.
- 판정: `[~]` 유지. scene camera를 실제 캔버스 렌더가 소비하지 않습니다(XII.4.1).

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[x]` 판정 — 통합 때 계획서 표기를 `[x]`로 바꿉니다.
- 근거: 행 지시(viewport pan/zoom matrix를 scene camera 입력으로 연결, document geometry 불변)와 검증 열(pan/zoom 후 save bytes 동일)이 코어 코드와 test로 충족됩니다. 이전 `[~]` 사유인 "CanvasHost 렌더 연결"은 XII.4.1(Metal 렌더)의 범위이고 이 행 정의에 없습니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/commands/src/view_tabs.rs` `CanvasViewport::world_to_view`·`panned`·`zoomed`.
- `crates/runtime/src/window_session.rs` `camera`(665행): scene `Viewport`를 만듭니다. viewport는 창 session 상태이고 문서 command·history를 만들지 않습니다.
- `window_session::tests::pan_and_zoom_move_the_scene_camera_without_changing_saved_bytes`(1968행): camera 변화로 가시 후보가 바뀌고, revision·history·직렬화 chunk·`design/` 파일 bytes가 같고 Save Now가 쓰지 않습니다. `app_window_tests::zoom_commands_and_pan_change_only_the_target_viewport`(400행).

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-runtime` | 202 passed, 0 failed |
| `cargo test -p stellar-commands` | 123 passed |

### 남은 조건과 finding

- 이 판정으로 XII.4.4(선행 XI.3.5 `[x]`, XII.4.3)의 차단 사유가 사라집니다. XII.4.4 보고서를 다시 판정해야 합니다.
- Swift CG 표시 경로는 UI 재설계 대상이며 이 판정과 무관합니다.
