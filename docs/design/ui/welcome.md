# 시작 창(Welcome)과 첫 실행 UI 명세

- 상태: 초안(2026-11-14), 코어 연결 전(Stub 데이터)
- 영역: `apps/macos/Stellar/UI/Welcome/`
- 기준: [macOS 셸 디자인 가이드](../macos-shell-design.md)(이하 "가이드"), 기술 아키텍처 명세(이하 "아키텍처"), Apple Human Interface Guidelines(이하 "HIG")
- WBS 연결
  - New Project·Open·Clone 명령: 계획 XII.6.1~XII.6.3.
  - `stellar` 명령 설치: 계획 XXIV.5.3(위치 결정), XXIV.5.4(설치·제거), XXIV.5.5(상태 확인), XXIV.5.6(첫 실행 화면). 위치·안전 조건·확인 규칙은 6장이 정의하고, 각 행 보고서(`docs/task-reports/XXIV.5.3.md`~`XXIV.5.6.md`)가 근거와 검사 결과를 기록합니다.
  - 시작 창 화면 자체는 전용 WBS가 없습니다(가이드 10.4).

## 1. 범위

이 영역은 열린 프로젝트가 없을 때 보이는 시작 창과 첫 실행 안내를 맡습니다.

| 구성 | 내용 | 근거 |
|---|---|---|
| 시작 창 | New Project, Open Project, Clone Project 진입점과 최근 프로젝트 목록 | 가이드 7.1, 아키텍처 §25 |
| 빈 상태 | 최근 프로젝트 없음·읽는 중·읽기 실패 | 가이드 7.1, HIG Writing |
| Git 안내 | 시스템 Git이 없거나 쓸 수 없을 때 설치 안내만 표시, 자동 설치 없음 | 아키텍처 §24.1, 가이드 7.2 |
| Clone sheet | 원격 URL, 새 폴더 이름, 상위 폴더 입력 | 계획 XII.6.3, 아키텍처 §11.2 |
| `stellar` 명령 설치 sheet | 첫 실행 시 셸 PATH에 `stellar` 명령을 넣을지 묻고, 설치·제거·결과·오류와 실제 설치한 이름을 보여 줌 | ADR-0005 §2·§7, 계획 XXIV.5.3~XXIV.5.6 |

셸은 의미를 소유하지 않습니다. 프로젝트 생성, 폴더 판정(ProjectProbe), Clone, 최근 목록 저장, Git 탐색은 모두 코어가 하고, 이 영역은 값 snapshot을 그리고 명령을 내보내기만 합니다(가이드 1.2 원칙 4, 아키텍처 §8.1).

## 2. 화면 구성

```text
┌──────────────────────── 시작 창 (NSWindow: 제목 "Welcome", 닫기, 크기 고정) ────────────────────────┐
│  [앱 아이콘]                          │  📁 Design System                                          │
│  Stellar                              │     ~/Projects/design-system                               │
│  Version x.y.z                        │  📁 Marketing Site                                         │
│                                       │     ~/Projects/marketing-site                              │
│  [⊞ New Project…    ] ⌘N              │  ❔ Archived Prototype                                      │
│  [📁 Open Project…  ] ⌘O              │     Not found: ~/Projects/archived-prototype               │
│  [⇩ Clone Project…  ]                 │                                                            │
│                                       │   (목록이 비었거나 읽는 중·실패면 이 자리에 상태 문구)       │
│  (진행 중 작업: 제목, 진행 막대, Cancel) │                                                            │
│  (Git 안내: ⚠ 제목, 설명, 명령+복사,     │                                                            │
│   Check Again, Locate Git…)            │                                                            │
└───────────────────────────────────────┴────────────────────────────────────────────────────────────┘
```

- 창은 표준 `NSWindow`(titled, closable, full-size content view, 투명 제목 막대)이며 크기 조절을 막습니다. 자체 창 프레임이나 창 컨트롤을 만들지 않습니다(가이드 2.1). 콘텐츠 위쪽은 safe area 아래에 둡니다.
- 창 제목은 "Welcome"입니다. 화면에는 숨기지만 Window 메뉴와 VoiceOver가 창을 구분하는 데 씁니다. 앱 이름은 제목에 반복하지 않고 콘텐츠의 큰 제목으로만 보여 줍니다(가이드 2.2).
- 창 위치는 frame autosave(`StellarWelcomeWindow`)로 기억합니다(가이드 2.6).
- 왼쪽 열: 앱 아이콘(시스템이 주는 앱 아이콘을 그대로 표시, 새 그래픽 없음), 앱 이름(large title), 버전(secondary label), 동작 버튼 세 개, 진행 표시, Git 안내.
- 오른쪽: 최근 프로젝트 목록(`NSTableView`, inset 스타일, 머리글 없음, 자동 행 높이). 행은 SF Symbol, 이름(primary), 경로(secondary, `~`로 줄이고 가운데 말줄임)입니다.
- 두 열 사이는 시스템 구분선(`NSBox` separator)입니다. 직접 그린 배경, 반투명 표면, 커스텀 컨트롤이 없습니다(가이드 1.2 원칙 3, 3.2).

## 3. 상태

### 3.1 최근 프로젝트 목록

| 상태 | 표시 | 동작 |
|---|---|---|
| 읽는 중(`loading`) | 작은 회전 표시 + "Loading Recent Projects…" | 없음 |
| 빈 목록(`loaded([])`) | "No Recent Projects" + "Projects you create, open, or clone appear here." | 다음 동작은 같은 창 왼쪽 열의 세 버튼이 맡습니다. 빈 상태 안에 같은 버튼을 한 번 더 두지 않습니다(VoiceOver 중복 방지). |
| 목록(`loaded([...])`) | 행 목록 | 4절 |
| 실패(`failed(reason)`) | "Couldn’t Load Recent Projects" + 이유 코드(`WelcomeFailureReason`)에서 셸이 만든 설명 + Try Again | Try Again → `reloadRecentProjects` |
| 없어진 폴더(`isMissing`) | `questionmark.folder` 아이콘, 이름은 secondary 색, 경로 앞에 "Not found:" | 열기를 누르면 코어가 `projectMissing` 안내를 돌려줍니다. Show in Finder는 비활성 |

선택 상태와 "없음" 상태는 색에만 기대지 않고 아이콘과 문구를 함께 바꿉니다(가이드 4.2).

### 3.2 Git 상태(아키텍처 §24.1)

| 상태 | 표시 | New·Clone | Open |
|---|---|---|---|
| 탐색 중(`checking`) | 안내 없음 | 켬 | 켬 |
| 사용 가능(`available`) | 안내 없음 | 켬 | 켬 |
| 없음(`notFound`) | ⚠ "Git Isn’t Installed", 설명, Terminal에서 실행할 명령 `xcode-select --install`(선택·복사 가능), Check Again, Locate Git… | 끔 + 도움말 "Requires Git…" | 켬 |
| 쓸 수 없음(`unusable(problem)`) | ⚠ "Git Can’t Be Used", 문제 코드(`WelcomeGitProblem`: 버전이 낮음·실행 파일 아님·실행 실패)에서 셸이 만든 설명, Check Again, Locate Git… | 끔 | 켬 |

