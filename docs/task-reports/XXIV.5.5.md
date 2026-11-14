# 작업 보고서: XXIV.5.5 설치 상태 확인

- WBS: XXIV.5.5
- 작업명: 설치 상태 확인을 구현합니다. 확인 시점·방법과 결과 처리는 Apple·각 셸 공식 문서를 근거로 정해 기록하고, 처리는 XXIV.5.4와 같은 조건을 지키며, 확인은 UI 스레드 밖에서 합니다.
- 상태: 부분 완료 (`[~]`). 구현과 `DEV-MAC` fixture 검증은 시작 창 검토 수정 커밋(`a481400`)에서 끝났지만, 선행 XXIV.5.4가 `[x]`가 아니므로(엄격 선행 XXIV.5.1 대기) `[x]`로 올리지 않습니다(계획서 §1.2 규칙 4). 처음 보고서는 `[ ]`로 두었으나, 보고서 검증(완료 보고서는 `[x]`·`[~]`만 허용)과 선행이 `[x]`가 아닌 같은 처지의 행(XXIV.1.3, XXIV.4.3 등)의 표기에 맞춰 9B 통합에서 `[~]`로 정정했습니다.
- 선행: XXIV.5.4 (`[~]`, 엄격)

## 필수 실행 환경

- `DEV-MAC`: Apple silicon(arm64), macOS 27.2, Xcode 27.2, Debug 빌드.

## 변경 파일 목록

- `apps/macos/StellarMac/UI/Welcome/StellarCommandInstaller.swift`: `status()` 판정 순서, `shadowed`·`namesUnavailable` 상태
- `apps/macos/StellarMac/UI/Welcome/WelcomeViewController.swift`: `StellarCommandStatusCheck`, 첫 실행 확인, 관리 진입점 확인
- `apps/macos/StellarMac/UI/Welcome/WelcomeCommandInstallSheetController.swift`: 확인 단계(`checking`), `refreshStatus()`(Check Again)
- `apps/macos/StellarMac/UI/Welcome/WelcomeStyle.swift`: `WelcomeFirstLaunchPolicy`
- `apps/macos/StellarMac/UI/Welcome/WelcomeUICheck.swift`: 상태 fixture, 메인 스레드 감시
- `docs/design/ui/welcome.md` 6.5절(정의 원본)
- `docs/task-reports/XXIV.5.5.md`(이 보고서). 계획서 상태 표기는 바꾸지 않았습니다.

## 결정: 확인 시점·방법·처리

정의 원본은 `docs/design/ui/welcome.md` 6.5절입니다.

**확인 시점**

| 시점 | 내용 | 근거 |
|---|---|---|
| 첫 실행 | 시작 창이 뜬 뒤 메인 스레드 밖에서 한 번 확인합니다. 아직 답하지 않았고 제안 대상 상태면 sheet를 띄웁니다. | ADR-0005 §7, HIG Launching("Launch fast": 실행을 막지 않음) |
| 관리 진입점 | 앱 메뉴 "Set Up the stellar Command…"를 고를 때마다 확인합니다. | — |
| 실행 직전 | `install(_:)`·`uninstall()` 안에서 다시 확인합니다. | XXIV.5.4 조건 |
| Check Again | 충돌 뒤 사용자가 다시 확인을 고를 때 | — |
| 하지 않음 | 주기적 확인, 파일 시스템 감시, 앱 활성화 때 확인 | 자원 사용과 방해 최소화 |

**확인 방법**: 셸을 실행하지 않습니다. `lstat`으로 후보 폴더의 링크를 읽고, 링크 대상 번들의 Info.plist로 소유를 확인하고, 확인한 PATH(XXIV.5.3)로 같은 이름의 다른 프로그램을 찾습니다. 셸 명령 탐색은 PATH 순서의 첫 실행 파일입니다(zshmisc(1)·bash(1) COMMAND EXECUTION). 설치기 생성과 판정은 큐에서 하고 결과만 메인 스레드로 넘깁니다(아키텍처 §6.4).

**판정과 처리**(위에서부터 먼저 맞는 것, 모두 XXIV.5.4 조건: 동의 뒤에만 변경, 남의 파일 보존, 대체 이름 규칙)

