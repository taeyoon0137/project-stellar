# 작업 보고서: XVI.9.2 Rust 코어·resources 번들 포함과 rpath·코드 서명 구조 검증

- WBS: XVI.9.2
- 작업명: Rust core library와 resources를 app bundle에 포함하고 rpath/code signature 구조 검증
- 상태: 부분 완료 (`[~]`) — 구현과 DEV-MAC 검증 완료, "clean machine launch smoke"는 깨끗한 장비 없이 미확보
- 선행: XVI.9.1 (`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (로컬 Release 빌드, ad-hoc 서명 사본, 재배치·sandbox 실행). 개발 도구가 없는 깨끗한 macOS 장비·VM은 사용하지 못했습니다.

## 변경 파일 목록

- `scripts/macos/bundle_check.sh`(신규) — 번들 구조 allowlist, Mach-O 목록, 동적 링크, LC_RPATH, C ABI 정적 포함,
  코드 서명 구조, 재배치 실행 smoke.
- `scripts/macos/release_build.sh` — 마지막에 `bundle_check.sh --adhoc-sign` 실행.
- `docs/task-reports/XVI.9.2.md`(신규), 계획서(XVI.9.2 `[ ]`→`[~]`).

## 구현 요약

- **Rust 코어 포함 방식**: `stellar-ffi`는 staticlib으로 실행 파일에 링크됩니다(XVI.9.1의 arm64 release 빌드). 번들 안에
  별도 dylib·framework가 없어 rpath·중첩 서명 대상이 없습니다. 검사는 생성 헤더(`stellar_ffi.h`)의 C ABI 함수 12개가 모두
  실행 파일에 정의되어 있는지 `nm`으로 확인합니다.
- **resources**: 번들에 넣을 런타임 resource는 아직 없습니다(Rust 코어는 런타임에 파일을 읽지 않고, 앱 아이콘 등 그래픽
  리소스는 저장소 지침상 임의로 만들지 않음). `StellarMac/` 폴더에 추가되는 resource는 Xcode synchronized group과 Resources
  phase로 `Contents/Resources`에 들어가며, allowlist가 이를 허용합니다.
- **bundle_check.sh**(하나라도 어긋나면 종료 1):
  1) 번들 항목 allowlist(`Info.plist`, `PkgInfo`, `MacOS/StellarMac`, `_CodeSignature/CodeResources`, `Resources/**`),
  2) Mach-O는 실행 파일 하나, 3) 동적 링크는 `/usr/lib`·`/System/Library`만, 4) LC_RPATH 없음 또는
  `@executable_path`/`@loader_path` 상대만, load command에 빌드 경로 없음, 5) C ABI 정적 포함,
  6) `codesign --verify --deep --strict`, Sealed Resources v2, Info.plist bound, Hardened Runtime, identifier = 번들 ID,
  7) 번들을 임시 폴더로 복사해 `env -i` 최소 환경 + sandbox로 저장소·DerivedData(`~/Library/Developer`)·Xcode·
  CommandLineTools·`~/.cargo`·`~/.rustup` 읽기를 막고 `--smoke`/`--lifecycle-check`/`--canvas-check` 실행(차단이 실제로
  동작하는지 먼저 확인).
- 서명되지 않은 Release 산출물은 `--adhoc-sign`으로 임시 사본에만 ad-hoc 서명해 구조를 검사합니다(원본 산출물 불변).
  Developer ID 서명 산출물은 XVI.9.3에서 같은 스크립트로 그대로 검사합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `scripts/macos/release_build.sh` | `BUILD SUCCEEDED`(Swift warning/error 0), XVI.9.1 검사 ✓, bundle 검사 7단계 ✓, 종료 0 |
| `otool -L .../StellarMac` | 시스템 라이브러리 23개(`/System/Library/Frameworks/*`, `/usr/lib/*`, `/usr/lib/swift/*`)만 |
| `otool -l .../StellarMac` LC_RPATH | 없음 |
| `codesign --verify --deep --strict --verbose=2`(ad-hoc 사본) | `valid on disk`, `satisfies its Designated Requirement` |
| sandbox 차단 확인: `sandbox-exec ... /bin/cat <repo>/Cargo.toml` | `Operation not permitted`(차단 동작) |
| 재배치·차단 실행 `--smoke` / `--lifecycle-check` / `--canvas-check` | `stellar core version: 0.0.0` / `lifecycle ok` / `canvas ok` |
| `git diff --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 번들 구조·Mach-O 1개·시스템 링크만·rpath 없음·C ABI 12개 정적 포함 | 통과 |
| 코드 서명 구조(ad-hoc + Hardened Runtime): deep strict verify, Sealed Resources v2, Info.plist bound | 통과 |
| 재배치 실행(개발 도구·빌드 트리 읽기 차단, 최소 환경) | 통과 |
| 검사 민감도: 번들에 staticlib(`MacOS/libstellar_ffi.a`) 혼입 → "허용되지 않은 번들 항목" 실패, `Frameworks/` 추가 → 실패, 미서명 번들 → "CodeResources 없음" 실패, `Resources/note.txt`는 통과 | 확인 |
| **clean machine launch smoke** | 미실행(깨끗한 장비 없음) |

## 차이(작업 지시와 다른 점)

- "clean machine" 대신 같은 개발 장비에서 재배치 + 최소 환경 + 개발 경로 읽기 차단(sandbox)으로 근사했습니다. 계획
  1.3의 5번 규칙(환경을 모방해 `[x]`로 바꾸지 않음)에 따라 `[~]`로 둡니다.
- 번들에 포함할 resource가 아직 없어 resource 포함은 경로·allowlist 수준으로만 검증했습니다.

## 남은 위험 / 한계

- 개발 도구·개발 인증서가 없는 깨끗한 macOS 26 장비(또는 VM)에서 Developer ID 서명·공증본 첫 실행 smoke가 필요합니다
  (XVI.9.4 공증본으로 수행하면 Gatekeeper 첫 실행까지 함께 확인 가능).
- `/usr/lib/swift/*`는 macOS 26 시스템 Swift runtime에 의존합니다(최소 OS 26.0과 일치).
- sandbox-exec는 deprecated 도구입니다. 향후 macOS에서 동작하지 않으면 재배치 실행 단계 대체가 필요합니다.

## 재평가(2026-11-14, Sparkle 내장 반영)

- **상태**: `[~]`를 유지합니다. 미확보 항목은 여전히 "clean machine launch smoke" 하나입니다.
  - 이 장비에서 쓸 수 있는 깨끗한 macOS 환경이 없습니다. Parallels에는 Windows 11 VM 하나만 있습니다
    (`prlctl list -a`). tart·UTM 같은 macOS VM 도구도 없습니다.
  - macOS VM을 새로 만들려면 restore image를 내려받고 사용자가 설정해야 합니다. 새 사용자 계정을 만드는 것은 시스템
    변경이라 사용자 확인이 필요합니다. 에이전트는 둘 다 하지 않았습니다.
  - 계획 §1.3 규칙 5에 따라, 같은 장비의 근사 검증으로 `[x]`로 올리지 않습니다.
- **번들 구조 변경**: 리뷰 수정 커밋 `5b993ac`(ADR-0017)로 `Contents/Frameworks/Sparkle.framework`가 번들에 추가되었습니다.
  `scripts/macos/bundle_check.sh`를 다음과 같이 바꿨습니다.
  - allowlist에 Sparkle.framework 하위를 추가했습니다. 헤더·`PrivateHeaders`·`Modules`와 다른 framework는 거부합니다.
  - Mach-O를 고정 목록 6개로 제한했습니다(실행 파일, framework, Autoupdate, Updater, Downloader, Installer). 모두 arm64
    slice를 포함해야 합니다.
  - 동적 링크는 시스템 경로와 `@rpath/Sparkle.framework/Versions/B/Sparkle`만 허용하고, rpath 대상이 번들 안에 있는지
    확인합니다.
  - 모든 중첩 코드가 Hardened Runtime이고 앱과 TeamIdentifier가 같아야 합니다.
  - 서명된 앱에 library validation을 끄는 entitlement가 있으면 실패로 처리합니다.
  - 재배치 실행에 `--update-check`를 추가했습니다. 번들 안 Sparkle.framework를 실제로 load합니다.
- **ad-hoc 사본 예외**: ad-hoc 서명에는 Team ID가 없습니다. 그래서 Hardened Runtime의 library validation이 번들 안
  framework load를 거부합니다(`mapping process and mapped file (non-platform) have different Team IDs`로 실제 확인).
  임시 검사 사본의 앱 실행 파일에만 `com.apple.security.cs.disable-library-validation`을 붙여 서명합니다. 배포 서명에는
  붙이지 않고, 서명본에서는 이 entitlement가 없는지 검사합니다.
- **서명 구조**: 안쪽부터 서명하는 `scripts/macos/codesign_bundle.sh`(신규)를 ad-hoc 검사와 배포 서명이 함께 씁니다.

| 명령 | 결과 |
|---|---|
| `scripts/macos/release_build.sh`(ad-hoc 사본) | 번들 검사 7단계 ✓(Mach-O 6개, 링크 70개 항목, rpath `@executable_path/../Frameworks`, 중첩 코드 5개), 재배치 `--smoke`/`--lifecycle-check`/`--canvas-check`/`--update-check` 통과, 종료 0 |
| `scripts/macos/sign_release.sh`(Developer ID 서명본에 같은 검사) | 같은 team·library validation 유지 상태로 재배치 4개 모드 통과, 종료 0 |

| 검사 | 결과 |
|---|---|
| 검사 민감도: `Contents/Frameworks/Other.framework` 추가 → `허용되지 않은 번들 항목` 실패 | 확인 |
| 검사 민감도: Sparkle `Versions/B/Headers` 추가 → `헤더가 남아 있음` 실패 | 확인 |
| 검사 민감도: framework Resources에 Mach-O 추가 → `Mach-O 파일 목록` 실패 | 확인 |
| **clean machine launch smoke** | 미실행(깨끗한 장비 없음) |