- 설치는 실행하지 않습니다. 명령은 보여 주고 복사만 하게 합니다. 셸은 시스템 Git을 직접 실행하지 않으므로, Git이 없는 Mac에서 시스템 설치 대화상자가 뜨게 하지 않습니다.
- Locate Git…은 탐색 순서 1번(사용자 지정 경로)을 지정하는 경로입니다. 고른 경로를 `useGitExecutable`로 넘기고, 검사는 코어가 합니다.
- New Project는 `git init`, Clone은 `git clone`이 필요하므로 끕니다. Open은 제한 모드 열기(아키텍처 §24.3)가 Git 없이도 가능하므로 켜 둡니다.

### 3.3 진행 중 작업

- 생성·열기·Clone 중에는 왼쪽 열에 "Cloning “이름”…" 같은 제목, 진행 막대(진행률을 모르면 불확정), Cancel(취소 가능할 때만)을 보이고 세 동작 버튼을 끕니다.
- 제목에는 프로젝트 표시 이름만 넣고 Clone URL은 넣지 않습니다(아키텍처 §39: Git URL은 민감값).
- 작업에는 코어가 발급한 식별자(`WelcomeActivity.id`)가 있고, Cancel은 지금 보이는 작업의 식별자로 `cancelActivity(id:)`를 보냅니다. 한 작업이 끝나고 다른 작업이 시작된 직후 늦게 도착한 취소가 새 작업을 취소하지 않도록, 코어는 식별자가 다르면 무시합니다.

### 3.4 안내(alert)

코어가 snapshot에 `notice`를 실으면 창의 sheet로 `NSAlert`을 한 번 띄우고, 닫으면 `dismissNotice(id)`를 보냅니다(가이드 5.1, HIG Alerts). 다른 sheet가 떠 있으면 그 sheet가 닫힌 뒤 띄웁니다.

| 종류 | 제목 | 근거 |
|---|---|---|
| `notStellarProject` | This Folder Isn’t a Stellar Project — `stellar.yaml`이 없어 열 수 없고 기존 저장소를 전환하지 않음 | 아키텍처 §25.2, 가이드 7.2 |
| `projectMissing` | Project Not Found | — |
| `folderAlreadyExists` | A Folder with This Name Already Exists | — |
| `gitIdentityRequired` | Set Your Git Name and Email — 최초 커밋 전에 `user.name`·`user.email` 필요 | 아키텍처 §25.1, 가이드 7.2 |
| `operationFailed(reason)` | The Operation Couldn’t Be Completed + 이유 코드에서 만든 설명(권한, 디스크 공간, 네트워크, 원격 인증, 원격 없음, Git 종료 상태, 그 밖의 오류 코드, 프로젝트 열기 실패 원인, 이 빌드에 없는 New·Clone) | 아키텍처 §37.3(자격 증명은 시스템 Git) |

문구는 셸이 종류·이유 코드에서 만듭니다. 코어는 문구가 아니라 코드를 보냅니다(지역화 대비). 경로, 버전, 종료 상태, 오류 식별자 같은 값만 그대로 끼워 넣습니다.

## 4. 상호작용

| 동작 | 결과 명령 | 비고 |
|---|---|---|
| New Project… | `NSSavePanel`(제목 "New Project", 버튼 "Create", 이름표 "Project Name:")에서 상위 폴더와 새 폴더 이름을 받아 `createProject(parentDirectory:folderName:)` | 아키텍처 §25.1, 계획 XII.6.1. 이미 있는 이름은 `panel(_:userEnteredFilename:confirmed:)`에서 Save를 누른 순간(`okFlag` true) 막고 패널 위 알림을 띄웁니다. AppKit은 이 메서드를 "바꾸기" 확인보다 먼저 부르고 nil이면 저장을 취소한 채 패널을 둡니다(NSOpenSavePanelDelegate). `validate`는 바꾸기 확인 뒤에 불리므로 두 번째 방어선입니다. |
| Open Project… | `NSOpenPanel`(폴더만)에서 고른 폴더로 `openProject(directory:)` | 폴더 판정은 코어(계획 XII.6.2) |
| Clone Project… | Clone sheet(5절) → `cloneProject(remoteURL:parentDirectory:folderName:)` | 계획 XII.6.3 |
| 행 더블클릭, Return, Enter | `openRecent(id:)` | 빈 곳 더블클릭은 무시 |
| Delete, Forward Delete | `removeRecent(id:)` | 목록에서만 뺍니다. 폴더는 건드리지 않습니다. |
| 문맥 메뉴 | Open / Show in Finder / Remove from Recents | Show in Finder는 셸이 `NSWorkspace`로 처리 |
| Try Again | `reloadRecentProjects` | |
| Check Again | `recheckGit` | |
| Locate Git… | `NSOpenPanel` → `useGitExecutable(path:)` | |
| Cancel(진행 중) | `cancelActivity(id:)` | 계획 XIV.1.4. 3.3절 |

모든 패널은 창 sheet로 띄웁니다. 패널이 없는 상태(창 없음)에서는 명령을 보내지 않습니다.

## 5. Clone sheet

- 입력: Repository URL(`NSTextField`, content type URL), Name(`NSTextField`), Location(경로 표시 + Choose…). 모두 AppKit 네이티브 텍스트 컨트롤입니다(아키텍처 §11.2, 가이드 5.2).
- 양식은 오른쪽 정렬 이름표 열과 남는 폭을 쓰는 입력 열로 배치합니다(HIG Layout).
- 이름은 URL에서 자동으로 채웁니다(`…/design-system.git` → `design-system`, `git@host:team/ds.git` → `ds`). 사용자가 이름을 고치면 더 이상 덮어쓰지 않습니다.
- 검사: URL은 비어 있지 않고 공백이 없어야 하며, 이름은 비어 있지 않고 `/`, `:`, 앞쪽 `.`이 없어야 합니다. 통과하기 전에는 Clone 버튼이 꺼져 있습니다. URL 형식과 접근 가능 여부는 코어·Git이 판단합니다.
- 키: Return = Clone(기본 버튼), Escape = Cancel.
- 안내 문구로 `stellar.yaml`이 있는 저장소만 열 수 있음을 미리 알립니다(아키텍처 §25.2).
- URL은 로그·진단에 남기지 않습니다(아키텍처 §39).

## 6. `stellar` 명령 설치(첫 실행, 계획 XXIV.5.3~XXIV.5.6)

### 6.1 결정과 문서 관계

