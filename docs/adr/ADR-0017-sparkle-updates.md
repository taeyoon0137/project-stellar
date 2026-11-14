# ADR-0017: macOS 앱 업데이트를 Sparkle 2로 구현

## Status

Proposed (2026-11-14).

- Sparkle 채택은 제품 책임자가 2026-11-14에 결정했습니다(workflow 작업 지시로 전달됨).
- 아래 세부 구성은 구현 lane이 제안한 것이며 사용자 확인이 필요합니다: feed 위치 placeholder, 서명 appcast 요구,
  의존성 기록 방식(`platform_dependencies`), ad-hoc 검사 사본 예외, test vector 키 사용.

## Context

기술 아키텍처 §40.3은 Stable·Beta 채널을 정의합니다. §40.4는 업데이트 규칙을 정합니다: background 확인,
사용자 승인 후 다운로드·설치, 실행 중 프로젝트 강제 종료 금지, 최소 OS 호환성 검사. 구현 계획 XVI.9.5는
"Stable/Beta update feed 분리, 앱은 background check만, 다운로드·설치는 사용자 승인 후"를 요구하고, XVI.9.6은
업데이트 전 열린 프로젝트의 저장·복구를 요구합니다.

XVI.9.5의 첫 구현은 자체 JSON feed(schema 1)와 기본 브라우저 다운로드를 썼습니다. 통합 리뷰에서 다음 문제가
나왔습니다.

1. **feed가 동작하지 않음.** 타깃이 `GENERATE_INFOPLIST_FILE = YES`만 썼습니다. `INFOPLIST_KEY_*` build setting으로는
   사용자 정의 key를 넣을 수 없어 feed URL을 설정할 방법이 없었습니다. 테스트도 "미설정이면 scheduler 없음"을 통과
   조건으로 고정했습니다.
2. **지시 없는 새 파일 포맷.** 작업 지시 없이 feed JSON schema를 새로 정의했습니다(계획 §1.2 규칙 3 위반).
3. **무결성을 TLS에만 의존.** feed 서명, 다운로드 host 제한, 응답 크기 제한이 없었습니다. feed host가 탈취되면
   승인된 다운로드를 임의의 https URL로 바꿀 수 있었습니다.

제품 책임자 결정(2026-11-14)은 다음과 같습니다.

- 자동 업데이트를 Sparkle로 구현합니다.
- feed URL은 설정할 수 있어야 하고, 기본값은 별도 공개 릴리스 저장소 placeholder입니다.
- 에이전트는 저장소를 만들거나 배포하지 않습니다. 공개 릴리스 저장소는 실제 배포 시점에 만듭니다.
- Sparkle EdDSA 키는 제품 책임자 요청(2026-11-14)으로 개발 Mac에서 생성했습니다. 개인 키는 그 Mac의 로그인
  키체인에만 있고, 저장소에는 공개 키만 둡니다(아래 8).

## Decision

1. **의존성.** Sparkle 2.10.1을 Swift Package Manager로 Xcode 프로젝트에 추가합니다.
   - 버전 요구는 `exactVersion`이고, `project.xcworkspace/xcshareddata/swiftpm/Package.resolved`를 커밋합니다
     (revision `ab7d71291238df6e0492143b120c9b696a35f603`).
   - binary xcframework는 Sparkle `Package.swift`가 checksum(`4c2e48a3…022a`)으로 고정합니다.
   - 스크립트는 `-disableAutomaticPackageResolution`(resolved 버전만 사용)과 `-packageAuthorizationProvider netrc`로
     빌드합니다. Sparkle은 공개 package라 인증이 필요 없습니다. netrc를 쓰면 키체인 조회와 그에 따른 접근 대화상자를
     피할 수 있습니다.
   - 라이선스는 MIT입니다. 포함 구성요소는 bsdiff·SUSignatureVerifier(BSD-2-Clause), sais-lite(MIT),
     orlp/ed25519(Zlib)입니다.
   - `config/dependencies.yaml`의 `platform_dependencies`에 기록합니다. `cargo xtask validate-dependencies`는
     `dependencies`만 xtask cargo 직접 의존성과 대조하므로, cargo가 아닌 항목을 그 목록에 넣으면 `unused-approval`로
     실패합니다.
   - Zlib은 자동 허용 목록에 없어 `separate_approval: true`로 표시했습니다. 확인이 필요합니다.
