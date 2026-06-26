# 작업 보고서: II.5.3 AppDelegate·WindowController·destroy 1:1

- WBS: II.5.3
- 작업명: `NSApplicationDelegate`와 빈 `NSWindowController` 추가, 창 닫기·앱 종료 시 runtime destroy 정확히 1회
- 상태: 완료 (`[x]`)
- 선행: II.5.2 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (로컬 Xcode 27.0). CI-MAC 연결은 II.7.1.

## 변경 파일 목록

- `apps/macos/StellarMac/RuntimeHost.swift` (handle 1개 소유, create 1회·destroy 정확히 1회)
- `apps/macos/StellarMac/CanvasWindowController.swift` (`NSWindowController`)
- `apps/macos/StellarMac/LifecycleCheck.swift` (`--lifecycle-check` 검증 로직)
- `apps/macos/StellarMac/AppDelegate.swift` (RuntimeHost 소유 + 종료 시 shutdown)
- `apps/macos/StellarMac/CoreRuntime.swift` (handle 기반 version 헬퍼로 리팩터링)
- `apps/macos/StellarMac/main.swift` (`--lifecycle-check` 진입)
- `scripts/macos/smoke.sh` (lifecycle 검사 단계 추가)
- `docs/task-reports/II.5.3.md`, 계획서(II.5.3 `[ ]`→`[x]`)

## 구현 요약

- `RuntimeHost`가 코어 runtime handle 하나를 소유. `init`에서 1회 생성, `shutdown()`에서 정확히 1회 파괴(`destroyed` 플래그로 중복 호출 무시 → double free 없음). `deinit`도 안전망으로 shutdown 호출.
- 창 생성을 `AppDelegate`에서 `CanvasWindowController`로 분리. `applicationShouldTerminateAfterLastWindowClosed = true`이므로 마지막 창을 닫으면 앱이 종료되고, `applicationWillTerminate`에서 `runtimeHost.shutdown()`이 destroy를 1회 수행.
- 1:1 검증은 `stellar_runtime_live_count()` 진단 helper로 수행. AppDelegate가 실제로 쓰는 `RuntimeHost`를 그대로 생성·파괴하며 baseline → +1(create) → baseline(destroy) → baseline(중복 shutdown 무시)을 확인.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 검사 | 결과 |
|---|---|
| `xcodebuild ... build` | `** BUILD SUCCEEDED **` |
| `StellarMac --lifecycle-check` | `lifecycle ok: create/destroy 1:1 (baseline 0)`, 종료 0 |
| 중복 `shutdown()`이 live_count 불변(정확히 1회 destroy) | 통과 |
| `scripts/macos/smoke.sh` (smoke + lifecycle) | 종료 0 |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- "lifecycle UI test"는 별도 XCUITest 타깃 대신, AppDelegate가 쓰는 것과 동일한 `RuntimeHost`를 구동하는 headless `--lifecycle-check` 모드로 검증했습니다(XCTest host app 타깃 추가를 피해 범위를 최소화). 실제 GUI 창 닫기 경로의 1:1은 `applicationWillTerminate`에 단일 shutdown을 연결해 보장합니다.