- ADR-0005(Accepted) §2: 사용자가 부르는 명령은 `stellar`이고(Cargo package는 `stellar-cli`, 실행 파일 이름만 `stellar`), PATH에서 `stellar`가 Stellar 자기 링크가 아닌 다른 프로그램으로 먼저 해석되면 `stellar-cli`로 설치하며, `stellar-cli`도 다른 프로그램이 쓰면 설치하지 않고 알립니다. 설치 화면은 실제로 설치한 이름을 보여 줍니다.
- ADR-0005 §7: 앱 번들에 `stellar`를 helper로 넣고(배치·서명은 XXIV.5.1), 첫 실행 때 설치 화면을 제공하며, 동의 전에는 어떤 파일도 만들거나 바꾸지 않습니다. 위치·제거 방법·안전 조건은 아키텍처 §37(최소 권한)과 Apple·각 셸 공식 문서를 근거로 이 영역이 정합니다. 설치 상태는 OS 사용자 데이터에만 둡니다.
- Debug와 Release에서 앱에 포함하는 helper 경로는 `Contents/Helpers/stellar`입니다. 배치와 서명 검사 근거는 [XXIV.5.1 보고서](../../task-reports/XXIV.5.1.md)에 기록합니다.
- helper가 손상되거나 빠진 번들은 `helperUnavailable` 상태가 되고 첫 실행 제안을 띄우지 않습니다.
- 가이드 7.3·10.4는 아직 "ADR-0005가 Accepted되기 전에는 이 화면을 정하지 않음"으로 남아 있습니다. 이 영역 작업은 가이드를 수정하지 않으므로, 가이드 갱신은 별도 작업으로 남깁니다(15장).

### 6.2 원칙

| 원칙 | 구현 | 근거 |
|---|---|---|
| 명시적 동의 | Install(또는 Install as stellar-cli, Update)을 눌러야만 파일을 바꿉니다. 상태 판정은 읽기만 합니다. 첫 실행 sheet의 기본 버튼은 주 동작(Return), Not Now는 Escape입니다. | ADR-0005 §7 |
| 최소 권한 | 관리자 인증, 보조 도구 설치, 셸 실행을 쓰지 않습니다. 현재 사용자 홈 아래 폴더에만 링크를 만들고, 여러 사용자가 함께 쓰는 폴더(`/usr/local/bin` 등)는 쓸 수 있어도 고르지 않습니다. | 아키텍처 §37.3(최소 권한), §37.1(실행 파일 제한) |
| 셸 설정 파일 불변·불실행 | `~/.zshrc` 등을 고치지도 실행하지도 않습니다. zsh 시작 파일은 셸이 읽어 실행하는 명령 파일이므로(zsh(1) STARTUP/SHUTDOWN FILES), 결과 PATH를 알려면 사용자 코드를 실행해야 합니다. 대신 PATH에 추가할 한 줄을 보여 주고 복사만 하게 합니다. | zsh(1), 아키텍처 §37 |
| 남의 파일 보존 | Stellar 소유 링크가 아닌 파일·링크·폴더는 덮어쓰거나 지우지 않습니다. 새 링크는 `symlink(2)`로 만들어 그 자리에 무엇이 있으면 `EEXIST`로 실패합니다. | symlink(2) |
| 원자적 교체·경합 방지 | 오래된 소유 링크는 같은 폴더에 새 링크를 만든 뒤 `renamex_np(RENAME_SWAP)`로 맞바꾸고, 맞바뀐 항목이 확인한 소유 링크가 아니면 다시 맞바꿔 되돌립니다. 제거는 항목을 같은 폴더 임시 이름으로 `rename(2)`한 뒤 확인하고, 아니면 `RENAME_EXCL`로 되돌립니다. 맞바꾸기를 지원하지 않는 볼륨은 같은 확인·되돌리기 방식으로 바꿉니다. | rename(2) |
| 되돌릴 수 있음 | Remove Command가 후보 폴더에서 이 앱 helper를 가리키는 링크(`stellar`, `stellar-cli`)를 모두 지웁니다. 설치는 이 앱 helper를 가리키는 링크만 만들므로 이것으로 설치를 되돌립니다. 다른 Stellar 사본을 가리키거나 번들이 없어 소유를 확인하지 못한 링크는 관리 화면에 보인 적이 없으므로 지우지 않습니다(6.4절). 설치 때 만든 폴더는 다른 도구도 쓸 수 있으므로 남깁니다. | ADR-0005 §7 |
| 메인 스레드 보호 | 설치기 생성(`/etc/paths` 읽기), 상태 판정, 설치·제거는 별도 큐에서 하고 결과만 메인 스레드에 반영합니다. sheet가 닫히거나 다른 작업이 시작되면 늦게 온 결과는 버립니다. | 아키텍처 §6.3·§6.4, 가이드 9장, HIG Launching("Launch fast") |
| 기록 | 첫 실행 제안에 답했는지만 앱 사용자 기본값(`StellarCommandInstallOffered`, OS 사용자 데이터)에 둡니다. 설치 여부는 기록하지 않고 매번 파일 시스템에서 판정합니다. 프로젝트, Git, 텔레메트리에는 남기지 않습니다. | ADR-0005 §7 |
| 실행 환경 전제 | 앱이 App Sandbox 밖(Developer ID 배포)에서 실행된다는 전제입니다. 샌드박스 앱은 홈 아래 임의 폴더에 쓸 수 없습니다. | 현재 Xcode 설정(샌드박스 없음, hardened runtime) |

### 6.3 위치 결정(XXIV.5.3)

1. 후보 폴더(선호 순): `~/.local/bin`, `~/bin`. 모두 현재 사용자 홈 아래라 다른 사용자 계정에 영향이 없고 관리자 권한이 필요 없습니다(§37 최소 권한). `~/.local/bin`은 XDG Base Directory 명세가 사용자 실행 파일 위치로 정한 폴더이고, `~/bin`은 그다음 흔한 사용자 폴더입니다.
2. 이미 있는 Stellar 소유 링크가 있으면 그 링크를 관리합니다(같은 명령을 두 곳에 만들지 않음, 6.5절).
3. 후보 중 **확인한 PATH에 있고** 쓸 수 있는(없으면 만들 수 있는) 첫 폴더를 고릅니다.
4. 없으면 쓸 수 있는(또는 만들 수 있는) 첫 후보를 고르고, 필요하면 만듭니다. 이때 PATH 추가 안내를 보입니다.
5. "쓸 수 있음": 링크를 따라가 현재 사용자가 쓸 수 있는 디렉터리이고 읽기 전용 볼륨이 아님. "만들 수 있음": 홈 아래이고 가장 가까운 기존 상위 폴더에 쓸 수 있음.
6. 어느 후보도 쓸 수 없으면 `noWritableLocation(~/.local/bin)`을 보입니다.
7. 앱 번들이 App Translocation 위치(경로에 `/AppTranslocation/`) 또는 읽기 전용 볼륨(`URLResourceKey.volumeIsReadOnlyKey`)에 있으면 링크 대상이 곧 사라지므로 설치하지 않습니다(`unsupportedAppLocation`). Apple Code Signing Guide(보관 문서)의 Gatekeeper path randomization 설명대로 격리된 앱은 임의 위치로 옮겨 실행됩니다. 경로 문자열 판정은 Apple이 지원하는 API가 아니므로 읽기 전용 볼륨 판정을 함께 씁니다.

