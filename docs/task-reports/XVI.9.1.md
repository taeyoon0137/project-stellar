# 작업 보고서: XVI.9.1 macOS Release 구성·번들 ID·최소 OS·arm64 전용 고정

- WBS: XVI.9.1
- 작업명: Release configuration, bundle identifier, minimum macOS 26, arm64-only 설정 고정
- 상태: 완료 (`[x]`)
- 선행: Phase XII (단계 수준 선행, 완화 적용 — 아래 "차이")

## 필수 실행 환경

- `DEV-MAC` (로컬 Xcode 27.2 Release 빌드, `lipo`·`vtool`·`PlistBuddy` 검사).

## 변경 파일 목록

- `apps/macos/StellarMac.xcodeproj/project.pbxproj` — Rust 코어 build phase가 Release에서
  `cargo build -p stellar-ffi --release --locked --target aarch64-apple-darwin`을 실행, Release `LIBRARY_SEARCH_PATHS`를
  `target/aarch64-apple-darwin/release`로, project Release에 `ONLY_ACTIVE_ARCH = NO` 명시.
- `scripts/macos/release_build.sh`(신규) — Release 빌드와 구성 검사.
- `scripts/macos/sign_release.sh` — 사전 cargo 빌드를 같은 arm64 target으로 맞춤.
- `docs/task-reports/XVI.9.1.md`(신규), 계획서(XVI.9.1 `[ ]`→`[x]`).

## 구현 요약

- 기존 Xcode 설정(`ARCHS = arm64`, `MACOSX_DEPLOYMENT_TARGET = 26.0`, `PRODUCT_BUNDLE_IDENTIFIER = xyz.taeyoon.stellar`)은
  유지하고, 남아 있던 구멍을 막았습니다: Release 빌드가 build phase에서 항상 debug Rust 코어를 빌드하고
  `target/release`의 (별도로 빌드되지 않으면 없거나 오래된) staticlib을 링크하던 문제입니다. 이제 Release는 lockfile 고정
  arm64 release staticlib을 직접 빌드해 링크하므로, host 아키텍처와 무관하게 arm64 전용입니다.
- `release_build.sh`는 `-showBuildSettings`로 Release 고정값을 확인하고, 서명 없이 Release를 빌드한 뒤 실행 파일·Rust
  코어 아키텍처(arm64 전용), `LC_BUILD_VERSION`(MACOS, minos 26.0), Info.plist(번들 ID, `LSMinimumSystemVersion` 26.0,
  실행 파일 이름)를 검사하고 `--smoke`를 실행합니다. 하나라도 어긋나면 종료 1입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `scripts/macos/release_build.sh` | `BUILD SUCCEEDED`(Swift warning/error 0), 모든 검사 ✓, `stellar core version: 0.0.0`, 종료 0 |
| `lipo -archs .../Release/StellarMac.app/Contents/MacOS/StellarMac` | `arm64` |
| `lipo -archs target/aarch64-apple-darwin/release/libstellar_ffi.a` | `arm64` |
| `file .../StellarMac` | `Mach-O 64-bit executable arm64` |
| `xcrun vtool -show-build .../StellarMac` | `platform MACOS`, `minos 26.0`, `sdk 27.2` |
| `PlistBuddy -c "Print :LSMinimumSystemVersion"` | `26.0` |
| `shasum -a 256 .../StellarMac`(서명 전 Release) | `862274d87a0015f0fdb27671de65b27a8349783cc02575c4fcc26c1ba36c866e` |
| `scripts/macos/smoke.sh`(Debug 회귀) | `BUILD SUCCEEDED`, `smoke OK`, 종료 0 |
| `git diff --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **built binary architecture arm64 only**: Release 실행 파일과 링크된 Rust 코어가 모두 단일 `arm64`(x86_64·arm64e 없음) | 통과 |
| Release 설정 고정값(ARCHS, ONLY_ACTIVE_ARCH, 최소 OS, 번들 ID) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 "Phase XII"는 단계 수준 선행으로, 사용자 지시(2026-11-14)에 따라 완화했습니다. 이 작업이 실제로 필요로 하는
  Xcode 프로젝트·Release configuration은 b703259에 있습니다.
- Release 빌드의 Rust target을 `aarch64-apple-darwin`으로 명시했습니다(host가 arm64가 아니어도 arm64 전용 보장).

## 남은 위험 / 한계

- 깨끗한 CI(`CI-MAC`) runner에서는 실행하지 않았습니다. x86_64 host에서 실행하려면 `rustup target add aarch64-apple-darwin`이 필요합니다.
- 버전 번호(`MARKETING_VERSION` 0.0.0, `CURRENT_PROJECT_VERSION` 1) 정책은 정하지 않았습니다(업데이트 feed와 함께 결정 필요).

## 후속 수정 (리뷰 MF-1: 새 Rust target에서 Release 빌드 실패)

- **증상**: 비어 있는 Rust target에서 `scripts/macos/release_build.sh`가 Xcode Rust 코어 phase에서
  `error[E0463]: can't find crate for zerofrom_derive`(`bytemuck_derive`도 같음)로 실패했습니다. 이미 채워진 target에서는
  host 산출물을 다시 링크하지 않아 드러나지 않았습니다. X.9.1 text-shaping 의존성 통합(`c61485f`)으로 release 그래프에
  proc-macro crate가 들어온 뒤 생긴 문제입니다.
- **원인**: Xcode는 build setting `MACOSX_DEPLOYMENT_TARGET=26.0`을 phase 환경으로 넘깁니다. host와 target이 같은
  `aarch64-apple-darwin`이라 host 산출물인 proc-macro dylib도 이 값으로 링크되고, 12.0 이상에서는 chained fixups 형식이
  됩니다. release profile 기본 `strip = "debuginfo"`가 이 dylib를 `dyld_info` 기준 `mis-aligned LINKEDIT string pool`
  상태로 만들어 rustc가 proc-macro를 읽지 못합니다. 같은 `cargo build -p zerofrom --release --target aarch64-apple-darwin`이
  변수 없음·11.0에서는 통과하고 12.0·26.0에서는 실패했으며, strip을 끄면 26.0에서도 통과했습니다.
- **수정**: Release phase가 `CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP=none`으로 host 산출물(build script·proc-macro와 그
  의존성)만 strip하지 않습니다. 링크되는 staticlib과 deployment target(26.0)은 바뀌지 않습니다.
- **회귀 검사**: `scripts/macos/rust_core_phase_check.sh`(신규)는 pbxproj의 phase 스크립트를 그대로 꺼내 StellarMac
  build setting 환경과 빈 임시 `CARGO_TARGET_DIR`에서 실행합니다. 수정 전 Release는 E0463으로 종료 1, 수정 후 종료 0입니다.

| 명령 | 결과 |
|---|---|
| `scripts/macos/rust_core_phase_check.sh`(수정 전 phase) | `error[E0463]: can't find crate for zerofrom_derive`, 종료 1 |
| `scripts/macos/rust_core_phase_check.sh`(수정 후) | `✓ Rust 코어 phase(Release): 새 target에서 성공, MACOSX_DEPLOYMENT_TARGET=26.0, staticlib arm64`, 종료 0 |
| `scripts/macos/rust_core_phase_check.sh Debug` | 종료 0(dev profile은 strip하지 않아 원래 통과) |
| `plutil -lint apps/macos/StellarMac.xcodeproj/project.pbxproj` | `OK` |
