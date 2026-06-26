# 작업 보고서: II.5.1 macOS AppKit 앱 프로젝트 생성

- WBS: II.5.1
- 작업명: `apps/macos`에 macOS, arm64 전용 AppKit 앱 프로젝트 생성(캔버스는 AppKit view, 설정은 SwiftUI placeholder)
- 상태: 완료 (`[x]`)
- 선행: II.4.6 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (로컬 Xcode 27.0 / macOS SDK 27.0). CI-MAC 연결은 II.7.1에서 수행.

## 변경 파일 목록

- `apps/macos/StellarMac.xcodeproj/project.pbxproj` (objectVersion 77, app target 1개)
- `apps/macos/StellarMac.xcodeproj/xcshareddata/xcschemes/StellarMac.xcscheme` (공유 scheme)
- `apps/macos/StellarMac/main.swift`, `AppDelegate.swift`, `CanvasView.swift`, `SettingsPlaceholderView.swift`
- `docs/task-reports/II.5.1.md`, 계획서(II.5.1 `[ ]`→`[x]`)

## 구현 요약

- arm64 전용(`ARCHS = arm64`, `ONLY_ACTIVE_ARCH = YES`), `MACOSX_DEPLOYMENT_TARGET = 26.0`, `SDKROOT = macosx`. 스토리보드 없이 `main.swift`에서 `NSApplication`을 명시 구성.
- 캔버스는 AppKit `NSView` placeholder(`CanvasView`, 렌더링 없음), 설정 화면은 SwiftUI placeholder(`SettingsPlaceholderView`)로 분리.
- pbxproj는 `PBXFileSystemSynchronizedRootGroup`(Xcode 16+ synchronized group)을 사용해 `StellarMac/` 폴더의 소스가 타깃에 자동 포함되도록 함 — 파일 추가마다 pbxproj를 수정할 필요 없음.
- Info.plist는 `GENERATE_INFOPLIST_FILE = YES`로 생성(수동 plist 파일 없음).
- 빌드 산출물(DerivedData)은 저장소 밖(`~/Library/Developer/Xcode/DerivedData`)에 생성. `.gitignore`가 Xcode 산출물을 이미 무시.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| `xcodebuild -project ... -list` (target/scheme 인식) | StellarMac target·scheme 노출 |
| `DEVELOPER_DIR=.../Xcode-beta.app/... xcodebuild -project apps/macos/StellarMac.xcodeproj -scheme StellarMac -configuration Debug CODE_SIGNING_ALLOWED=NO build` | `** BUILD SUCCEEDED **` |

## 남은 경고 수

- 0 (빌드 경고 없음)

## 작업 지시와 다른 점

- "SwiftUI 설정용 placeholder target"은 별도 빌드 target을 분리하지 않고 앱 target 내 SwiftUI placeholder view(`SettingsPlaceholderView`)로 최소 구현했습니다. 별도 target 분리는 설정 화면 실제 구현 시점에 검토합니다.
- 로컬 `xcode-select`가 CommandLineTools를 가리켜, Xcode는 `DEVELOPER_DIR` 환경변수로 지정해 호출했습니다(전역 상태 변경 없음).