| 상태 | 조건 | 처리 |
|---|---|---|
| `helperUnavailable` | 번들에 실행 가능한 `Contents/Helpers/stellar` 없음 | Close. 첫 실행 제안 없음 |
| `unsupportedAppLocation` | App Translocation 또는 읽기 전용 볼륨 | Close. 제안·기록 없음(옮긴 뒤 다시 제안) |
| `installed` | 이 앱 helper를 가리키는 소유 링크가 있고 그 이름이 가려지지 않음 | Done / Remove Command(이 앱 helper 링크만 지움) |
| `shadowed` | 이 앱을 가리키는 소유 링크가 모두 다른 프로그램에 가려짐 | 대체 이름 가능하면 Install as stellar-cli 제안, 아니면 알림. Remove Command 제공 |
| `needsRepair` | 다른 위치를 가리키는 소유 링크가 있고 이름이 가려지지 않음 | Update(원자적 교체). 계획에 지금 링크 대상(절대 경로)과 그 앱 번들이 있는지를 실어 동의 화면에 보여 줌 |
| `ready` | 새 설치 가능(`stellar` 또는 대체 이름) | Install / Install as stellar-cli |
| `namesUnavailable` | 두 이름 모두 다른 프로그램 | 설치 0, 알림(Close) |
| `noWritableLocation` | 쓸 수 있는 후보 없음 | 만들 폴더 안내, Close |

- 첫 실행 제안 대상: `ready`, `needsRepair`, `shadowed`, `namesUnavailable`, `noWritableLocation`. 답하면(메뉴로 연 sheet 포함) `StellarCommandInstallOffered`를 기록하고 다시 띄우지 않습니다.
- 대체 이름으로 설치한 뒤 `stellar`를 쓸 수 있게 되어도 이름을 자동으로 바꾸지 않습니다.
- 셸의 명령 위치 기억: zsh `HASH_CMDS`·`HASH_DIRS`(zshoptions(1))와 bash hash table(bash(1), `checkhash` 기본 꺼짐)은 한 번 찾은 위치를 기억하므로, 설치 완료 문구는 "새 Terminal 창에서" 실행하라고 안내합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| Debug `xcodebuild … build`(XXIV.5.3 보고서와 같은 옵션) | `BUILD SUCCEEDED` |
| `StellarMac --welcome-ui-check`(커밋된 실행 인자) 3회 | 3회 모두 `welcome-ui ok: 378 checks [disk installer ok; main-thread fs calls 0, background 104; …]`, 종료 코드 0 |
| `./scripts/macos/smoke.sh`(`--welcome-ui-check` 포함) | `welcome-ui ok: 378 checks …`, `smoke OK`, 종료 코드 0 |

| 상태 fixture(보고서 정의와 일치) | 결과 |
|---|---|
| `helperUnavailable`(L8), `unsupportedAppLocation`(L7) | 통과 |
| `installed`: 새 설치 후(I1), 자기 링크만 있음(N7), 대체 이름 설치 후(N1, N9) | 통과 |
| `shadowed` + 대체 계획(N9), 대체 불가(N10) | 통과 |
| `needsRepair`(R1, 절대·상대·dangling; 지금 대상과 번들 없음 표시, 제거 뒤 남은 상대 경로 링크) | 통과 |
| `ready`(L1\~L6, N1\~N5, N8), `namesUnavailable`(N6, N6b), `noWritableLocation`(L5) | 통과 |
| 첫 실행 정책: 제안 대상 5종 제안, helper 없음·임시 위치·설치됨 미제안, 답한 뒤 미제안 | 통과 |
| 동의 없는 파일 변경 0: 상태 확인 경로(I1, N6, N9, 확인 단계 sheet)에서 `mutationCount == 0` | 통과 |
| UI 스레드 밖 확인: `StellarCommandStatusCheck`, 첫 실행 판정, sheet 확인·Check Again에서 메인 스레드 파일 조회 0 | 통과 |

## 차이(작업 지시와 다른 점)

- 구현은 이 행의 WBS 커밋이 아니라 시작 창 검토 수정 커밋(`Fix:`)에 있습니다.

## 남은 위험 / 한계

- 셸 alias·함수, 순서 모르는 폴더 목록 밖의 PATH 폴더는 판정에 들어가지 않습니다(셸 실행 필요).
- 앱 활성화 때 확인하지 않으므로, 앱 실행 중 다른 프로그램이 같은 이름을 차지하면 다음 관리 진입점 확인 때 알게 됩니다.

## 재개 조건

- XXIV.5.4가 `[x]`가 됩니다.
- helper가 든 빌드에서 같은 상태 fixture를 다시 통과시킨 뒤 `Task(XXIV.5.5)`로 표기를 올립니다.