**확인한 PATH**(셸을 실행하지 않고 아는 부분)

- 순서를 아는 폴더(`ordered`): `/etc/paths`, `/etc/paths.d/*`(이름순), 앱 프로세스의 `PATH` 순서. path_helper(8)는 `/etc/paths`를 기본값으로 읽고 `/etc/paths.d`의 내용을 덧붙여 PATH를 만들며, macOS의 `/etc/zprofile`이 login zsh에서 path_helper를 실행합니다. Terminal은 기본으로 사용자 계정의 login shell(zsh)을 엽니다(Terminal 사용 설명서 "General settings": Shells open with — Default login shell).
- 순서를 모르는 폴더(`unordered`): 사용자 셸 설정 파일이 흔히 PATH에 더하는 폴더(홈 후보 두 곳, Homebrew Apple silicon 기본 접두사 `/opt/homebrew/bin`, MacPorts 기본 접두사 `/opt/local/bin`) 중 `ordered`에 없는 것. 이름 충돌 판정에만 씁니다(6.4절). zsh는 `~/.zprofile`·`~/.zshrc`를 `/etc/zprofile`(path_helper) 뒤에 읽으므로(zsh(1) STARTUP/SHUTDOWN FILES), 이 파일들이 앞에 붙인 폴더는 시스템 폴더보다 먼저 탐색될 수 있습니다.
- `ordered`에 없는 폴더는 "PATH에 없음"이 아니라 "확인하지 못함"으로 표시합니다("Stellar couldn’t confirm that … is on your PATH").

### 6.4 이름 결정과 설치·제거(XXIV.5.4)

**셸 명령 탐색 모델**: zsh(zshmisc(1) COMMAND EXECUTION), bash(bash(1) COMMAND EXECUTION), POSIX sh(Shell Command Language 2.9.1.1)는 `/`가 없는 명령 이름을 함수·builtin이 아니면 PATH 폴더 순서대로 찾아 **처음 만난 실행 파일**을 씁니다. 실행 권한이 없는 같은 이름 파일은 고르지 않습니다.

**다른 프로그램**: 링크를 따라가 실행 가능한 일반 파일이고 Stellar 소유 링크가 아닌 것.

**Stellar 소유 링크**: 심볼릭 링크이고, 대상(상대 경로는 링크 폴더 기준으로 풀어서)이 `<이름>.app/Contents/Helpers/stellar` 형식이며, 그 앱 번들이 있으면 `CFBundleIdentifier`가 Stellar(`xyz.taeyoon.stellar`)의 것인 링크. 앱이 옮겨지거나 지워져 번들이 없는(dangling) 링크도 Stellar 소유로 봅니다. 다른 앱의 `Contents/Helpers/stellar`를 가리키는 링크는 다른 프로그램입니다.

- dangling 링크는 번들 식별자를 확인할 수 없어 모양만으로 소유로 본 것이므로, 같은 모양의 helper를 가진 다른 앱이 남긴 링크일 수 있습니다. 그래서 바꾸기 전에 Update 동의 화면이 링크의 지금 대상(상대 경로는 링크 폴더 기준으로 푼 절대 경로)과 그 대상이 더 이상 없다는 사실을 보여 주고, Stellar 사본이라고 단정하지 않습니다(`replaceOwnLink(previousDestination:previousAppExists:)`). 번들이 있어 식별자로 확인한 링크만 "another copy of Stellar"로 부르고 그 앱 위치를 보여 줍니다.
- Remove Command는 이 앱 helper를 가리키는 링크만 지우고, dangling 링크와 다른 사본의 링크는 남깁니다(6.2절). 남은 링크는 다음 확인에서 `needsRepair`가 되어 같은 동의 화면으로만 바꿉니다.

**이름 결정**(폴더 D에 이름 N을 설치할 때, N = `stellar` → `stellar-cli` 순)

| 판정 | 조건 | 결과 |
|---|---|---|
| 설치 위치 충돌 | `D/N`에 Stellar 소유 링크가 아닌 항목(파일·링크·폴더, 실행 가능 여부 무관) | N을 쓸 수 없음(`atInstallLocation`) |
| PATH 앞쪽 충돌 | D가 `ordered`에 있으면 그보다 앞 폴더, 없으면 `ordered` 전체 중 한 곳에 다른 프로그램 N | N을 쓸 수 없음(`earlierOnPath`) |
| 순서 모르는 충돌 | `unordered`(D 제외)에 다른 프로그램 N. 먼저 탐색될 수 있으므로 보수적으로 충돌로 봄 | N을 쓸 수 없음(`possiblyEarlierOnPath`) |
| PATH 뒤쪽 | D가 `ordered`에 있고 그보다 뒤 폴더에 다른 프로그램 N | N을 씀. 설치하면 그 프로그램이 가려진다는 사실을 동의 화면에 알림(`hiddenProgramPaths`) |
| 자기 링크 | `D/N`이 이 앱 helper를 가리키는 소유 링크 | 설치됨으로 판정(`stellar` 유지) |

- `stellar`를 쓸 수 없으면 `stellar-cli`로 같은 판정을 하고, 쓸 수 있으면 그 계획에 `stellar`를 못 쓴 이유(`primaryNameConflict`)를 실어 보냅니다.
- 둘 다 쓸 수 없으면 `namesUnavailable([충돌 2개])`: 아무것도 설치하지 않고, 두 충돌을 문장으로 보여 주는 알림 sheet(Close만)를 띄웁니다.

**실행**

- 실행 직전에 다시 확인합니다: helper 존재, 계획한 이름의 PATH 앞쪽 충돌, 설치 위치 항목. 그 사이 바뀌었으면 `conflict`로 보고하고 아무것도 바꾸지 않습니다. 충돌 뒤에는 같은 계획을 다시 실행하지 않고 Check Again으로 상태부터 다시 확인해 새 동의를 받습니다.
- 이미 이 앱을 가리키는 링크면 바꾸지 않습니다(멱등).
- 폴더가 없으면 홈 아래일 때만 현재 사용자 권한으로 만듭니다.

### 6.5 상태 확인(XXIV.5.5)

**확인 시점**

- 첫 실행: 시작 창이 뜬 뒤 메인 스레드 밖에서 한 번 확인합니다. 아직 답하지 않았을 때만 sheet를 띄웁니다.
- 관리 진입점: 앱 메뉴 "Set Up the stellar Command…"를 고를 때마다 확인합니다.
- 실행 직전: 설치·제거 함수 안에서 다시 확인합니다(6.4절).
- 사용자가 Check Again을 누를 때.
- 주기적 확인, 파일 시스템 감시, 앱 활성화 때 확인은 하지 않습니다(자원 사용과 방해 최소화, HIG Launching).

**판정**(위에서부터 먼저 맞는 것)

