# 창 세션·메뉴·카메라·오버레이 선별 통합 기록

- 날짜·환경: 2026-11-14, DEV-MAC, macOS 27.2 arm64.
- 전용 checkout: `project-stellar-takeover-session`, 브랜치: `stellar-takeover-session`.
- 시작 상태: `git status --short` 출력 없음. 기준 `eac4531bd38f3c5dcac2c0e2743d8f2108b1ff3e`.
- 원격 확인: `git ls-remote origin refs/heads/main`도 위 기준을 반환했습니다.
- 원본 범위: `3892c5e3..2636e8ce`. 원본 작업 트리는 읽거나 변경하지 않고 커밋 객체의 패치만 사용했습니다.
- 다른 checkout·main·보호된 설정·계정·배포 파일을 수정하지 않았습니다. push는 수행하지 않았습니다.

## 통합 결과와 범위

| 범위 | 결과 | 현재 상태 |
|---|---|---|
| XII.2.1 | 프로젝트 identity·독립 문서·history·loaded generation을 소유하는 창 session | `[x]` |
| XII.2.2 | macOS 창 controller의 session 소유·비동기 열기·닫기, 두 창 lifecycle | `[x]` |
| XII.2.5 | 창별 탭·active view·viewport·selection 격리 | `[x]` |
| XII.6.4 | Undo·Redo·Save Now를 창 session에 전달, stale 저장 거부, 창별 fixture 통과 | `[~]` |
| XII.6.5 | 공유 command ID와 macOS 메뉴·단축키, 캔버스 응답자 체인 전달 | `[~]`: Windows 미완료 |
| XII.4.3 | 코어 camera 변환·pan/zoom·줌 메뉴, 문서·저장 bytes 불변 | `[~]`: 실제 캔버스 renderer 연결 미완료 |
| XII.4.4 | 별도 selection overlay scene·hit-test layer, project snapshot 비포함 | `[~]`: 실제 화면 렌더링 미완료 |
| XII.4.1 / XI.8.2 | CanvasHost 연결과 실제 최소화·가림 검증의 선행 순환 | 현행 `[ ]` / `[~]` 유지; 이 lane에서 미통합 |

WBS 설명·검증 열·선행은 변경하지 않았습니다. 기준에 이미 완료된 다른 행도 그대로입니다. 원본 패치의 XII.4.3·XII.4.4 완료 표시는 채택하지 않았고, 원본 원장 커밋 `2636e8ce`도 복사하지 않았습니다. 현행 계획과 이 브랜치의 WBS별 커밋으로 원장을 재생성했습니다.

## 변경 파일

- `crates/runtime/src/window_session.rs`, `app.rs`, `app_command.rs`, `app_window_tests.rs`, `lib.rs`, `Cargo.toml`: session·명령·view state·camera 경로와 검사.
- `crates/commands/src/view_tabs.rs`, `selection.rs`, `lib.rs`, `crates/history/src/window.rs`: 창별 view state.
- `crates/scene/src/overlay.rs`, `lib.rs`: 비영속 선택 레이어 생성.
- `crates/ffi/src/window_session.rs`, `lib.rs`, `crates/ffi/include/stellar_ffi.h`, `tools/xtask/src/ffi_header.rs`: C ABI와 생성 헤더.
- `apps/macos/Stellar/CoreRuntime.swift`, `UI/Shell/`, `UI/Tools/`, `main.swift`: session 소유·메뉴 연결·검사. 기존 하단 도구 막대 구현은 보존했습니다.
- `crates/runtime/tests/fixtures/window-session-project/`, `Cargo.lock`, `scripts/macos/smoke.sh`: fixture·의존 그래프 반영·smoke 항목.
- `tools/xtask/src/crate_graph.rs`, `docs/architecture/crate-dependency-graph.md`, `docs/adr/ADR-0028-window-session-shell-surface.md`: runtime→scene 허용 근거.
- 구현 계획·원장·개별 작업 보고서와 `docs/design/ui/{shell_frame,tools_menus}.md`: 실제 통합 범위 기록.

## 후속 변경 보존과 방어적 수정

