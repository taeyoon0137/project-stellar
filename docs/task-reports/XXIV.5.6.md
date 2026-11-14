# 작업 보고서: XXIV.5.6 macOS 첫 실행 명령줄 설치 화면

- WBS: XXIV.5.6
- 작업명: `stellar` helper를 포함한 빌드의 첫 실행 설치 화면, 명시적 동의 전 파일 변경 0, 설치를 되돌리는 진입점, 실제 설치한 이름 표시를 구현하고 화면 구성·진입점 배치를 HIG 근거로 기록합니다.
- 상태: 미완료 (`[ ]` 유지). 선행 XXIV.5.5(`[ ]`), XII.9.1(`[ ]`), XII.6.5(`[ ]`)가 모두 `[x]`가 아니고, 수용 기준 중 앱 안 실제 표시와 키보드·VoiceOver smoke를 아직 확인하지 못했습니다. sheet 구현과 화면 밖 검사는 시작 창 검토 수정 커밋(`a481400`, `ab974fa`)에 있습니다.
- 선행: XXIV.5.5 (`[ ]`), XII.9.1 (`[ ]`), XII.6.5 (`[ ]`) — 모두 엄격

## 필수 실행 환경

- `DEV-MAC`: Apple silicon(arm64), macOS 27.2, Xcode 27.2, Debug 빌드.

## 변경 파일 목록

- `apps/macos/StellarMac/UI/Welcome/WelcomeCommandInstallSheetController.swift`: 단계별 화면(확인 중, 설치 제안, 대체 이름 제안, 다시 연결, 가림, 두 이름 막힘 알림, 설치됨, 제거 결과, 실패), 단계 변화 announcement
- `apps/macos/StellarMac/UI/Welcome/WelcomeViewController.swift`: `presentCommandInstallOfferIfNeeded`, 관리 진입점 `showStellarCommandSetup(_:)`과 메뉴 활성 판정, 실제 File 메뉴 항목(`performShellCommand:` + `ShellCommand`)을 받는 경로와 통합 어댑터용 `availability(of:)`·`run(_:)`
- `apps/macos/StellarMac/UI/Welcome/WelcomeStrings.swift`: sheet 문구, 메뉴 항목 "Set Up the stellar Command…"
- `apps/macos/StellarMac/UI/Welcome/WelcomeStyle.swift`: 첫 실행 정책, 복사 버튼 피드백, `WelcomeAccessibility.announce`
- `apps/macos/StellarMac/UI/Welcome/WelcomeUICheck.swift`: 설치 sheet 검사
- `apps/macos/StellarMac/main.swift`, `scripts/macos/smoke.sh`: `--welcome-ui-check` 실행 인자와 smoke 단계(리뷰 수정 커밋)
- `docs/design/ui/welcome.md` 6.6절·7장(정의 원본)
- `docs/task-reports/XXIV.5.6.md`(이 보고서). 계획서 상태 표기는 바꾸지 않았습니다.

## 결정: 화면 구성과 진입점

| 항목 | 결정 | 근거 |
|---|---|---|
| 형식 | 시작 창에 붙는 sheet. 아이콘(SF Symbol `terminal`), 제목, 설명, 링크 위치·대상, 주의 문구, PATH 한 줄과 복사 버튼, 버튼 줄 | HIG Sheets(범위가 정해진 작업, 부모 창 dim), 가이드 5.1 |
| 버튼 | 주 동작이 기본 버튼(Return), Not Now·Close가 Escape. 끝내면 Done, 바꾸지 않고 닫으면 Close·Not Now. Remove Command는 파괴적 표시이고 기본 버튼이 아니며 앞쪽에 둠 | HIG Sheets(Done·Cancel/Close 의미), 가이드 5.1 |
| 이름 표시 | 대체 이름이면 제목 "Install the Command as stellar-cli?", 버튼 "Install as stellar-cli", 설치 뒤 "The stellar-cli Command Is Installed". 링크 줄에 실제 경로 | ADR-0005 §2 "설치 화면은 실제로 설치한 이름을 보여 줌" |
| 첫 실행 | 시작 창이 보이고 다른 sheet가 없을 때만, 상태 확인을 마친 뒤 띄움. helper 없는 빌드에서는 띄우지 않음 | ADR-0005 §7, HIG Launching("Launch fast") |
| 되돌리기 진입점 | 앱 메뉴 "Set Up the stellar Command…"(추가 입력이 필요하므로 말줄임표, title-style 대문자)로 같은 sheet를 확인 단계부터 엶. 설치된 상태에서 Remove Command 제공. sheet가 떠 있으면 메뉴 비활성 | HIG The menu bar, 가이드 8.1 |
| 접근성 | 모든 버튼 제목, 아이콘만 있는 복사 버튼에 레이블·도움말, 단계 변화는 `announcementRequested`로 알림, 복사 뒤 "Copied" 피드백 | 가이드 8.1·8.3, AppKit NSAccessibility |
| 외형 | 시스템 의미 색과 텍스트 스타일만 사용, 직접 넣은 반투명 표면 없음 | 가이드 4.1·4.2 |

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| Debug `xcodebuild … build`(XXIV.5.3 보고서와 같은 옵션) | `BUILD SUCCEEDED` |
| `StellarMac --welcome-ui-check`(커밋된 실행 인자) 3회 | 3회 모두 `welcome-ui ok: 378 checks`, 종료 코드 0 |
| `./scripts/macos/smoke.sh`(`--welcome-ui-check` 포함) | `welcome-ui ok: 378 checks …`, `smoke OK`, 종료 코드 0 |