| 상태 | 조건 | 처리 |
|---|---|---|
| `helperUnavailable` | 번들에 실행 가능한 `Contents/Helpers/stellar` 없음 | Close만. 첫 실행 제안 없음 |
| `unsupportedAppLocation` | App Translocation 또는 읽기 전용 볼륨 | Close만. 첫 실행 제안 없음, 답 기록 없음(옮긴 뒤 다시 제안) |
| `installed` | 후보 폴더에 이 앱 helper를 가리키는 소유 링크가 있고, 그 이름에 PATH 앞쪽·순서 모르는 충돌이 없음 | Done, Remove Command |
| `shadowed` | 이 앱을 가리키는 소유 링크가 있지만 모두 다른 프로그램에 가려짐 | 대체 이름을 쓸 수 있으면 Install as stellar-cli 제안, 없으면 Close. Remove Command 제공 |
| `needsRepair` | 다른 위치를 가리키는 소유 링크가 있고 그 이름이 가려지지 않음 | Update(같은 자리에서 원자적 교체) |
| `ready` | 새 설치 가능(이름 결정 결과 `stellar` 또는 `stellar-cli`) | Install / Install as stellar-cli |
| `namesUnavailable` | 두 이름 모두 다른 프로그램 | 설치 0, 알림(Close) |
| `noWritableLocation` | 쓸 수 있는 후보 없음 | 만들 폴더를 안내, Close |

- 오래된 소유 링크의 이름이 가려졌으면 그 폴더에서 이름 결정을 다시 해 `ready`(대체 이름) 또는 `namesUnavailable`로 판정합니다.
- 대체 이름으로 설치했다가 나중에 `stellar`를 쓸 수 있게 되어도 이름을 자동으로 바꾸지 않습니다(사용자가 쓰던 명령 유지).
- 셸의 명령 위치 기억: zsh(`HASH_CMDS`, `HASH_DIRS`, zshoptions(1))와 bash(hash table, bash(1))는 한 번 찾은 명령 위치를 기억하므로, 이미 열린 Terminal 창은 바뀐 링크를 바로 따르지 않을 수 있습니다. 설치 완료 문구는 "새 Terminal 창에서" 실행하라고 안내합니다.

### 6.6 화면(XXIV.5.6)

| 상태 | 제목 | 버튼 |
|---|---|---|
| 확인 중 | Checking the stellar Command… + 회전 표시 | 없음(Escape는 닫기, 파일 변경 없음) |
| `ready`(`stellar`) | Install the stellar Command? + 링크 위치·대상, 폴더 생성 예고, 가려질 프로그램, PATH 안내 | Not Now(Esc) / **Install**(Return) |
| `ready`(`stellar-cli`) | Install the Command as stellar-cli? + `stellar`를 못 쓰는 이유(경로 포함) | Not Now / **Install as stellar-cli** |
| `needsRepair` | Update the <이름> Command? + 지금 링크 대상(확인한 Stellar 사본이면 그 앱 위치, 번들이 없으면 대상 경로와 "no longer exists") | Not Now / **Update** |
| `shadowed` | Another Program Runs Instead of <이름> + 가린 프로그램 경로 | Remove Command / Not Now / **Install as stellar-cli**, 불가하면 Remove Command / **Close** |
| `namesUnavailable` | The stellar Command Wasn’t Installed + 두 충돌 | **Close** |
| `installed` | The <설치한 이름> Command Is Installed (+ PATH 안내) | Remove Command(파괴적 표시, 기본 버튼 아님) / **Done** |
| `helperUnavailable` | The stellar Command Isn’t Included | **Close** |
| `unsupportedAppLocation` | Move Stellar to Your Applications Folder | **Close** |
| `noWritableLocation` | No Folder Available | **Close** |
| 작업 중 | Installing… / Removing… + 회전 표시 | 없음(Escape 무시) |
| 설치 실패(충돌) | The <이름> Command Couldn’t Be Installed + 바뀐 경로 | Close / **Check Again** |
| 설치 실패(파일 시스템) | 같은 제목 + 원인 | Close / **Try Again** |
| 제거 결과 | The Command Was Removed / Some Links Couldn’t Be Removed | **Done** |

- 첫 실행 제안 조건: 아직 답하지 않았고 상태가 `ready`, `needsRepair`, `shadowed`, `namesUnavailable`(알림), `noWritableLocation`일 때. helper가 없거나, 임시·읽기 전용 위치이거나, 이미 설치됐으면 띄우지 않습니다. 메뉴로 sheet를 열어 답해도 제안을 끝낸 것으로 기록합니다.
- 관리 진입점: 앱 메뉴 "Set Up the stellar Command…"가 같은 sheet를 확인 단계부터 엽니다(`WelcomeWindowController.showStellarCommandSetup(_:)`, sheet가 떠 있으면 메뉴 비활성). 설치를 되돌리는 Remove Command가 여기에 있습니다. 메뉴 항목 연결은 15장 통합 작업입니다.
- sheet는 창에 붙는 modal sheet이고, 작업을 끝내면 Done, 바꾸지 않고 닫으면 Close·Not Now입니다(HIG Sheets). 파괴적 동작은 기본 버튼이 아니고 앞쪽에 둡니다(가이드 5.1).

## 7. 키보드

| 키 | 동작 | 범위 |
|---|---|---|
| ⌘N | New Project…(File 메뉴 항목) | 앱 전체 |
| ⌘O | Open Project…(File 메뉴 항목) | 앱 전체 |
| Return, Enter | 선택한 최근 프로젝트 열기 | 최근 목록 |
| Delete, Forward Delete | 최근 목록에서 빼기 | 최근 목록 |
| ↑ ↓ | 행 이동 | 최근 목록(시스템 동작) |
| Tab, Shift-Tab | 컨트롤 사이 이동 | 창 전체(`autorecalculatesKeyViewLoop`), Full Keyboard Access로 모든 컨트롤 도달 |
| Return / Escape | 기본 버튼 / 취소 | Clone sheet, 설치 sheet |