- 최신 기준의 하단 도구 막대, 화면 픽셀 반올림 검사, runtime 종료 timeout을 유지했습니다.
- `ShellAppCoordinator.swift`의 시작 창 작업 ID API, 닫힌 창의 늦은 열기 결과 폐기, 일반 파일과 없는 폴더의 오류 구분을 유지했습니다. 겹친 3개 파일을 개별 병합했고 전체 incoming 선택으로 충돌을 해결하지 않았습니다.
- 복원 창은 프로젝트 열기 다음에 창 session을 여는 두 단계입니다. 요청 ID를 두 번째 완료까지 유지하고, 닫힘·새 요청 뒤 도착한 session은 닫습니다. 기존 session을 다시 열 때는 이전 창·프로젝트 session을 먼저 닫습니다.
- `ShellIntegrationCheck`는 첫 단계 전과 두 번째 단계 대기 중의 닫기를 모두 검사하며, 닫힌 객체를 계속 보유해도 session이 다시 붙지 않고 코어 session 수가 0임을 확인합니다.
- 메뉴 검사는 router를 직접 target으로 지정하는 대신 캔버스 first responder부터 메뉴 action을 전달하고 두 창의 session 격리를 확인합니다.

## 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-commands -p stellar-history -p stellar-runtime -p stellar-scene -p stellar-ffi --lib` | 종료 0, 374 통과·기존 2 제외(5만 노드 release 측정, 장기 nightly fuzz) |
| `cargo test -p stellar-runtime -p stellar-ffi` | 종료 0, library·통합·doc test 통과 |
| `cargo clippy -p stellar-commands -p stellar-history -p stellar-runtime -p stellar-scene -p stellar-ffi -p xtask --all-targets -- -D warnings` | 종료 0 |
| `cargo fmt -p stellar-commands -p stellar-history -p stellar-runtime -p stellar-scene -p stellar-ffi -p xtask -- --check` | 종료 0 |
| `cargo xtask generate-ffi-header` | 종료 0, 원본 generator로 헤더 재생성 |
| `cargo xtask generate-ledger` | 종료 0, validator·round-trip을 통과한 1,145행 재생성 |
| `cargo xtask sync-plan --check` | 종료 0 |
| `cargo xtask validate-task-status` | 종료 0 |
| `cargo xtask validate-crate-graph` | 종료 0 |
| `cargo xtask validate-adr crates/ffi/src/lib.rs crates/ffi/src/window_session.rs crates/runtime/Cargo.toml docs/adr/ADR-0028-window-session-shell-surface.md` | 종료 0 |
| `scripts/macos/smoke.sh` | 종료 0, `smoke OK` |
| `git diff --check` | 종료 0 |

macOS smoke 결과: shell 통합 120, 프로젝트 열기 177, 창 session 68, session 명령 56, 메뉴 대응 101 checks 통과. 기존 하단 도구 막대·native text Undo/Redo·IME·전체 영역 UI 검사도 통과했습니다. Xcode의 AppIntents framework 미사용 metadata 경고 1건은 남았고 Swift 소스 컴파일 오류는 없습니다.

처음 추가한 검사에 존재하지 않는 속성 참조가 있어 Xcode 빌드가 종료 65로 실패했습니다. 실제 `canvasHost.canvasView`로 수정한 뒤 전체 smoke를 재실행해 통과했습니다.

원시 로그는 `/tmp/stellar-session-cargo-test.log`, `/tmp/stellar-session-integration-test.log`, `/tmp/stellar-session-clippy.log`, `/tmp/stellar-session-macos-smoke.log`입니다. `/tmp` 로그는 영구 보관물이 아니므로 이 보고서의 명령·결과를 인수인계 기준으로 사용합니다.

## 인수인계

- WBS별 커밋: `5e1edbf4`(XII.2.1), `7b0ccfa3`(XII.2.2), `58b1aafe`(XII.2.5), `101bcd1d`(XII.6.4), `f23060f5`(XII.6.5), `91833750`(XII.4.3), `4d54410b`(XII.4.4). 선행 순환 보고는 `263c8c48`입니다. 뒤의 통합 보강·원장 커밋까지 함께 가져가야 합니다.
- XII.4.1과 XI.8.2의 순환은 해결하지 않았습니다. 실제 NSWindow 최소화·가림의 Metal 증거와 CanvasHost 연결이 필요합니다.
- scene camera·overlay는 아직 실제 CanvasHost 렌더에 소비되지 않습니다. 문서→scene 변환, 입력 파이프라인의 팬·줌, overlay 스타일, 대규모 선택 성능은 별도 범위입니다.
- 창 session은 첫 root 문서 하나를 로드합니다. 같은 경로를 UI에서 여러 창에 여는 정책, 충돌 선택 UI, autosave 통합은 남았습니다.
- 최종 전체 검사와 push는 통합 담당 범위입니다. 이 lane에서는 추가 agent·스케줄·상시 프로세스를 만들지 않았습니다.