2. **Info.plist.** `apps/macos/StellarMac/Info.plist`(부분 plist)를 `INFOPLIST_FILE`로 지정하고, Xcode가 생성하는
   key와 병합합니다(`GENERATE_INFOPLIST_FILE = YES` 유지). synchronized group의 resource에서는 제외합니다. key는 다음과
   같습니다.

   | key | 값 |
   |---|---|
   | `SUFeedURL` | `$(STELLAR_UPDATE_FEED_STABLE)` |
   | `StellarUpdateFeedBeta` | `$(STELLAR_UPDATE_FEED_BETA)` |
   | `SUPublicEDKey` | `$(STELLAR_UPDATE_PUBLIC_ED_KEY)`(기본 빈 값) |
   | `SURequireSignedFeed`, `SUVerifyUpdateBeforeExtraction` | `YES` |
   | `SUEnableAutomaticChecks` | `YES`(첫 실행 권한 질문 없이 background 확인) |
   | `SUScheduledCheckInterval` | `86400` |
   | `SUAllowsAutomaticUpdates`, `SUAutomaticallyUpdate` | `NO` |
   | `SUEnableSystemProfiling` | `NO` |

3. **feed 위치(기본값은 placeholder).**
   - Stable: `https://raw.githubusercontent.com/taeyoon0137/stellar-releases/main/appcast/stable.xml`
   - Beta: `…/appcast/beta.xml`

   `stellar-releases` 저장소는 아직 없으며 만들지 않았습니다. 다른 위치를 쓰려면 project build setting을 바꾸거나
   스크립트 환경 변수(`STELLAR_UPDATE_FEED_STABLE`, `STELLAR_UPDATE_FEED_BETA`)를 씁니다.
4. **채널 분리.** 두 가지를 겹쳐 씁니다.
   - 채널마다 appcast를 따로 둡니다. delegate `feedURLString(for:)`가 선호 채널의 URL 하나만 돌려줍니다.
   - Sparkle channel 필터를 함께 적용합니다(`allowedChannels`: Stable = 빈 집합, Beta = `{"beta"}`).

   Beta 전용 항목에는 `<sparkle:channel>beta</sparkle:channel>`을 붙입니다. 이렇게 하면 beta 항목이 Stable appcast에
   잘못 올라가도 Stable 사용자에게는 제안되지 않습니다. 사용자 채널은 `UserDefaults` `StellarUpdateChannel`이며
   `beta`일 때만 Beta입니다. delegate가 확인할 때마다 이 값을 읽으므로, 채널을 바꾸면 다음 확인부터 적용됩니다.
5. **승인.** Sparkle 표준 UI(`SPUStandardUserDriver`)를 씁니다.
   - 예약 확인(background)에서 새 버전을 찾으면 알립니다.
   - 사용자가 설치를 고른 뒤에만 다운로드하고, 설치·재실행도 사용자가 고른 뒤에만 합니다.
   - 자동 다운로드·설치는 Info.plist에서 꺼서 사용자가 켤 수 있는 선택지도 없습니다.
6. **무결성.** EdDSA(Ed25519)로 검증합니다.
   - appcast 끝에 `generate_appcast`/`sign_update`가 붙이는 서명 block을 요구합니다.
   - archive enclosure의 `sparkle:edSignature`는 압축을 풀기 전에 검증합니다.
   - 배포본 자체도 Developer ID 서명과 공증을 거칩니다(XVI.9.3·XVI.9.4).
   - Sparkle 기본값으로, appcast 서명 실패가 20일 동안 이어지면 key 교체 복구를 위해 제한 모드로 appcast를 처리합니다.
     제한 모드에서는 release notes·critical 표시·package 설치처럼 신뢰할 수 없는 값을 버립니다. archive 서명 검증은
     그대로 적용됩니다. 이 복구 경로를 끄려면 `SUSignedFeedFailureExpirationInterval = 0`을 설정하면 됩니다(사용자 결정).
