# 작업 보고서: II.5.2 Rust 코어 링크 + 시작 시 버전 호출

- WBS: II.5.2
- 작업명: Rust static library를 Debug 빌드에 연결, 앱 시작 시 `stellar_runtime_version` 호출·로그
- 상태: 완료 (`[x]`)
- 선행: II.5.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (로컬 Xcode 27.0). CI-MAC 연결은 II.7.1.

## 변경 파일 목록

- `apps/macos/StellarMac.xcodeproj/project.pbxproj` (cargo Run Script 단계 + 링크/헤더/브리징 설정)
- `apps/macos/StellarMac/StellarBridging.h` (Swift↔C bridging header)
- `apps/macos/StellarMac/CoreRuntime.swift` (FFI 래퍼: 버전 조회)
- `apps/macos/StellarMac/main.swift` (`--smoke` 모드 추가)
- `apps/macos/StellarMac/AppDelegate.swift` (시작 시 버전 로그)
- `scripts/macos/smoke.sh` (빌드+smoke 실행 검사)
- `docs/task-reports/II.5.2.md`, 계획서(II.5.2 `[ ]`→`[x]`)

## 구현 요약

- Xcode 타깃에 "Build Rust core (cargo)" Run Script 단계를 추가해 `xcodebuild build` 한 번으로 `cargo build -p stellar-ffi`를 실행하고 `target/debug/libstellar_ffi.a`를 생성. `ENABLE_USER_SCRIPT_SANDBOXING = NO`로 스크립트가 repo `target/`에 쓰도록 허용.
- 링크: `LIBRARY_SEARCH_PATHS = target/debug`, `OTHER_LDFLAGS = -lstellar_ffi -framework CoreFoundation -framework Security`. 헤더: `HEADER_SEARCH_PATHS = crates/ffi/include`, bridging header가 생성물 `stellar_ffi.h`를 include.
- `CoreRuntime.version()`이 FFI 규칙(길이 조회 → caller buffer 채우기)대로 버전을 읽음. `AppDelegate`는 시작 시 `NSLog`로 기록.
- smoke 경로: `StellarMac --smoke`는 GUI를 띄우지 않고 FFI로 버전을 읽어 출력 후 종료 코드 0/1 반환.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| `xcodebuild ... -configuration Debug ... build` (cargo 링크 포함) | `** BUILD SUCCEEDED **` |
| `StellarMac --smoke` | `stellar core version: 0.0.0`, 종료 코드 0 |
| `scripts/macos/smoke.sh` (빌드+실행 end-to-end) | `smoke OK`, 종료 코드 0 |

## 남은 경고 수

- 0 (Run Script가 매 빌드 실행된다는 Xcode note 1건은 의도된 동작 — cargo의 증분 빌드에 위임)

## 작업 지시와 다른 점

- "static 또는 dynamic library" 중 static library를 선택(dylib 경로 의존 회피).
- Debug·Release 두 config 모두 `target/debug`를 링크합니다(현 단계는 Debug 검증만 요구). Release용 cargo profile 매핑은 Release 빌드를 다룰 때 분리합니다.
