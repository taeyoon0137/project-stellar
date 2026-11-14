# XII.6.1 macOS 새 프로젝트 생성

## WBS

- WBS: XII.6.1
- 상태: 부분 완료 (`[~]`), macOS 검증 완료
- 선행: IV.4.7, XII.2.2, XII.2.3

## 필수 실행 환경

- DEV-MAC에서 실제 Git, Rust 코어, AppKit 메뉴와 창을 사용했습니다. Windows 창 작업 XII.2.3은 ADR-0006에 따라 보류 상태입니다.

## 변경 파일 목록

- `crates/project/src/create.rs`, `crates/project/src/create/initial_document.rs`에서 첫 문서를 만듭니다.
- `crates/runtime/src/app.rs`, `crates/runtime/src/app/create.rs`, `crates/ffi/src/lib.rs`, `tools/xtask/src/ffi_header.rs`, `crates/ffi/include/stellar_ffi.h`에서 비동기 요청과 C ABI를 제공합니다.
- macOS의 `CoreRuntime.swift`, `ShellRuntimeEventPump.swift`, `ShellAppCoordinator.swift`, `ShellIntegrationCheck.swift`와 시작 창의 모델·컨트롤러·문구가 생성 경로를 사용합니다.

## 구현 요약

파일 메뉴의 New Project(⌘N)와 시작 창 버튼이 같은 이름 선택 패널을 엽니다. 상위 폴더와 이름을 받으면 코어가 Git 저장소, 프로젝트 매니페스트와 첫 generation을 만들고 기존 열기 경로가 문서 창을 엽니다. 초기 문서에는 페이지, 뷰, 흰색 800 × 600 Frame이 있습니다. Git 신원이 설정되어 있으면 초기 파일을 커밋합니다. 신원이나 최초 커밋 문제가 있으면 생성된 문서를 열고 다음 조치를 알립니다. 기존 폴더에 대한 생성 요청은 거절합니다.

## 실행한 명령 / 검증

| 명령 | 환경 | 종료 코드 |
| --- | --- | --- |
| `./scripts/macos/smoke.sh` | DEV-MAC | 0 |
| `Stellar --shell-integration-check` | DEV-MAC | 0 |

- `xcodebuild -project apps/macos/Stellar.xcodeproj -scheme Stellar -configuration Debug build` 성공.
- `Stellar --shell-integration-check` 136개 검사 통과. 실제 메뉴의 action이 이름 패널을 구성하고, 취소하면 대상 폴더가 생기지 않습니다. 한글 이름으로 생성하면 Git과 저장 파일이 생기고 generation 1이 열립니다. 같은 이름 보호와 닫은 뒤 같은 문서 ID로 다시 열기를 확인했습니다. 파일 패널 응답은 검사에서 주입했습니다.
- `cargo test -p stellar-ffi create_via_abi_reopens_in_an_editable_window_and_preserves_existing_folder`로 실제 C ABI의 생성·열기·기존 폴더 보호를 검사합니다.
- `cargo xtask generate-ffi-header --check`로 C 헤더와 원본 일치를 확인했습니다.

## 차이

macOS 범위의 구현과 검증을 기록했습니다. 공통 원장의 선행 항목 XII.2.3이 보류 상태이므로 전체 WBS는 reported_done으로 기록합니다.

## 남은 위험 / 한계

Git 첫 커밋에 필요한 신원 설정은 사용자 환경에 따릅니다. 생성 직후 문서 식별자와 저장 상태를 검사했으며 시각적 렌더링은 캔버스 검증에서 별도로 다룹니다.