- 최근 목록이 있으면 창의 첫 포커스를 목록에 둡니다. 그 밖에는 사용자 조작 없이 포커스를 옮기지 않습니다(가이드 8.2).
- Clone은 자주 쓰는 명령이 아니므로 별도 단축키를 두지 않습니다(가이드 8.1 "커스텀 단축키는 자주 쓰는 앱 고유 명령에만").
- ⌘N·⌘O는 File 메뉴 항목(Tools 영역 `ShellCommand.newProject`·`.openProject`)이 소유합니다. 시작 창 버튼에는 같은 단축키를 걸지 않습니다(메뉴와 중복 등록 방지, 가이드 8.1 "모든 명령은 메인 메뉴 항목으로 존재"). 현재 앱은 메인 메뉴(`MainMenuBuilder`)도 시작 창도 띄우지 않으므로(15장 3·4) 앱 통합 전에는 시작 창 단축키가 동작하지 않습니다.
- 실제 File 메뉴 항목은 `MainMenuBuilder`가 만들며 action은 `performShellCommand:`, identifier는 `ShellCommand`입니다. `WelcomeViewController`는 이 모양을 그대로 받습니다.
  - 응답자 체인 경로(target 없는 항목. 실제 앱 메뉴는 `build(applicationTarget:)`로 New·Open·Clone에 앱 수준 router를 명시 target으로 달므로, 검사는 그 target을 뗀 항목으로 이 경로를 봅니다): `performShellCommand(_:)`가 New·Open·Clone을 처리하고, `NSMenuItemValidation`이 버튼과 같은 판정(`WelcomeCommandAvailability`: Git 필요 여부, 진행 중 작업, 창에 붙은 sheet)을 돌려줍니다. 시작 창 명령이 아닌 셸 명령은 실행·판정을 응답자 체인의 다음 대상(없으면 비활성)에 넘겨 가로채지 않습니다.
  - 명시 target 경로(Tools 영역 설계: 앱 수준 명령은 창이 없어도 동작하도록 앱 수준 router가 target): router는 시작 창에 묻지 않으므로, 통합 어댑터가 시작 창이 떠 있으면 `availability(of:)`를 메뉴 snapshot의 `enabledCommands`에 싣고 `run(_:)`으로 명령을 넘깁니다(15장 4).
  - 버튼 action(`newStellarProject:`, `openStellarProject:`, `cloneStellarProject:`)도 같은 판정을 씁니다. 지금 쓸 수 없는 항목은 숨기지 않고 비활성화합니다(가이드 8.1).

## 8. 접근성(가이드 8장)

- 모든 버튼은 제목이 있고, 아이콘만 있는 복사 버튼은 접근성 레이블과 도움말("Copy Command")을 둡니다.
- 최근 목록 표의 레이블은 "Recent Projects"입니다. 각 행은 하위 뷰를 접근성 요소 하나로 묶고 "이름, 경로[, Not Found]"를 읽습니다. 행 셀은 접근성 자식을 내보내지 않고(`accessibilityChildren` 빈 배열) 하위 레이블도 요소에서 빼서, VoiceOver가 셀 안으로 들어가 같은 내용을 두 번 읽지 않게 합니다.
- 행마다 접근성 사용자 동작(Open, Show in Finder, Remove from Recents)을 두어 문맥 메뉴 없이도 같은 동작을 할 수 있습니다.
- Git 안내와 진행 표시는 제목을 레이블로 가진 group 요소이고, 안의 버튼은 그대로 접근할 수 있습니다.
- 목록 상태 문구와 설치 sheet 단계가 바뀌면 `NSAccessibility.Notification.announcementRequested`(우선순위 high)로 알립니다. `.valueChanged`는 포커스가 없는 static text에서는 읽히지 않을 수 있기 때문입니다. sheet의 처음 표시는 시스템이 sheet를 읽으므로 알리지 않습니다.
- 복사 버튼은 복사한 뒤 1.5초 동안 `checkmark` 기호와 "Copied" 도움말로 바뀌고 같은 문구를 알립니다. 접근성 레이블("Copy Command")은 바꾸지 않습니다.
- 장식 아이콘(앱 아이콘, 행 아이콘, 안내 아이콘)은 접근성 트리에서 숨깁니다.

## 9. 외형과 시스템 설정

- 색은 시스템 의미 색(label 계층, separator)만 씁니다. 하드코딩한 색, asset catalog 색, 이미지 자산이 없습니다(가이드 4.2). 경고 아이콘은 SF Symbol의 시스템 multicolor 렌더링을 씁니다. 검사가 화면의 레이블 글자·배경 색, 이미지·버튼 tint, custom box 색이 모두 시스템 catalog 색인지 실행 중에 확인합니다(`NSColor(red:…)` 같은 색은 실패).
- 글꼴은 텍스트 스타일(large title, title 2·3, headline, body, callout, subheadline)과 시스템 고정폭 글꼴(명령·경로)만 씁니다(가이드 4.1).
- Light·Dark·대비 높이기는 표준 컨트롤과 의미 색이 자동으로 맞춥니다.
- 투명도 줄이기: 이 영역은 직접 넣은 반투명 표면(`NSVisualEffectView`, `NSGlassEffectView`)이 없으므로 시스템 처리로 충분합니다. 검사가 이를 확인합니다.
- 동작 줄이기: 이 영역은 직접 만든 애니메이션이 없습니다. 진행 표시와 sheet 전환은 시스템 컨트롤이 처리합니다.

## 10. SF Symbols

| 이름 | 쓰는 곳 |
|---|---|
| `plus.square` | New Project… |
| `folder` | Open Project…, 최근 프로젝트 행 |
| `square.and.arrow.down.on.square` | Clone Project… |
| `questionmark.folder` | 없어진 최근 프로젝트 행 |
| `exclamationmark.triangle.fill` | Git 안내(multicolor) |
| `doc.on.doc` | 명령 복사 버튼 |
| `checkmark` | 복사 직후 피드백 |
| `terminal` | `stellar` 명령 설치 sheet |

모두 `NSImage(systemSymbolName:)`으로 이름만 참조합니다.

## 11. 데이터 경계(프로토콜)

`WelcomeModel.swift`에 정의합니다. 모든 타입은 Swift 값 타입이고 Rust 타입을 노출하지 않습니다.

```swift
protocol WelcomeDataSource: AnyObject {
    func currentSnapshot() -> WelcomeSnapshot          // 미리 만든 값만 반환(메인 스레드, O(1))
    var snapshotDidChange: (() -> Void)? { get set }   // 메인 스레드에서 호출, 셸은 다음 런 루프에 한 번 반영
    func send(_ command: WelcomeCommand)               // 즉시 반환, 결과는 다음 snapshot
}
```

- `WelcomeSnapshot`: `generation`(세대 번호, 오래된 snapshot 폐기), `recentProjects`(loading / loaded / failed(`WelcomeFailureReason`)), `git`(checking / available / notFound / unusable(`WelcomeGitProblem`)), `activity?`(`id` 포함), `notice?`, `defaultParentDirectory?`.
- `WelcomeFailureReason`: `permissionDenied(path)`, `notEnoughSpace`, `networkUnavailable`, `authenticationFailed`, `remoteNotFound`, `gitCommandFailed(exitStatus)`, `internalError(code)`, `projectOpenFailed(ShellProjectOpenFailure, path)`(셸 어댑터가 코어 열기 실패 코드에서 고른 원인, 문구는 원인 제목과 해결 방법, XII.6.2), `projectCreationFailed(message)`(코어의 생성 실패 원인), `projectCloneUnavailable`(Clone 경로 준비 중). 코어 오류는 어댑터가 이 코드로 바꿉니다.
- `WelcomeCommand`: `createProject`, `openProject`, `cloneProject`, `openRecent`, `removeRecent`, `reloadRecentProjects`, `recheckGit`, `useGitExecutable`, `cancelActivity(id:)`, `dismissNotice`.
- 경로는 `String`(UTF-8)이라 ADR-0009의 `(ptr, len)` 문자열 규칙으로 옮기기 쉽습니다. 최근 항목 식별자는 코어가 발급한 불투명 문자열입니다.
- 표본 구현 `WelcomeDataSourceStub`은 의도적 placeholder입니다(계획 1.2 규칙 6). 실제 프로젝트를 만들거나 열지 않고, 받은 명령을 기록하며 최근 목록 제거·안내 확인·작업 취소만 흉내 냅니다.