| 수용 기준 | 결과 |
|---|---|
| 첫 실행 설치 화면 표시 | **미확인.** 화면 밖 창에서 sheet 내용·레이아웃만 검사했습니다. `AppDelegate`가 시작 창과 `presentCommandInstallOfferIfNeeded()`를 부르는 연결은 이 영역 밖이라 하지 않았고, helper가 든 빌드도 아직 없습니다. 첫 실행 판정은 보이지 않는 창이면 띄우지 않는 것까지 확인했습니다. |
| 동의 없는 파일 변경 0 | 통과: sheet 표시 전·Not Now·Escape·알림 상태에서 `mutationCount == 0`, 링크 없음 |
| 되돌리기 후 설치 항목 0 | 통과: sheet Install → Remove Command 뒤 `installedLinkPaths()` 빈 배열, 링크 없음(메모리·임시 폴더) |
| 실제 설치한 이름 표시 | 통과: 대체 이름 제안 제목·버튼·링크 줄, 설치 뒤 제목이 `stellar-cli` |
| Update 동의 화면(리뷰 수정) | 통과: 확인한 다른 Stellar 사본이면 그 앱 위치와 "another copy of Stellar", 번들이 없는 링크면 대상 경로와 "no longer exists"를 보여 주고 Stellar 사본이라고 하지 않음. 링크 대상 줄은 이 앱 helper, 버튼 Update, 두 경우 모두 4개 외형 레이아웃 통과. 수정 전 트리에서는 대상 경로가 없고 dangling 링크를 Stellar 사본이라고 해 실패했습니다. |
| 메뉴 활성 판정(리뷰 수정) | 통과: `MainMenuBuilder`가 만든 실제 File 메뉴 New·Open·Clone 항목(`performShellCommand:`, target 없음)이 표본·Git 없음·진행 중에서 버튼과 같은 판정, 응답자 체인(목록 → 시작 창 controller)으로 도달, 진행 중에는 아무것도 띄우지 않음, 캔버스 명령 메뉴는 비활성. 수정 전 트리에서는 시작 창이 이 action에 응답하지 않아 실패했습니다. 메뉴를 앱에 설치하는 통합은 아직 없습니다. |
| 키보드 smoke | 부분: 주 버튼 Return·보조 버튼 Escape key equivalent, 파괴적 버튼 비기본, 작업 중 Escape 무시를 검사했습니다. 실제 키 창에서의 기본 버튼 강조와 Full Keyboard Access 이동은 확인하지 않았습니다. |
| VoiceOver smoke | **미확인.** announcement 요청과 접근성 레이블만 검사했습니다. 실제 VoiceOver 읽기는 확인하지 않았습니다. |
| 단계별 레이아웃(Light·Dark·대비 높이기) | 통과: 설치 제안, 설치됨, 대체 이름 제안, 그 밖의 13개 단계, 작업 중, 확인 중에서 모호한 레이아웃, 잘린 레이블, 직접 넣은 반투명 표면, 하드코딩 색 0 |

## 차이(작업 지시와 다른 점)

- 구현은 이 행의 WBS 커밋이 아니라 시작 창 검토 수정 커밋(`Fix:`)에 있습니다.
- 메뉴 항목 자체는 Tools 영역(`ShellCommand`, XII.6.5)이 소유하므로 추가하지 않았습니다. 문구와 action(`showStellarCommandSetup:`)만 준비했습니다.

## 남은 위험 / 한계

- 앱 통합(시작 창 표시, 메인 메뉴 설치와 메뉴 항목 추가) 전에는 사용자가 이 화면을 볼 수 없습니다. smoke 연결은 리뷰 수정 커밋에서 했습니다.
- File 메뉴의 앱 수준 router가 명시 target이면 시작 창에 묻지 않습니다. 통합 어댑터가 `availability(of:)`·`run(_:)`을 써야 하며(welcome.md 7장·15장 4), 그 연결 전에는 시작 창의 ⌘N·⌘O가 동작하지 않습니다.
- 가이드 7.3·10.4는 ADR-0005 Accepted 전 문구로 남아 있습니다(이 영역 밖 문서).

## 재개 조건

- XXIV.5.5, XII.9.1, XII.6.5가 `[x]`가 됩니다.
- `AppDelegate`가 시작 창과 첫 실행 제안을 부르고, 앱 메뉴에 "Set Up the stellar Command…"가 연결됩니다.
- helper가 든 빌드를 처음 실행해 sheet가 뜨는지, 키보드만으로 Install·Not Now·Remove Command를 할 수 있는지, VoiceOver가 제목과 단계 변화를 읽는지 확인한 뒤 `Task(XXIV.5.6)`로 표기를 올립니다.
