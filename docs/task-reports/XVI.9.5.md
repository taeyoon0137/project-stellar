# 작업 보고서: XVI.9.5 Stable/Beta 업데이트 feed 분리와 background 확인

- WBS: XVI.9.5
- 작업명: Stable/Beta update feed 분리, 앱은 background check만 수행, 다운로드·설치는 사용자 승인 후
- 상태: 완료 (`[x]`)
- 선행: XVI.9.2 (`[~]`, 이 lane — 필요한 macOS 부분인 Release 번들 구성은 존재)

## 필수 실행 환경

- `DEV-MAC` (로컬 Xcode Debug·Release 빌드, headless `--update-check`). 실제 feed 서버·네트워크는 쓰지 않았습니다.

## 변경 파일 목록

- `apps/macos/StellarMac/UpdateChecker.swift`(신규) — `UpdateChannel`, `UpdateFeed`(JSON schema 1 해석·검증),
  `UpdateChecker`(background 확인·승인 gate), `UpdateScheduler`(`NSBackgroundActivityScheduler`), `UpdateConfiguration`,
  기본 loader(ephemeral `URLSession`, https 전용)·승인자(native alert)·downloader(기본 브라우저).
- `apps/macos/StellarMac/AppDelegate.swift` — feed가 설정된 경우에만 scheduler 시작, 종료 시 중지.
- `apps/macos/StellarMac/UpdateCheck.swift`(신규), `apps/macos/StellarMac/main.swift` — `--update-check` headless fixture.
- `scripts/macos/smoke.sh` — `--update-check` 실행 추가.
- `docs/task-reports/XVI.9.5.md`(신규), 계획서(XVI.9.5 `[ ]`→`[x]`).

## 구현 요약

- **채널 분리**: Stable·Beta는 서로 다른 feed URL(Info.plist `StellarUpdateFeedStable`/`StellarUpdateFeedBeta`, https만)을
  쓰고, checker는 자기 채널 URL 하나만 요청합니다. feed 문서는 자기 `channel`을 선언하며, 요청한 채널과 다르면
  `channelMismatch`로 거부합니다(잘못 배포된 feed가 다른 채널로 새지 않음). 사용자 채널은 `UserDefaults`
  `StellarUpdateChannel`(`beta`만 Beta, 그 밖은 Stable)입니다.
- **feed(JSON schema 1)**: `{schema, channel, items: [{version, build, minimumSystemVersion, downloadURL}]}`. https가 아닌
  다운로드 URL, 잘못된 OS 버전 문자열, 0 이하 build 항목은 버립니다. 현재 `CFBundleVersion`보다 크고 현재 macOS가 최소
  OS 이상인 항목 중 가장 높은 build를 고릅니다(아키텍처 §40.4 최소 OS 호환성 검사).
- **background check만**: `NSBackgroundActivityScheduler`(채널별 identifier, 하루 1회 반복, 허용 오차 1시간, utility QoS)가
  전용 utility queue에서 feed만 읽습니다. 자동 다운로드·자동 설치·앱 강제 종료는 없습니다.
- **승인 gate**: 새 항목이 있으면 native alert(다운로드/나중에)로 묻고, 승인한 경우에만 다운로드를 시작합니다.
  다운로드는 기본 브라우저로 공증된 배포본을 여는 방식이며, 설치는 사용자가 직접 합니다(외부 update framework 미사용).
- feed URL이 설정되지 않은 빌드(현재 저장소 상태)는 scheduler를 만들지 않아 네트워크 요청이 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `scripts/macos/smoke.sh` | `BUILD SUCCEEDED`(Swift warning/error 0), 기존 검사 OK, `update ok: ...`, `smoke OK`, 종료 0 |
| `scripts/macos/release_build.sh` | Release `BUILD SUCCEEDED`(Swift warning/error 0), 구성·bundle 검사 ✓, 종료 0 |
| `git diff --check` | 통과 |

| 검사(`--update-check`) | 결과 |
|---|---|
| **channel isolation fixture**: Stable checker는 Stable URL만 요청하고 Stable build 3을 제안(Beta build 5를 보지 않음), Beta checker는 Beta URL만 요청하고 build 5를 제안 | 통과 |
| 채널 격리 위반: Stable URL이 `channel: beta` 문서를 내면 `channelMismatch`, 승인 요청·다운로드 0회 | 통과 |
| 승인 gate: 거절 시 다운로드 0회(`declined`), 승인 시 같은 항목 다운로드 정확히 1회(`approved`) | 통과 |
| background: feed loader 호출이 main thread에서 0회 | 통과 |
| 최소 OS 99.0 항목·현재 build 이하(upToDate)·http 다운로드·버전 형식 오류·build 0 제외, schema 2·비 JSON 거부, http feed URL은 요청 없이 거부 | 통과 |
| 채널 선호값(기본·beta·알 수 없는 값), scheduler 구성(identifier·반복·86,400초·utility), feed 미설정 앱은 scheduler 없음 | 통과 |
| 검사 민감도: feed 채널 검사를 임시로 끄면 "채널 불일치 결과: approved(...)", 승인 여부를 무시하면 "stable 거절 결과: approved(...)"로 실패(종료 1) 확인 후 원복 | 확인 |

## 차이(작업 지시와 다른 점)

- 외부 update framework(예: 서명 검증·자동 설치를 제공하는 framework)는 `config/dependencies.yaml` 승인 대상이 아니어서
  쓰지 않았습니다. 대신 의존성 없는 feed 확인과 승인 gate를 구현했고, 승인 후 "다운로드·설치"는 기본 브라우저로 공증된
  배포본을 내려받아 사용자가 설치하는 방식입니다. 앱 안 자동 설치가 필요하면 framework 승인 또는 자체 설치기 설계가 필요합니다.
- feed 위치(호스팅 URL)는 배포 결정이 없어 비워 두었습니다. 설정 전에는 업데이트 확인이 동작하지 않습니다.
- feed JSON schema 1은 이 작업에서 정한 최소 형식입니다(아키텍처 문서에 형식 정의 없음).

## 남은 위험 / 한계

- feed 자체 서명(예: Ed25519)은 없습니다. 보호는 https와, 내려받은 배포본에 대한 Gatekeeper(Developer ID·공증) 검사에
  의존합니다. 앱 안 자동 설치를 도입하면 feed·payload 서명 검증이 필수입니다.
- 실제 alert 클릭·브라우저 다운로드·`NSBackgroundActivityScheduler`의 실제 실행 시점은 사람이 확인하지 않았습니다(구성만 검사).
- 업데이트 전 열린 프로젝트 저장·복구와 강제 종료 금지(XVI.9.6)는 XIII.1.6 선행 때문에 다루지 않았습니다.
- 버전 번호 정책(`CFBundleVersion` 증가 규칙)은 정하지 않았습니다.