명령 설치는 코어와 무관한 셸 기능이므로 별도 타입입니다.

- `StellarCommandInstaller`(값 타입): `status()`, `install(_:)`, `uninstall()`, `installedLinkPaths()`, `isStellarOwnedLink(atPath:)`, `shadowingProgram(named:installingIn:)`.
- `StellarCommandSearchPath`: 순서를 아는 PATH(`ordered`)와 순서를 모르는 흔한 폴더(`unordered`). `system(fileSystem:environmentPath:)`가 셸 실행 없이 만듭니다.
- `StellarCommandNameConflict`: 이름을 쓸 수 없는 이유(설치 위치, PATH 앞쪽, 순서 모르는 폴더)와 경로.
- `StellarCommandFileSystem` 프로토콜로 파일 시스템을 주입합니다. 실제 구현은 `LocalStellarCommandFileSystem`(`lstat`, `symlink`, `renamex_np`, `rename`, `unlink`, Info.plist 읽기), 검사용은 메모리 구현 `StellarCommandFileSystemStub`(변경 횟수·경합 hook 포함), 메인 스레드 호출 감시용은 `StellarCommandFileSystemThreadAudit`입니다.
- `StellarCommandStatusCheck.evaluate(on:makeInstaller:completion:)`: 설치기 생성과 판정을 큐에서 하고 결과만 메인 스레드로 넘깁니다.
- `WelcomeFirstLaunchPolicy`: 첫 실행 제안 여부를 사용자 기본값에 기록합니다. 프로젝트 파일과 Git에는 넣지 않습니다.
- 설치기는 파일 시스템을 바꾸는 로직이지만 지금은 이 영역(`UI/Welcome/`) 안에 있습니다. 앱 통합 때 UI 밖 모듈로 옮기는 것을 검토합니다(15장).

## 12. 성능과 스레드

- snapshot 반영은 바뀐 경우에만 하고, 연속 변경은 다음 런 루프에서 한 번만 반영합니다. 최근 목록은 배열이 바뀔 때만 표를 다시 읽습니다.
- 검사에서 측정한 표본 snapshot 반영 시간은 약 0.17~0.18ms였습니다(Apple Silicon 개발 기기, Debug, 화면 밖 창). 판정 기준이 아니라 참고값이며 UI 스레드 단일 점유 상한 2ms(아키텍처 §6.3) 안입니다.
- 파일 I/O(설치기 생성 때 `/etc/paths` 읽기, 설치 상태 판정, 설치·제거)는 별도 큐에서 합니다. 검사가 `StellarCommandFileSystemThreadAudit`으로 상태 확인·첫 실행 판정·설치·제거·다시 확인 경로의 메인 스레드 파일 조회가 0인지 확인합니다. 새 프로젝트 패널의 같은 이름 검사만 메인 스레드에서 `stat` 한 번을 합니다.
- 프로젝트 생성·열기·Clone·Git 탐색은 코어가 메인 스레드 밖에서 하고 결과를 snapshot으로 돌려줘야 합니다.

## 13. 검사

`WelcomeUICheck.run() -> Bool`이 화면 밖에서 검사합니다. `Stellar --welcome-ui-check`로 실행하며(`main.swift`), `scripts/macos/smoke.sh`가 다른 검사 뒤에 실행합니다.

- 시작 창 6개 상태(목록, 빈 목록, 읽는 중, 실패, Git 없음, Clone 진행 중)와 가장 긴 내용(쓸 수 없는 Git + 진행 중), Clone sheet, 설치 sheet 13개 단계를 Light, Dark, 대비 높이기 Light·Dark에서 레이아웃합니다.
- 모호한 Auto Layout, 세로로 잘린 여러 줄 레이블, 직접 넣은 반투명 표면이 없는지 확인합니다(`NSScrollView`가 스스로 넣는 표면은 시스템 소유라 제외).
- 왼쪽 열이 safe area 안에 잘리지 않고 들어가는지 확인합니다.
- 접근성 레이블, 행 접근성 자식 0, 기본·취소 키, 버튼에 메뉴와 겹치는 단축키 없음, 메뉴 활성 판정(표본·Git 없음·진행 중; 버튼 selector와 `MainMenuBuilder`가 만든 실제 File 메뉴 항목 모두), 실제 File 메뉴 항목이 응답자 체인으로 시작 창에 도달하는지, 캔버스 명령 메뉴를 켜지 않는지, 어댑터 경로(`availability(of:)`, `run(_:)`), Return·Delete 키 경로, 문맥 메뉴 활성 상태, 상태별 버튼 활성, 작업 식별자 취소(다른 식별자 무시), 이유 코드 문구, 저장 패널 같은 이름 판정, announcement(목록 상태·sheet 단계), 복사 피드백(검사용 이름 pasteboard)을 확인합니다.
- 하드코딩 색: 레이아웃 검사와 함께 모든 외형에서 시스템 catalog 색이 아닌 색을 찾습니다.
- 설치기를 메모리 파일 시스템으로 검사합니다(fixture 이름은 XXIV.5.3~XXIV.5.5 보고서와 같음).
  - 위치 결정 L1~L8: 기본 위치·폴더 생성, PATH에 있는 후보 우선, 공용 폴더 미선택, 쓸 수 있는 위치 없음, 쓸 수 없는 첫 후보 건너뛰기, App Translocation·읽기 전용 볼륨, helper 없음.
  - 이름 결정·설치·제거 I1, N1\~N10, R1\~R6, E1: 새 설치·멱등·제거(되돌리기 후 설치 항목 0), 설치 위치의 남의 파일·링크·폴더, PATH 앞쪽·순서 모르는 폴더의 다른 `stellar`, PATH 뒤쪽 프로그램 알림, 두 이름 모두 막힘(설치 0), 자기 링크 유지, 다른 앱 helper 링크, 설치 뒤 가림, 오래된 링크 다시 연결(상대 경로 대상을 절대 경로로 풀어 보고), 확인과 실행·교체·제거 사이 경합, 제거가 이 앱 helper 링크만 지움(dangling·다른 사본 링크 보존), 파일 시스템 오류 전달. 판정만으로 변경 0(`mutationCount`).
  - PATH 구성: `/etc/paths`, `/etc/paths.d`, 프로세스 `PATH`, 순서 모르는 폴더.
