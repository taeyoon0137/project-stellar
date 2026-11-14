# 작업 보고서: XII.10.37 정밀 스크롤의 창별 팬 통합

## WBS

- WBS: XII.10.37
- 상태: `[~]` — 수식키 없는 정밀 스크롤과 관성 delta의 창별 camera 연결을 부분 구현했습니다.
- 통합 기준: `757f246b9ff823e0b42372ad4d0fbecf5da08914`
- 보존한 후보: `f27f182aea089a7d70f3e24d681661386787811a` (분기 기준 `bc5b2db2`)
- 후보 작업트리의 clean 상태, Git 객체·실제 diff·호스트 검증 기록을 확인하고 merge했습니다.
  최신 ADR-0028의 앱/CLI 공용 저장 잠금 설명을 보존했습니다.

## 변경 파일

- `apps/macos/StellarMac/CanvasView.swift`, `CoreRuntime.swift`: 정밀 스크롤 handler와 팬 FFI wrapper.
- `apps/macos/StellarMac/UI/Shell/ShellProjectWindow.swift`, `ShellRuntimeEventPump.swift`: 창 session 팬 요청과 완료 추적.
- `apps/macos/StellarMac/UI/Shell/ShellSessionCommandsCheck.swift`: 실제 코어를 쓰는 기본 창의 합성 입력 회귀 검사.
- `crates/ffi/src/window_session.rs`: 기존 `pan_viewport`의 C ABI와 창 격리·오류 검사.
- `tools/xtask/src/ffi_header.rs`, `crates/ffi/include/stellar_ffi.h`: 헤더 원본과 생성 결과.
- `docs/adr/ADR-0028-window-session-shell-surface.md`, `docs/status/canvas-scroll-pan.md`: ABI와 지원 범위.
- 구현 계획의 XII.10.37 상태와 생성 원장, 이 보고서: 부분 구현으로 정합화.

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test --locked --offline -p stellar-ffi` | 0, 71 passed |
| `cargo test --locked --offline -p stellar-runtime --lib zoom_commands_and_pan_change_only_the_target_viewport` | 0, 1 passed |
| `cargo test --locked --offline -p stellar-runtime --lib pan_and_zoom_move_the_scene_camera_without_changing_saved_bytes` | 0, 1 passed |
| `cargo run --locked --offline -q -p xtask -- generate-ffi-header --check` | 0, drift 없음 |
| `./scripts/macos/smoke.sh` | 0, session-commands 82 · window-close 98 · window-session 68 · key-mapping 101, smoke OK |
| `cargo xtask check-core` | 0, fmt·clippy 통과; workspace 3917 passed / 10 ignored, 별도 doctest 54 passed |
| `cargo xtask generate-ledger` | 0, 계획서의 부분 상태로 원장 재생성 |
| `cargo xtask sync-plan --check`, `validate-task-status`, `validate-adr`, `validate-crate-graph` | 각각 0 |
| `cargo xtask validate-task-report docs/task-reports/XII.10.37.md` | 0, 최초 표 헤더 형식 오류(exit 65) 수정 후 통과 |
| `git diff --check`, `git diff --cached --check` | 각각 0 |

## 환경과 증거

- DEV-MAC: macOS arm64 로컬 통합 작업트리. 후보의 과거 실행과 위 통합 후 재실행을 구분합니다.
- 외부 운영 기록과 상세 빌드 로그는 저장소 밖 인수인계에 보관합니다.
- 현재 원격 기준 `757f246b`의 [bootstrap 실행](https://github.com/taeyoon0137/project-stellar/actions/runs/43583914328)은 성공했습니다.
  이 실행은 새 통합 커밋의 원격 검증이 아닙니다.

## 테스트

실제 runtime fixture를 복사한 두 기본 프로젝트 창에서 시작·변경·관성 스크롤을 전달하고
배율에 따른 x/y 이동, 32개 연속 이동의 합계, 다른 창 보존, revision·undo·dirty·저장 bytes 불변을 검사합니다.
닫기 대기 플래그가 설정된 상태의 새 팬 접수 거부와 일반 휠·수식키를 팬 handler가 소비하지 않는지도 검사합니다.
FFI 검사는 두 창 격리와 비유한 delta·잘못된 session/runtime handle을 포함합니다.
macOS 빌드에는 AppIntents.framework 의존성이 없어 metadata 추출을 건너뛴다는 도구 경고 1건이 있습니다.
Swift 컴파일 오류는 없으며 UI 구조 검사도 통과했습니다. Rust 검증 로그의 경고는 0건입니다.

## 성능

T0→T5, Metal Present, 실제 장치 입력 지연은 측정하지 않았습니다. 요청마다 runtime snapshot을 만들며
열린 프로젝트·문서·창을 순회하므로 전체 경로의 상수 시간 성능을 주장하지 않습니다.

## 차이

전체 제스처 WBS가 아니라 검증된 정밀 스크롤 경로만 이력 보존 merge로 통합했습니다.
핀치 입력 ABI, 수식키 줌, Space/Hand/Zoom 도구, smart magnify의 미완료 상태는 유지합니다.
기존 요청 상한·오류 처리·저장 포맷·하단 툴바 디자인은 바꾸지 않았습니다.

## 남은 위험

- CanvasView는 배경만 그립니다. 문서 노드 표시·Metal 화면 팬은 아직 연결되지 않았습니다.
- 실제 트랙패드 방향·관성 체감과 Present 10ms 예산은 미검증입니다.
- `QueueFull`로 접수 거부된 delta의 재시도는 없습니다. 공용 입력 큐로의 fallback을 창별 팬 복구로 해석하지 않습니다.
- 팬 요청이 미완료인 상태의 실제 창 닫기와 공용 입력 큐의 중복 전달 여부는 직접 교차 검증하지 않았습니다.
- 창별 camera의 재실행 복구와 비 macOS 플랫폼은 이번 검증 대상이 아닙니다.
