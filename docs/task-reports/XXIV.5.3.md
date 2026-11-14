# 작업 보고서: XXIV.5.3 명령줄 설치 위치 결정

- WBS: XXIV.5.3
- 작업명: 명령줄 설치 위치 결정을 구현하고, 위치와 근거를 §37 보안 경계(최소 권한)와 Apple·각 셸 공식 문서로 정해 기록합니다. 계산은 UI·Document 스레드 밖에서 합니다.
- 상태: 미완료 (`[ ]` 유지). 엄격 선행 XXIV.5.1(`stellar` helper 번들 배치)이 `[ ]`이므로 완료로 올리지 않았습니다. 위치 결정 구현과 `DEV-MAC` fixture 검증은 시작 창 검토 수정 커밋(`a481400`)에서 끝났습니다.
- 선행: XXIV.5.1 (`[ ]`, 엄격)

## 필수 실행 환경

- `DEV-MAC`: Apple silicon(arm64), macOS 27.2, Xcode 27.2, Debug 빌드.

## 변경 파일 목록

- `apps/macos/StellarMac/UI/Welcome/StellarCommandInstaller.swift`: 후보 폴더, 위치 선택, PATH 구성(`StellarCommandSearchPath`), 메인 스레드 감시용 `StellarCommandFileSystemThreadAudit`
- `apps/macos/StellarMac/UI/Welcome/WelcomeViewController.swift`: `StellarCommandStatusCheck.evaluate`(큐에서 설치기 생성·판정)
- `apps/macos/StellarMac/UI/Welcome/WelcomeUICheck.swift`: fixture L1~L8, PATH 구성, 메인 스레드 감시 검사
- `docs/design/ui/welcome.md` 6.2·6.3절(정의 원본)
- `docs/task-reports/XXIV.5.3.md`(이 보고서). 계획서 상태 표기는 바꾸지 않았습니다.

## 결정: 설치 위치

정의 원본은 `docs/design/ui/welcome.md` 6.3절입니다. 요약하면 다음과 같습니다.

| 항목 | 결정 | 근거 |
|---|---|---|
| 후보 | `~/.local/bin`, `~/bin`(이 순서). 홈 아래만 씁니다. | 아키텍처 §37.3 "최소 권한", §37.1 "프로젝트 지정 실행 파일 제한"의 취지(권한 상승 없음, 다른 사용자 영향 없음). `~/.local/bin`은 XDG Base Directory 명세의 사용자 실행 파일 위치(보조 근거) |
| 제외 | `/usr/local/bin` 등 여러 사용자가 함께 쓰는 폴더는 쓸 수 있어도 고르지 않습니다. 관리자 인증, 권한 있는 helper를 쓰지 않습니다. | §37 최소 권한, ADR-0005 §7 |
| 선택 순서 | ① 이미 있는 Stellar 소유 링크의 폴더 ② 확인한 PATH에 있고 쓸 수 있는(없으면 만들 수 있는) 첫 후보 ③ 쓸 수 있는(만들 수 있는) 첫 후보와 PATH 추가 안내 ④ 없으면 `noWritableLocation(~/.local/bin)` | — |
| 확인한 PATH | 순서를 아는 부분: `/etc/paths`, `/etc/paths.d/*`, 앱 프로세스 `PATH`. 순서를 모르는 부분: 홈 후보, `/opt/homebrew/bin`, `/opt/local/bin` | path_helper(8): `/etc/paths` 기본값에 `/etc/paths.d` 내용을 덧붙여 PATH 구성. macOS `/etc/zprofile`: login zsh에서 `path_helper -s` 실행. Apple Terminal 사용 설명서 "Change General settings in Terminal on Mac": 새 창은 기본 login shell(zsh)로 엶 |
| 셸 설정 파일 | 읽지도 실행하지도 않습니다. 확인하지 못한 폴더는 "확인하지 못함"으로 표시하고 `export PATH="…:$PATH"` 한 줄을 복사하게 합니다. | zsh(1) STARTUP/SHUTDOWN FILES: `.zprofile`·`.zshrc`는 셸이 읽어 실행하는 명령 파일이므로 결과 PATH를 알려면 사용자 코드를 실행해야 함(§37 최소 권한) |
| 앱 위치 | App Translocation 경로 또는 읽기 전용 볼륨(`URLResourceKey.volumeIsReadOnlyKey`)이면 설치하지 않습니다. | Apple Code Signing Guide(보관 문서)의 Gatekeeper path randomization. 경로 문자열 판정은 지원 API가 아니어서 읽기 전용 볼륨 판정을 함께 씀 |
| helper 경로 | `Contents/Helpers/stellar` | ADR-0005 §2(실행 이름 `stellar`)·§7(helper로 번들에 넣음), Apple "Placing content in a bundle"(helper tool → `Contents/Helpers/`). 최종 배치는 XXIV.5.1 보고서가 정함 |
| 스레드 | 설치기 생성(`/etc/paths` 읽기)과 판정은 `StellarCommandStatusCheck.evaluate(on:)`의 큐와 sheet 작업 큐에서 하고, 결과만 메인 스레드로 넘깁니다. | 아키텍처 §6.3(UI 스레드 단일 점유 2ms), §6.4, HIG Launching("Launch fast") |

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `xcodebuild -project apps/macos/StellarMac.xcodeproj -scheme StellarMac -configuration Debug -derivedDataPath apps/macos/build -packageAuthorizationProvider netrc -disableAutomaticPackageResolution CODE_SIGNING_ALLOWED=NO build` | `BUILD SUCCEEDED`, Welcome 파일 경고 0 |
| `StellarMac --welcome-ui-check`(임시 실행 인자, 커밋하지 않은 `main.swift` 분기) 3회 | 3회 모두 `welcome-ui ok: 353 checks [disk installer ok; main-thread fs calls 0, background 103; …; running app command status=helperUnavailable]`, 종료 코드 0 |
| `./scripts/macos/smoke.sh` | `smoke OK`, 종료 코드 0(시작 창 검사는 smoke에 연결돼 있지 않음) |
| `git diff --check` | 출력 없음 |

