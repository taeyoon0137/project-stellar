# 정밀 스크롤로 창별 viewport 팬

기본 프로젝트 창의 CanvasView는 수식키 없는 정밀 스크롤과 관성 이벤트를 해당 창의
`pan_viewport`로 보냅니다. AppKit이 시스템 방향을 적용한 화면 point 이동량을 넘기면
코어가 배율에 따라 camera 중심을 이동합니다. 일반 휠과 수식키 조합은 기존 입력 경로가 받습니다.
이동 중에는 창의 배율·메뉴 상태를 다시 계산하지 않습니다. [창 session ABI](../adr/ADR-0028-window-session-shell-surface.md)

## 2026-11-14 macOS 호스트 검증

저장된 `crates/runtime/tests/fixtures/window-session-project`의 복사본을 열었습니다.

| 실행 | 관측 |
|---|---|
| `./scripts/macos/smoke.sh` | Debug 앱 컴파일·링크 후 화면 밖 기본 프로젝트 창에서 창 session 명령 82개 검사 |
| `StellarMac --session-commands-check crates/runtime/tests/fixtures/window-session-project` | 시작·변경·관성 이벤트와 50% 배율에서 x/y 이동량 확인. 대기 없이 보낸 32개 이벤트 합계 확인. 다른 창, 문서 revision·undo·dirty 및 Save Now 뒤 저장 파일 bytes 보존 |
| `cargo test --locked --offline -p stellar-ffi` | 71개 통과. 두 창 격리, 유효하지 않은 delta와 session 오류 경계 포함 |
| `cargo test --locked --offline -p stellar-runtime --lib zoom_commands_and_pan_change_only_the_target_viewport` | 기존 viewport의 배율·팬과 다른 창 보존 검사 |
| `cargo run --locked --offline -q -p xtask -- generate-ffi-header --check` | 생성 헤더와 원본 일치 |

초기 제한 환경에서 Xcode는 고정 Sparkle 패키지와 SwiftPM 캐시에 접근하지 못해 종료 코드 74를
반환했습니다. 호스트의 기존 패키지와 캐시를 이용한 위 실행에서 Swift 코드를 빌드했습니다.

검사는 기본 창에 전달한 합성 NSEvent와 코어 camera를 측정합니다. 실제 트랙패드 조작, Metal 화면
표시와 Present 지연, 핀치·수식키 줌·일시 Hand 도구는 후속 범위입니다. XII.10.37은 진행 중입니다.
