# 작업 보고서: XVI.9.5 Stable/Beta 업데이트 feed 분리와 background 확인

- WBS: XVI.9.5
- 작업명: Stable/Beta update feed 분리, 앱은 background check만 수행, 다운로드·설치는 사용자 승인 후
- 상태: 부분 완료 (`[~]`)
  - 엄격 선행 XVI.9.2가 `[~]`라서 `[x]`로 올릴 수 없습니다(계획 §1.2 규칙 4). XVI.9.2는 깨끗한 장비 실행을
    확보하지 못했습니다.
  - 구현과 channel isolation fixture는 DEV-MAC에서 통과했습니다.
- 선행: XVI.9.2 (`[~]`, 엄격 선행이며 충족되지 않음. 아래 "차이" 참조)

## 필수 실행 환경

- `DEV-MAC`: 로컬 Xcode 27.2 Debug·Release 빌드, headless `--update-check`, Developer ID 서명본 재배치 실행.
- 외부 feed 서버와 외부 네트워크는 쓰지 않았습니다. fixture는 127.0.0.1 loopback 서버입니다.

## 변경 파일 목록

현재 구현은 리뷰 수정 커밋 `5b993ac`(`Fix:`, ADR-0017)입니다. 첫 구현 `afedb93`의 자체 JSON feed를 이 커밋이
대체했습니다.

- `apps/macos/StellarMac/UpdateChecker.swift`:
  - `UpdateChannel`: 선호 채널, Sparkle `allowedChannels`.
  - `UpdateConfiguration`: Info.plist 구성 검증.
  - `UpdateChannelPolicy`·`UpdateChannelDelegate`: 채널별 appcast URL과 channel 필터.
  - `AppUpdateController`: 구성이 완전할 때만 Sparkle updater와 표준 UI를 시작.
- `apps/macos/StellarMac/UpdateCheck.swift`: `--update-check` fixture(실제 Sparkle, loopback HTTP 서버, 고정 서명
  appcast).
- `apps/macos/StellarMac/AppDelegate.swift`, `apps/macos/StellarMac/main.swift`: controller 연결, 모드 설명.
- `apps/macos/StellarMac/Info.plist`(신규): 업데이트 key.
- `apps/macos/StellarMac.xcodeproj/project.pbxproj`: Sparkle package, `INFOPLIST_FILE`, rpath, `STELLAR_UPDATE_*`.
- `apps/macos/StellarMac.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved`(신규).
- `config/dependencies.yaml`, `docs/adr/ADR-0017-sparkle-updates.md`(신규).
- `scripts/macos/smoke.sh`, `scripts/macos/release_build.sh`: package 해석 플래그, Info.plist 업데이트 구성 검사.
- 번들·서명 스크립트(`bundle_check.sh`, `codesign_bundle.sh`, `sign_release.sh`): XVI.9.2·XVI.9.3 보고서 참조.
- `docs/task-reports/XVI.9.5.md`(이 문서).

## 구현 요약

- **채널 분리**:
  - Stable은 `SUFeedURL`, Beta는 `StellarUpdateFeedBeta`로 서로 다른 appcast를 씁니다. delegate는 선호 채널 URL
    하나만 Sparkle에 줍니다.
  - Sparkle channel 필터도 함께 씁니다. Stable은 기본 채널만 받고, Beta는 기본 채널과 `beta`를 받습니다.
  - 그래서 Stable appcast에 beta 항목이 잘못 올라가거나 Stable URL에 Beta appcast가 배포되어도 Stable 사용자는 beta
    build를 받지 않습니다.
  - 최소 OS 검사는 Sparkle `sparkle:minimumSystemVersion`이 합니다(아키텍처 §40.4).
- **background 확인만**:
  - `SUEnableAutomaticChecks = YES`, `SUScheduledCheckInterval = 86400`으로 하루 1회 확인합니다.
  - `SUAllowsAutomaticUpdates = NO`, `SUAutomaticallyUpdate = NO`라 자동 다운로드·설치가 없고, 사용자가 켤 선택지도
    없습니다.