7. **활성화 조건.** 앱(`AppUpdateController`)은 구성이 완전할 때만 Sparkle updater를 시작합니다. 완전한 구성의
   조건은 다음과 같습니다.
   - 두 appcast가 https이고 서로 다름
   - 서명 appcast 요구와 추출 전 검증이 켜져 있음
   - 자동 설치가 명시적으로 꺼져 있음
   - 공개 키가 base64 32바이트

   하나라도 어긋나면 updater를 만들지 않으므로 네트워크 요청도 없습니다. 공개 키가 설정된 뒤(2026-11-14)로는
   구성이 완전하므로 updater가 시작됩니다. 공개 릴리스 저장소가 생기기 전에는 background 확인이 feed를 찾지 못하고
   조용히 실패합니다(사용자에게 대화상자를 띄우지 않음).
8. **배포 키와 남은 사용자 단계.**
   1. 키(완료, 2026-11-14): `generate_keys --account stellar`로 만들었습니다. keychain account 이름 `stellar`는 같은
      Mac에서 Sparkle을 쓰는 다른 앱의 기본 키(`ed25519`)와 섞이지 않게 하려고 정했습니다. 공개 키
      `bNzmhRu95zcg9IcwH2rWI5FzHKrxY4pVe2+24VYO4Po=`를 project build setting `STELLAR_UPDATE_PUBLIC_ED_KEY`(Debug·Release)에
      넣었습니다. 개인 키는 개발 Mac의 로그인 키체인에만 있습니다. 다른 Mac으로 옮기거나 백업할 때만
      `generate_keys --account stellar -x <파일>`로 저장소 밖에 내보내고, 절대 커밋하지 않습니다. 개인 키를 잃으면
      Sparkle key rotation 절차로 새 키를 배포해야 합니다(아래 Migration).
   2. 공개 릴리스 저장소 `stellar-releases`를 만듭니다(또는 다른 feed 위치를 정합니다). 실제 배포 시점의 사용자 단계입니다.
   3. 릴리스마다 다음을 수행합니다.
      1. `scripts/macos/sign_release.sh`와 `scripts/macos/notarize.sh`를 실행합니다.
      2. `ditto -c -k --keepParent`로 archive를 만듭니다.
      3. 채널별 폴더에서 `generate_appcast --account stellar`를 실행합니다. Beta 항목에는 `--channel beta`를 붙입니다.
      4. appcast와 archive를 올립니다.
9. **번들·서명.**
   - Sparkle.framework는 `Contents/Frameworks`에 내장하고, `LD_RUNPATH_SEARCH_PATHS`는
     `@executable_path/../Frameworks`입니다.
   - 서명은 안쪽부터 합니다(`scripts/macos/codesign_bundle.sh`): XPC services → Autoupdate → Updater.app →
     framework → 앱. 모두 Hardened Runtime으로 서명합니다. Downloader.xpc만 entitlements를 보존하고, `--deep` 서명은
     쓰지 않습니다.
   - `bundle_check.sh`는 framework 하위만 허용하고, Mach-O 목록을 고정하며, 모든 중첩 코드가 같은 TeamIdentifier인지
     확인합니다.
   - ad-hoc 검사 사본은 Team ID가 없어 library validation을 통과하지 못합니다. 그래서 임시 사본의 앱 실행 파일에만
     `com.apple.security.cs.disable-library-validation`을 붙입니다. 이미 서명된 앱(배포본)에 이 entitlement가 있으면
     실패로 처리합니다.
   - Sparkle binary는 universal(x86_64+arm64)로 둡니다. thinning하지 않습니다. 실행 파일 크기가 바뀌면 Sparkle이
     delta update를 건너뛰기 때문입니다.
10. **검증 fixture.** `--update-check`는 실제 Sparkle updater를 127.0.0.1 loopback HTTP fixture 서버에 연결해 다음을
    확인합니다.
    - 채널별 실제 요청 경로
    - 제안 항목
    - 변조·무서명 appcast 거부
    - 거절 시 다운로드 0회, 승인 시 1회

    fixture appcast는 RFC 8032 §7.1 TEST 1 키로 저장소 밖에서 미리 서명했습니다. 이 키는 개인 키까지 공개된 test
    vector입니다. 앱과 저장소에는 공개 키와 고정 서명만 둡니다. `sign_release.sh`는 이 공개 키를 넣은 배포 서명을
    거부합니다.

## Alternatives