| fixture(보고서 정의와 일치 여부) | 결과 |
|---|---|
| L1 PATH에 홈 후보 없음 → `~/.local/bin/stellar`, 폴더 생성, PATH 안내 | 통과 |
| L2 PATH에 있는 `~/.local/bin` → 생성 없음, PATH 확인됨 | 통과 |
| L3 PATH에 있는 `~/bin`을 PATH에 없는 첫 후보보다 먼저 선택 | 통과 |
| L4 쓸 수 있고 PATH에 있는 `/usr/local/bin`이 있어도 홈 후보 선택 | 통과 |
| L5 홈에 쓸 수 없음 → `noWritableLocation(~/.local/bin)` | 통과 |
| L6 쓸 수 없는 `~/.local/bin` 건너뛰고 `~/bin` 생성 | 통과 |
| L7 App Translocation·읽기 전용 볼륨 → `unsupportedAppLocation` | 통과 |
| L8 helper 없음 → `helperUnavailable(…/Contents/Helpers/stellar)` | 통과 |
| PATH 구성(`/etc/paths` → `/etc/paths.d` → 프로세스 `PATH`, 순서 모르는 폴더에서 중복 제거) | 통과 |
| UI 스레드 호출 0: 상태 확인, 첫 실행 판정, sheet 확인·설치·제거·Check Again 경로에서 메인 스레드 파일 조회 0(배경 호출 103) | 통과 |
| 실제 파일 시스템(임시 폴더): 판정만으로 폴더를 만들지 않음, 설치 위치 일치 | 통과 |

변이 확인(커밋하지 않고 되돌림): helper 경로를 `Contents/Helpers/stellar-cli`로 바꾸면 L1~L8 등 32건 실패. 상태 판정을 메인 스레드로 옮기면 `thread: 메인 스레드 파일 조회 ["isExecutableFile", "isExecutableFile"]`으로 실패.

## 차이(작업 지시와 다른 점)

- 구현은 이 행의 WBS 커밋이 아니라 시작 창 검토 수정 커밋(`Fix:`)에 들어 있습니다. 이 코드는 XXIV.5 행이 생기기 전에 시작 창 영역에 먼저 들어왔고, 검토 지적에 따라 ADR-0005 기준으로 고쳤습니다.
- 설치기는 시작 창 영역(`apps/macos/StellarMac/UI/Welcome/`)에 있습니다. 이 작업의 허용 경로가 그 영역뿐이라 옮기지 않았습니다.
- "Document 스레드 밖" 조건: 설치기는 코어·Document 스레드를 쓰지 않습니다. 셸 전용 큐에서만 실행합니다.

## 남은 위험 / 한계

- 셸 설정 파일이 더하는 PATH는 알 수 없습니다. 순서 모르는 폴더 목록 밖의 폴더는 판정에 들어가지 않습니다.
- App Translocation 경로 판정은 Apple이 지원하는 방법이 아닙니다.
- 검사는 임시 실행 인자로 실행했습니다. `main.swift`·`scripts/macos/smoke.sh` 연결은 이 영역 밖이라 하지 않았습니다.

## 재개 조건

- XXIV.5.1이 `[x]`가 되어 번들에 helper가 실제로 들어갑니다.
- helper 배치가 `Contents/Helpers/stellar`와 다르면 `helperRelativePath`를 고칩니다.
- `--welcome-ui-check`를 smoke에 연결한 뒤 같은 fixture가 통과하면 `Task(XXIV.5.3)`로 표기를 올립니다.