- **승인 gate**: Sparkle 표준 UI가 새 버전을 알립니다. 사용자가 설치를 고른 뒤에만 다운로드하고, 설치·재실행도
  사용자가 고른 뒤에만 합니다.
- **무결성**: `SURequireSignedFeed`와 `SUVerifyUpdateBeforeExtraction`을 켜서 appcast와 archive 모두 EdDSA 서명이
  맞아야 합니다. 이전의 TLS 전용 보호를 대체합니다.
- **활성화 조건(리뷰 지적 수정)**:
  - feed URL은 build setting에서 부분 Info.plist를 거쳐 실제 번들 Info.plist에 들어갑니다(`release_build.sh`가 확인).
  - 앱은 구성이 완전할 때만 updater를 시작합니다. 조건은 두 URL이 https이고 서로 다를 것, 서명 요구, 자동 설치 금지,
    32바이트 공개 키입니다.
  - 기본 feed는 공개 릴리스 저장소 placeholder이고 저장소는 아직 없습니다. 공개 키는 사용자가 만들 때까지 비어 있어
    현재 빌드는 `disabled(missingPublicKey)`로 시작하지 않습니다.
  - 테스트는 이 상태를 통과 조건으로 고정하지 않습니다. 대신 다음 세 가지를 각각 확인합니다.
    - 번들 Info.plist가 실제로 구성을 싣는지
    - 키가 없으면 시작하지 않는지
    - 구성이 완전한 bundle은 시작하는지

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `xcodebuild -resolvePackageDependencies ... -packageAuthorizationProvider netrc` | Sparkle 2.10.1 해석, 종료 0 |
| `scripts/macos/smoke.sh` | `BUILD SUCCEEDED`(Swift warning/error 0), 기존 검사 OK, `update ok: ...`, `smoke OK`, 종료 0 |
| `scripts/macos/release_build.sh`(`sign_release.sh` 안에서 실행) | Release `BUILD SUCCEEDED`, `✓ 업데이트 구성: stable/beta appcast 분리, 서명 appcast 요구, 자동 설치 금지, EdDSA 공개 키 없음(...)`, 번들 검사 ✓, 종료 0 |
| `scripts/macos/sign_release.sh`(Developer ID) | 서명본 재배치 실행 `--update-check` 통과, 종료 0 |
| `cargo xtask validate-dependencies` | 종료 0 |
| `git diff --check` | 통과 |

| 검사(`--update-check`) | 결과 |
|---|---|
| 번들 Info.plist: Stable/Beta appcast https, 자동 확인, 86,400초 주기, 키 없음 → `disabled(missingPublicKey)` | 통과 |
| 구성 규칙: URL 누락·빈 값·http·file·host 없음·같은 URL·서명 요구 꺼짐/누락·자동 설치 허용/누락·키 빈 값/형식 오류/31바이트 거부 | 통과 |
| `AppUpdateController`: https 전용 기본값은 http fixture bundle 거부, 구성이 완전한 bundle은 `started(stable)`, 확인 전 요청 0 | 통과 |
| **channel isolation fixture**: Stable updater 요청은 `/stable.xml` 하나, 제안 build 3(최소 OS 99 build 4와 beta 채널 build 6 제외) | 통과 |
| **channel isolation fixture**: Beta updater 요청은 `/beta.xml` 하나, 제안 build 5 | 통과 |
| **channel isolation fixture**: Stable URL에 Beta appcast가 배포됨 → beta build 5 제외, build 3만 | 통과 |
| 서명 뒤 다운로드 URL을 바꾼 appcast·서명 block 없는 appcast → 제안 없음, 주기 오류 | 통과 |
| 승인 gate(background 확인): 거절 → 다운로드 0회(요청은 appcast뿐), 승인 → `willDownloadUpdate` 정확히 1회, 설치 단계 0회 | 통과 |
| 검사 민감도(임시 변이 후 원복): Stable에 `beta` 허용 → `stable 제안 ... ["6"]`로 실패 | 확인 |
| 검사 민감도: Beta가 Stable URL 요청 → `beta 요청: ["/stable.xml"]`로 실패 | 확인 |
| 검사 민감도: 거절을 무시하고 설치 → `거절했는데 다운로드: ["3"]`로 실패 | 확인 |
| 검사 민감도: host가 서명 appcast를 요구하지 않음 → `변조 appcast에서 업데이트 제안: ["3"]`로 실패 | 확인 |