- 실제 파일 시스템 구현은 임시 폴더 안에서만 검사합니다: 설치, `RENAME_SWAP` 다시 연결, 임시 링크 잔여 0, 기대와 다른 항목 교체·삭제 거부, `symlink` EEXIST, 대체 이름 설치·되돌리기.
- 메인 스레드 감시: 상태 확인·첫 실행 판정·sheet 설치·제거·Check Again 경로에서 메인 스레드 파일 조회 0.
- 설치 sheet는 확인 → Install → 작업 큐 → 결과, Remove → 결과, Not Now(파일 변경 0), 대체 이름 제안·설치 결과 이름, 알림 상태(설치 동작 없음), Update 동의 화면의 지금 링크 대상(확인한 사본은 앱 위치, dangling은 대상 경로와 Stellar 사본이라고 하지 않음)까지 확인합니다.
- 실제 앱의 `Contents/Helpers/stellar`에 임시 홈 링크를 설치하고 그 링크로 `version` JSON을 실행합니다. 실제 사용자 PATH의 설치 상태는 읽어서 보고합니다.

## 14. 확정 수치 제안

가이드 12장에 기록할 후보입니다. 근거는 HIG Layout이며 통합 시 측정해 확정합니다. 값은 `WelcomeMetrics` 한곳에 있습니다.

| 항목 | 값 |
|---|---|
| 시작 창 콘텐츠 크기 | 780 × 480pt |
| 왼쪽 동작 열 폭 | 280pt |
| 앱 아이콘 표시 크기 | 64pt |
| 창·sheet 바깥 여백 | 20pt |
| 묶음 사이 간격 | 16pt |
| 묶음 안 행 간격 | 6pt |
| sheet 콘텐츠 폭 | 460pt |

## 15. 코어 연결과 통합 때 남은 일

1. **데이터 어댑터**: `WelcomeDataSource`를 구현하는 코어 어댑터. 최근 목록 저장소, ProjectCreateRequest(XII.6.1), ProjectProbe 후 session 열기(XII.6.2), clone-open pipeline(XII.6.3), Git 탐색·capability(XIV.1.3), Git 명령 취소(XIV.1.4)를 FFI pull 이벤트(`stellar_runtime_poll_event`)와 명령 호출로 연결합니다.
2. **안내 코드 매핑**: 코어 오류 코드 → `WelcomeNotice.Kind`. `gitIdentityRequired`는 지금 안내만 하며, 이름·이메일을 받는 입력 sheet는 XII.6.1 상세 UX에서 정합니다.
3. **앱 연결**: `AppDelegate`가 열린 프로젝트가 없을 때 `WelcomeWindowController`를 띄우고 `presentCommandInstallOfferIfNeeded()`를 부르게 합니다. 프로젝트가 열리면 시작 창을 닫습니다. 현재 앱은 캔버스 창을 바로 엽니다.
4. **메뉴**: `MainMenuBuilder` 메뉴를 앱에 설치합니다. File 메뉴의 New Project…(⌘N), Open Project…(⌘O), Clone Project…는 앱 수준 router가 명시 target이므로, 그 router의 model(통합 어댑터)이 시작 창이 떠 있으면 `WelcomeViewController.availability(of:)`를 `enabledCommands`에 싣고 `run(_:)`으로 넘기게 합니다(7장). 앱 메뉴에 "Set Up the stellar Command…"(`WelcomeStrings.setUpCommandMenuItem`) 항목을 두어 `showStellarCommandSetup(_:)`으로 같은 sheet를 관리 모드로 엽니다(XII.6.5, XXIV.5.6, 가이드 8.1). 메뉴 정의는 Tools 영역(`ShellCommand`)이 소유하므로 그 영역에서 명령을 추가합니다.
5. **smoke 연결**: 완료. `main.swift`의 `--welcome-ui-check` 실행 인자와 `scripts/macos/smoke.sh` 단계로 커밋된 트리에서 실행합니다.
6. **helper**: 앱의 `Contents/Helpers/stellar`가 실행 가능해야 설치 화면에서 링크를 만들 수 있습니다. 정상 번들이 갖춰야 할 빌드·서명 조건은 [XXIV.5.1 보고서](../../task-reports/XXIV.5.1.md)에 기록합니다.
7. **문서 반영**: 가이드 7.3·10.4는 ADR-0005 Accepted 전 문구("이 화면을 정하지 않음", "Open Questions 7 결정 대기")로 남아 있으므로 6장과 계획 XXIV.5.3~XXIV.5.6을 가리키도록 고칩니다. 가이드 12장에 14절 수치를 기록합니다. 시작 창 화면 자체는 전용 WBS가 없습니다(가이드 10.4).
8. **설정 창**: 명령 설치 상태 보기·제거를 설정 창에도 둘지는 설정 창 작업에서 정합니다.
9. **모듈 위치**: 파일 시스템을 바꾸는 설치기(`StellarCommandInstaller.swift`)를 UI 밖 모듈로 옮길지 앱 통합 때 정합니다.

## 16. 한계

- PATH 확인은 셸 설정 파일을 실행하지 않으므로 사용자의 실제 PATH와 다를 수 있습니다. 확인하지 못한 폴더는 "확인하지 못함"으로 표시하고 추가할 줄을 안내합니다.
- PATH 앞쪽 충돌은 순서를 아는 폴더와 흔한 폴더(홈 후보, Homebrew·MacPorts 기본 접두사)만 봅니다. 셸 설정 파일이 그 밖의 폴더를 앞에 붙이고 거기에 다른 `stellar`가 있으면 감지하지 못합니다. 셸 alias·함수·builtin 이름 충돌도 보지 않습니다(셸 실행 필요).
- App Translocation 판정의 경로 문자열 검사는 Apple이 지원하는 방법이 아닙니다. 읽기 전용 볼륨 판정을 함께 쓰지만, macOS 동작이 바뀌면 달라질 수 있습니다.
- dangling 소유 링크(번들이 없는 `<이름>.app/Contents/Helpers/stellar`)는 번들 식별자를 확인할 수 없으므로 모양만으로 Stellar 소유로 봅니다. 다른 앱이 남긴 같은 모양의 링크일 수 있으므로, Update 동의 화면에서 지금 대상과 "더 이상 없음"을 보여 준 뒤에만 바꾸고, Remove Command는 지우지 않습니다. 사용자가 이런 링크를 앱에서 지우는 방법은 없습니다(Finder나 Terminal로 지움).
- 설치 위치를 사용자가 직접 고르는 기능은 없습니다.
- 실제 화면 표시(키 창 기본 버튼 강조, sheet 표시·닫힘, 실제 VoiceOver 읽기)는 화면 밖 검사로 확인하지 않았습니다. 앱 통합 후 실제 앱에서 확인해야 합니다(XXIV.5.6 수용 기준).
- 새 프로젝트 패널의 같은 이름 차단은 AppKit 문서의 호출 순서(`userEnteredFilename`이 바꾸기 확인보다 먼저)에 기대며, 실제 패널에서 바꾸기 확인이 뜨지 않는지는 앱 통합 후 확인해야 합니다. 코어도 대상 폴더가 있으면 `folderAlreadyExists`로 거부해야 합니다.
- 실제 VoiceOver가 announcement를 읽는지는 확인하지 않았습니다(검사는 요청을 보냈는지만 봄).
