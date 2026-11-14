# XXIV.5.1 macOS 앱에 L1 CLI 포함

- WBS: XXIV.5.1
- 상태: 부분 완료 (`[~]`). Developer ID 공증 후 clean runner 검증이 남아 있습니다.
- 선행: XVI.9.4, XXIV.4.1

## 필수 실행 환경

- `DEV-MAC`에서 Apple silicon, Debug 및 Release 앱을 검사합니다. `CI-MAC` 항목은 공증된 번들을 깨끗한 러너에서 실행하는 것입니다.

## 변경 파일 목록

- `apps/macos/Stellar.xcodeproj/project.pbxproj`, `scripts/macos/embed_cli.sh` — 앱 서명 전 Cargo CLI를 빌드해 번들에 설치합니다.
- `scripts/macos/codesign_bundle.sh`, `scripts/macos/sign_release.sh`, `scripts/macos/bundle_check.sh` — 중첩 CLI의 서명 순서와 검사를 추가합니다.
- `apps/macos/Stellar/UI/Welcome/WelcomeUICheck.swift`, `StellarCommandInstaller.swift`, `docs/design/ui/welcome.md` — 정상 번들에서 링크 설치와 실제 명령 실행을 확인합니다.
- `planning/task-status.yaml`, `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` — 구현 및 검증 상태를 기록합니다.

## 구현 요약

Debug와 Release는 `stellar-cli` package의 `stellar` Mach-O를 `Stellar.app/Contents/Helpers/stellar`에 둡니다. Cargo에서 실제 target 디렉터리를 읽으며 Release는 `aarch64-apple-darwin`을 사용합니다. 배포 서명은 CLI에 앱 entitlement를 전달하지 않고 `--options runtime` 및 앱과 같은 identity·timestamp 정책을 적용합니다. 이어서 Sparkle 중첩 코드와 앱을 서명합니다. Apple의 중첩 코드 표는 이 위치를 helper tool 디렉터리로 지정합니다. [Apple TN2206, Nested Code](https://developer.apple.com/library/archive/technotes/tn2206/_index.html)

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| Debug Xcode 빌드, `Stellar --welcome-ui-check` | 기존 preview는 `helperUnavailable`, 수정된 앱은 `ready(stellar)`. 임시 홈에서 사용자 동의 시 실행되는 installer로 링크를 만든 뒤 실제 `version` JSON을 확인했습니다. 수정된 Debug 및 Release 앱의 Welcome 검사 381개가 각각 통과했습니다. |
| `scripts/macos/release_build.sh` | Release 앱의 복사본에서 ad-hoc 서명, `codesign --verify --strict`, arm64, helper entitlement 없음, 재배치 후 최소 환경에서 `stellar --version` 실행이 모두 통과했습니다. |

## 차이(작업 지시와 다른 점)

실제 Developer ID로 서명·공증한 결과는 이 작업의 로컬 검증에 포함하지 않았습니다. 번들 설치는 L1 CLI를 대상으로 하며 L2 MCP helper의 배치는 XXIX.5.1입니다.

## 남은 위험 / 한계

`CI-MAC`에서 공증된 앱을 받아 Gatekeeper 확인 후 `Contents/Helpers/stellar --version`을 실행한 증거가 필요합니다. 새 성능 계약은 추가하지 않았습니다.