## 차이(작업 지시와 다른 점)

- **선행**: 엄격 선행 XVI.9.2가 `[~]`입니다. 첫 구현은 그 상태에서 코드를 수정해 계획 §1.2 규칙 4를 어겼습니다
  (리뷰 지적). 이번 변경은 새 WBS 작업이 아니라, 이미 커밋된 코드의 리뷰 지적을 고친 `Fix:` 커밋입니다
  (작업 지시: "리뷰 지적부터 수정"). 행은 `[~]`로 둡니다.
- **포맷**: feed는 Sparkle appcast(RSS + Sparkle namespace) 표준 포맷입니다. 첫 구현의 자체 JSON schema 1은
  배포된 적이 없어 제거했습니다.
- **feed 위치**: 기본값은 공개 릴리스 저장소 placeholder(`stellar-releases`)이며 저장소는 만들지 않았습니다.
- **키 생성**: Sparkle EdDSA 키 생성은 사용자 단계로 두었습니다(ADR-0017 Decision 8).
- **fixture 서명**: RFC 8032 §7.1 TEST 1 test vector 키로 저장소 밖에서 만들었습니다. 앱에는 공개 키와 고정 서명만
  들어갑니다.
- **파일 수**: 한 커밋이 프로덕션 파일 5개를 넘습니다(계획 §1.2 규칙 13). Sparkle 내장이 번들·서명 스크립트를 함께
  바꾸지 않으면 Release 검사가 실패해, 하나의 `Fix:` 커밋으로 묶었습니다.

## 남은 위험 / 한계

- 사용자 단계 전에는 배포본이 업데이트를 확인하지 않습니다. 남은 단계는 키 생성, 공개 키 설정, 릴리스 저장소 생성,
  appcast·archive 게시입니다. `sign_release.sh`가 이 상태를 경고합니다.
- 실제 Sparkle 표준 UI 클릭, 실제 archive 다운로드·압축 해제·설치·재실행, `NSBackgroundActivityScheduler` 대신 쓰는
  Sparkle 예약 타이머의 실제 실행 시점은 사람이 확인하지 않았습니다.
- 업데이트 전 열린 프로젝트 저장·복구와 강제 종료 금지(XVI.9.6)는 XIII.1.6 선행 때문에 다루지 않았습니다.
- appcast 서명 실패가 20일 넘게 이어지면 Sparkle이 key 교체 복구용 제한 모드로 들어갑니다. 이때도 archive 서명
  검증은 유지됩니다(ADR-0017 Decision 6). 이 복구 경로를 끌지는 사용자가 결정해야 합니다.
- `validate-dependencies`는 `platform_dependencies`를 검사하지 않습니다(도구 후속).
- Zlib 구성요소 라이선스는 별도 승인이 필요합니다.
- 버전 번호 정책(`CFBundleVersion` 증가 규칙)은 정하지 않았습니다.

## 상태 이력

- 2026-11-14 첫 구현(`afedb93`): `[x]`.
- 통합 리뷰: `[~]`로 정정. 사유는 엄격 선행 XVI.9.2 `[~]`와 feed 미설정으로 실제 확인이 일어나지 않던 것입니다.
- 2026-11-14 리뷰 수정(`5b993ac`, Sparkle): feed 미전달 문제는 해소했습니다. 엄격 선행 미충족으로 `[~]`를
  유지합니다.