- **자체 JSON feed + 브라우저 다운로드 유지:** 앱 안 설치가 없습니다. 자체 포맷(리뷰 지적 2)이고, 무결성은 TLS에만
  의존합니다(리뷰 지적 3). 탈락했습니다.
- **자체 설치기:** 실행 중인 번들 교체와 권한 상승 처리를 새로 만들어야 하고, 보안 표면이 큽니다. 검증된 framework를
  대신할 이유가 없습니다.
- **Mac App Store:** 아키텍처 §40.1에서 초기 범위에서 제외했습니다.
- **단일 appcast + channel tag만:** Sparkle이 권장하는 방식이지만, 행 지시가 feed 분리를 요구합니다. 그래서 분리한 appcast에
  channel tag를 겹쳐 씁니다.
- **framework 수동 내장·Carthage:** SPM이 Sparkle이 지원하는 경로이고 checksum으로 고정됩니다.

## Consequences

- 장점: 표준 appcast 포맷, EdDSA 서명 검증, 사용자 승인 기반 앱 안 설치, 하루 1회 background 확인을 얻습니다.
- 앱 번들에 외부 binary framework(약 2.7 MB)와 중첩 코드 5개가 추가됩니다. 서명·공증 대상도 늘어납니다.
- Sparkle의 x86_64 slice는 쓰이지 않습니다.
- `validate-dependencies`는 `platform_dependencies`의 라이선스·사용 여부를 검사하지 않습니다. 도구 담당 작업에서 검사를
  넓혀야 합니다.
- XVI.9.6(업데이트 전 dirty project 저장·복구)은 Sparkle의 설치·재실행 hook으로 구현해야 합니다. 예:
  `-updater:shouldPostponeRelaunchForUpdate:untilInvokingBlock:`, `-updaterShouldRelaunchApplication:`. XIII.1.6
  이후에 진행합니다.
- 키가 비어 있는 빌드는 업데이트를 확인하지 않습니다. `sign_release.sh`가 이 상태를 경고합니다. 현재 저장소는 키가
  설정되어 있습니다.
- 개인 키는 개발 Mac 한 대의 키체인에만 있습니다. 그 Mac을 잃으면 기존 배포본에 업데이트를 보낼 수 없으므로, 배포를
  시작하기 전에 저장소 밖 안전한 곳에 백업해야 합니다(사용자 단계).

## Compatibility

- Stellar의 파일 포맷, 공개 API, FFI는 바뀌지 않습니다.
- appcast는 Sparkle 포맷(RSS + Sparkle namespace)이며 Stellar 포맷이 아닙니다. 첫 구현의 자체 JSON feed schema 1은
  배포된 적이 없어 제거했습니다.
- `UserDefaults` `StellarUpdateChannel` 의미는 유지됩니다.
- Sparkle 2.10은 macOS 12 이상을 요구하고, 앱은 macOS 26 이상을 요구합니다.

## Performance

- 확인은 하루 1회 background에서 합니다. 인터랙션 성능 예산 항목에는 영향이 없습니다.
- 앱 시작 시 framework load 비용이 늘지만 측정하지 않았습니다. 해당 예산 항목도 없습니다.

## Security

- appcast와 archive의 EdDSA 서명을 모두 요구하고, 압축 해제 전에 검증합니다.
- feed는 https만 허용합니다(구성 검사).
- 자동 다운로드·설치를 금지합니다.
- 모든 중첩 코드를 Developer ID, Hardened Runtime, secure timestamp로 서명하고 공증합니다. 배포본은 library
  validation을 유지합니다.
- feed host가 탈취되어도, 서명된 appcast 때문에 다운로드 위치나 버전을 바꿀 수 없습니다. 예외는 위 6의 20일 복구
  모드이며, 그때도 archive 서명 검증은 적용됩니다.
- 개인 키는 저장소에 두지 않습니다. 배포 서명은 test vector 키를 거부합니다.
- feed 응답 크기 제한은 Sparkle 동작에 맡겼습니다. 별도 상한은 두지 않았습니다.

## Migration

- 첫 구현의 JSON feed는 배포되지 않아 옮길 데이터가 없습니다.
- 업데이트 키를 교체할 때는 Sparkle 문서의 key rotation 절차를 따릅니다. 새 키로 서명한 archive를 Developer ID
  서명과 함께 배포합니다.
