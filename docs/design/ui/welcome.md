# 시작 창(Welcome)과 첫 실행 UI 명세

- 상태: 초안(2026-11-14), 코어 연결 전(Stub 데이터)
- 영역: `apps/macos/StellarMac/UI/Welcome/`
- 기준: [macOS 셸 디자인 가이드](../macos-shell-design.md)(이하 "가이드"), 기술 아키텍처 명세(이하 "아키텍처"), Apple Human Interface Guidelines(이하 "HIG")
- WBS 연결: 통합 시 정합니다. 관련 행은 New Project·Open·Clone 명령(계획 XII.6.1~XII.6.3)이고, 시작 창 화면 자체와 `stellar` 명령 설치 화면은 전용 WBS가 없습니다(가이드 10.4).

## 1. 범위

이 영역은 열린 프로젝트가 없을 때 보이는 시작 창과 첫 실행 안내를 맡습니다.

| 구성 | 내용 | 근거 |
|---|---|---|
| 시작 창 | New Project, Open Project, Clone Project 진입점과 최근 프로젝트 목록 | 가이드 7.1, 아키텍처 §25 |
| 빈 상태 | 최근 프로젝트 없음·읽는 중·읽기 실패 | 가이드 7.1, HIG Writing |
| Git 안내 | 시스템 Git이 없거나 쓸 수 없을 때 설치 안내만 표시, 자동 설치 없음 | 아키텍처 §24.1, 가이드 7.2 |
| Clone sheet | 원격 URL, 새 폴더 이름, 상위 폴더 입력 | 계획 XII.6.3, 아키텍처 §11.2 |
| `stellar` 명령 설치 sheet | 첫 실행 시 셸 PATH에 `stellar` 명령을 넣을지 묻고, 설치·제거·결과·오류를 보여 줌 | 제품 책임자 결정(2026-11-14), ADR-0005 결정 7 |

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
| 실패(`failed(detail)`) | "Couldn’t Load Recent Projects" + 코어가 준 설명 + Try Again | Try Again → `reloadRecentProjects` |
| 없어진 폴더(`isMissing`) | `questionmark.folder` 아이콘, 이름은 secondary 색, 경로 앞에 "Not found:" | 열기를 누르면 코어가 `projectMissing` 안내를 돌려줍니다. Show in Finder는 비활성 |

선택 상태와 "없음" 상태는 색에만 기대지 않고 아이콘과 문구를 함께 바꿉니다(가이드 4.2).

### 3.2 Git 상태(아키텍처 §24.1)

| 상태 | 표시 | New·Clone | Open |
|---|---|---|---|
| 탐색 중(`checking`) | 안내 없음 | 켬 | 켬 |
| 사용 가능(`available`) | 안내 없음 | 켬 | 켬 |
| 없음(`notFound`) | ⚠ "Git Isn’t Installed", 설명, Terminal에서 실행할 명령 `xcode-select --install`(선택·복사 가능), Check Again, Locate Git… | 끔 + 도움말 "Requires Git…" | 켬 |
| 쓸 수 없음(`unusable(detail)`) | ⚠ "Git Can’t Be Used", 코어가 준 설명, Check Again, Locate Git… | 끔 | 켬 |

- 설치는 실행하지 않습니다. 명령은 보여 주고 복사만 하게 합니다. 셸은 시스템 Git을 직접 실행하지 않으므로, Git이 없는 Mac에서 시스템 설치 대화상자가 뜨게 하지 않습니다.
- Locate Git…은 탐색 순서 1번(사용자 지정 경로)을 지정하는 경로입니다. 고른 경로를 `useGitExecutable`로 넘기고, 검사는 코어가 합니다.
- New Project는 `git init`, Clone은 `git clone`이 필요하므로 끕니다. Open은 제한 모드 열기(아키텍처 §24.3)가 Git 없이도 가능하므로 켜 둡니다.

### 3.3 진행 중 작업

- 생성·열기·Clone 중에는 왼쪽 열에 "Cloning “이름”…" 같은 제목, 진행 막대(진행률을 모르면 불확정), Cancel(취소 가능할 때만)을 보이고 세 동작 버튼을 끕니다.
- 제목에는 프로젝트 표시 이름만 넣고 Clone URL은 넣지 않습니다(아키텍처 §39: Git URL은 민감값).

### 3.4 안내(alert)

코어가 snapshot에 `notice`를 실으면 창의 sheet로 `NSAlert`을 한 번 띄우고, 닫으면 `dismissNotice(id)`를 보냅니다(가이드 5.1, HIG Alerts). 다른 sheet가 떠 있으면 그 sheet가 닫힌 뒤 띄웁니다.

| 종류 | 제목 | 근거 |
|---|---|---|
| `notStellarProject` | This Folder Isn’t a Stellar Project — `stellar.yaml`이 없어 열 수 없고 기존 저장소를 전환하지 않음 | 아키텍처 §25.2, 가이드 7.2 |
| `projectMissing` | Project Not Found | — |
| `folderAlreadyExists` | A Folder with This Name Already Exists | — |
| `gitIdentityRequired` | Set Your Git Name and Email — 최초 커밋 전에 `user.name`·`user.email` 필요 | 아키텍처 §25.1, 가이드 7.2 |
| `operationFailed(detail)` | The Operation Couldn’t Be Completed + 설명 | — |

문구는 셸이 종류 코드에서 만듭니다. 코어는 문구가 아니라 코드를 보냅니다(지역화 대비).

## 4. 상호작용

| 동작 | 결과 명령 | 비고 |
|---|---|---|
| New Project… | `NSSavePanel`(제목 "New Project", 버튼 "Create", 이름표 "Project Name:")에서 상위 폴더와 새 폴더 이름을 받아 `createProject(parentDirectory:folderName:)` | 아키텍처 §25.1, 계획 XII.6.1. 이미 있는 이름은 패널에서 막습니다. |
| Open Project… | `NSOpenPanel`(폴더만)에서 고른 폴더로 `openProject(directory:)` | 폴더 판정은 코어(계획 XII.6.2) |
| Clone Project… | Clone sheet(5절) → `cloneProject(remoteURL:parentDirectory:folderName:)` | 계획 XII.6.3 |
| 행 더블클릭, Return, Enter | `openRecent(id:)` | 빈 곳 더블클릭은 무시 |
| Delete, Forward Delete | `removeRecent(id:)` | 목록에서만 뺍니다. 폴더는 건드리지 않습니다. |
| 문맥 메뉴 | Open / Show in Finder / Remove from Recents | Show in Finder는 셸이 `NSWorkspace`로 처리 |
| Try Again | `reloadRecentProjects` | |
| Check Again | `recheckGit` | |
| Locate Git… | `NSOpenPanel` → `useGitExecutable(path:)` | |
| Cancel(진행 중) | `cancelActivity` | 계획 XIV.1.4 |

모든 패널은 창 sheet로 띄웁니다. 패널이 없는 상태(창 없음)에서는 명령을 보내지 않습니다.

## 5. Clone sheet

- 입력: Repository URL(`NSTextField`, content type URL), Name(`NSTextField`), Location(경로 표시 + Choose…). 모두 AppKit 네이티브 텍스트 컨트롤입니다(아키텍처 §11.2, 가이드 5.2).
- 양식은 오른쪽 정렬 이름표 열과 남는 폭을 쓰는 입력 열로 배치합니다(HIG Layout).
- 이름은 URL에서 자동으로 채웁니다(`…/design-system.git` → `design-system`, `git@host:team/ds.git` → `ds`). 사용자가 이름을 고치면 더 이상 덮어쓰지 않습니다.
- 검사: URL은 비어 있지 않고 공백이 없어야 하며, 이름은 비어 있지 않고 `/`, `:`, 앞쪽 `.`이 없어야 합니다. 통과하기 전에는 Clone 버튼이 꺼져 있습니다. URL 형식과 접근 가능 여부는 코어·Git이 판단합니다.
- 키: Return = Clone(기본 버튼), Escape = Cancel.
- 안내 문구로 `stellar.yaml`이 있는 저장소만 열 수 있음을 미리 알립니다(아키텍처 §25.2).
- URL은 로그·진단에 남기지 않습니다(아키텍처 §39).

## 6. `stellar` 명령 설치(첫 실행)

### 6.1 결정과 문서 관계

- 제품 책임자 결정(2026-11-14): 명령 이름은 `stellar`(대체 이름 `stellar-cli`)이고, 첫 실행 시 설치 UI를 제공합니다. 다른 생태계와의 이름 충돌은 고려하지 않습니다.
- ADR-0005 결정 7: 앱 번들 `Contents/Helpers/stellar-cli`에 보조 실행 파일을 두고, 초기 릴리스에는 보조 실행 파일과 alias를 넣지 않습니다.
- 가이드 7.3은 ADR-0005 Open Questions 7(PATH 노출 방법)이 정해질 때까지 이 화면을 정하지 않는다고 적고 있습니다. 이 구현은 위 제품 책임자 결정을 따르되, **보조 실행 파일이 없는 빌드에서는 첫 실행 제안을 띄우지 않으므로** 초기 릴리스(ADR-0005 결정 7)에는 노출되지 않습니다. 통합 시 가이드 7.3·10.4와 ADR-0005 Open Questions 7에 이 결정을 반영해야 합니다(이 작업은 기존 문서를 수정하지 않았습니다).

### 6.2 원칙

| 원칙 | 구현 |
|---|---|
| 명시적 동의 | Install을 눌러야만 설치합니다. 첫 실행 sheet의 기본 버튼은 Install(Return), Not Now는 Escape입니다. 답하면(설치·나중에) 다시 묻지 않습니다(`StellarCommandInstallOffered` 사용자 기본값). |
| 권한 상승 없음 | 관리자 인증, 보조 도구 설치, 셸 실행을 쓰지 않습니다. 현재 사용자가 쓸 수 있는 폴더 또는 홈 아래 새로 만들 수 있는 폴더에만 링크를 만듭니다. |
| 남의 파일 보존 | Stellar 소유 링크가 아닌 파일·링크·폴더는 덮어쓰거나 지우지 않습니다. 링크 생성은 `symlink(2)`로 하므로 그 자리에 무엇이 생기면 실패합니다. |
| 되돌릴 수 있음 | Remove Command가 후보 폴더의 Stellar 소유 링크만 지웁니다. 설치 때 만든 폴더(`~/.local/bin` 등)는 다른 도구도 쓸 수 있으므로 남깁니다. |
| 셸 설정 파일 불변 | `~/.zshrc` 같은 파일을 고치지 않습니다. PATH에 없다고 판단되면 추가할 한 줄을 보여 주고 복사만 하게 합니다. |
| 메인 스레드 보호 | 상태 판정과 설치·제거는 별도 큐에서 하고 결과만 메인 스레드에 반영합니다. sheet가 닫히거나 다른 작업이 시작되면 늦게 온 결과는 버립니다(아키텍처 §6.4, 가이드 9장). |

### 6.3 위치 선택

1. 후보 폴더(선호 순): `~/.local/bin`, `~/bin`, `/usr/local/bin`. 홈 아래를 먼저 두어 다른 사용자 계정에 영향을 주지 않습니다.
2. 후보 중 **PATH에 있다고 확인되고** 쓸 수 있는 첫 폴더를 고릅니다.
3. 없으면 홈 아래 첫 후보(`~/.local/bin`)를 고르고 필요하면 만듭니다. 이때 PATH 추가 안내를 보입니다.
4. 홈 아래 폴더도 만들 수 없으면 "No Folder Available"을 보입니다.

PATH 확인은 셸을 실행하지 않고 `/etc/paths`, `/etc/paths.d/*`, 앱 프로세스의 `PATH`만 읽습니다. 사용자의 셸 설정 파일이 더하는 경로는 알 수 없으므로, 목록에 없는 폴더는 "PATH에 없음"이 아니라 "확인하지 못함"으로 표시합니다("Stellar couldn’t confirm that … is on your PATH").

### 6.4 소유 판정과 상태

- **Stellar 소유 링크**: 심볼릭 링크이고, 대상(상대 경로는 링크 폴더 기준으로 풀어서)이 `<이름>.app/Contents/Helpers/stellar-cli` 형식인 것. 앱이 옮겨져 대상이 없어진 링크도 Stellar 소유로 봅니다.
- 상태와 화면:

| 상태 | 제목 | 버튼 |
|---|---|---|
| `ready(plan)` | Install the stellar Command? + 링크 위치·대상, 폴더 생성 예고, PATH 안내 | Not Now(Esc) / **Install**(Return) |
| `needsRepair(plan)` | Update the stellar Command? (다른 사본이나 옮겨진 앱을 가리킴) | Not Now / **Update** |
| `blocked(conflict, fallback?)` | Another stellar Command Exists — 남의 파일은 바꾸지 않음. 대체 이름이 가능하면 제안 | Not Now / **Install as stellar-cli**, 불가하면 **Close** |
| `installed` | The stellar Command Is Installed (+ PATH 안내) | Remove Command(파괴적 표시, 기본 버튼 아님) / **Done** |
| `helperUnavailable` | Command-Line Tool Not Included | **Close** |
| `unsupportedAppLocation` | Move Stellar to Your Applications Folder (App Translocation 또는 읽기 전용 볼륨에서 실행) | **Close** |
| `noWritableLocation` | No Folder Available | **Close** |
| 작업 중 | Installing… / Removing… + 회전 표시 | 없음(Escape 무시) |
| 설치 실패 | The stellar Command Couldn’t Be Installed + 원인(충돌·파일 시스템 오류) | Close / **Try Again** |
| 제거 결과 | The stellar Command Was Removed / Some Links Couldn’t Be Removed | **Done** |

- 실행 직전에 다시 확인합니다. 확인과 실행 사이에 남의 파일이 생기면 `conflict`로 보고하고 아무것도 바꾸지 않습니다.
- 이미 이 앱을 가리키는 링크가 있으면 같은 계획을 다시 실행해도 바꾸지 않습니다(멱등).
- 오래된 Stellar 소유 링크는 같은 폴더의 임시 링크를 만든 뒤 `rename(2)`로 원자적으로 바꿉니다.
- 첫 실행 제안 조건: 아직 답하지 않았고 상태가 `ready`, `needsRepair`, 또는 대체 이름이 있는 `blocked`일 때만. 보조 실행 파일이 없거나 이미 설치됐거나 막다른 상태면 띄우지 않습니다.

## 7. 키보드

| 키 | 동작 | 범위 |
|---|---|---|
| ⌘N | New Project… | 시작 창 |
| ⌘O | Open Project… | 시작 창 |
| Return, Enter | 선택한 최근 프로젝트 열기 | 최근 목록 |
| Delete, Forward Delete | 최근 목록에서 빼기 | 최근 목록 |
| ↑ ↓ | 행 이동 | 최근 목록(시스템 동작) |
| Tab, Shift-Tab | 컨트롤 사이 이동 | 창 전체(`autorecalculatesKeyViewLoop`), Full Keyboard Access로 모든 컨트롤 도달 |
| Return / Escape | 기본 버튼 / 취소 | Clone sheet, 설치 sheet |

- 최근 목록이 있으면 창의 첫 포커스를 목록에 둡니다. 그 밖에는 사용자 조작 없이 포커스를 옮기지 않습니다(가이드 8.2).
- Clone은 자주 쓰는 명령이 아니므로 별도 단축키를 두지 않습니다(가이드 8.1 "커스텀 단축키는 자주 쓰는 앱 고유 명령에만").
- 버튼 action은 first responder action(`newStellarProject:`, `openStellarProject:`, `cloneStellarProject:`)으로도 받을 수 있게 했습니다. 통합 시 File 메뉴 항목을 같은 selector에 연결합니다.

## 8. 접근성(가이드 8장)

- 모든 버튼은 제목이 있고, 아이콘만 있는 복사 버튼은 접근성 레이블과 도움말("Copy Command")을 둡니다.
- 최근 목록 표의 레이블은 "Recent Projects"입니다. 각 행은 하위 뷰를 접근성 요소 하나로 묶고 "이름, 경로[, Not Found]"를 읽습니다.
- 행마다 접근성 사용자 동작(Open, Show in Finder, Remove from Recents)을 두어 문맥 메뉴 없이도 같은 동작을 할 수 있습니다.
- Git 안내와 진행 표시는 제목을 레이블로 가진 group 요소이고, 안의 버튼은 그대로 접근할 수 있습니다.
- 목록 상태 문구와 설치 sheet 제목이 바뀌면 VoiceOver에 값 변경을 알립니다.
- 장식 아이콘(앱 아이콘, 행 아이콘, 안내 아이콘)은 접근성 트리에서 숨깁니다.

## 9. 외형과 시스템 설정

- 색은 시스템 의미 색(label 계층, separator)만 씁니다. 하드코딩한 색, asset catalog 색, 이미지 자산이 없습니다(가이드 4.2). 경고 아이콘은 SF Symbol의 시스템 multicolor 렌더링을 씁니다.
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

- `WelcomeSnapshot`: `generation`(세대 번호, 오래된 snapshot 폐기), `recentProjects`(loading / loaded / failed), `git`(checking / available / notFound / unusable), `activity?`, `notice?`, `defaultParentDirectory?`.
- `WelcomeCommand`: `createProject`, `openProject`, `cloneProject`, `openRecent`, `removeRecent`, `reloadRecentProjects`, `recheckGit`, `useGitExecutable`, `cancelActivity`, `dismissNotice`.
- 경로는 `String`(UTF-8)이라 ADR-0009의 `(ptr, len)` 문자열 규칙으로 옮기기 쉽습니다. 최근 항목 식별자는 코어가 발급한 불투명 문자열입니다.
- 표본 구현 `WelcomeDataSourceStub`은 의도적 placeholder입니다(계획 1.2 규칙 6). 실제 프로젝트를 만들거나 열지 않고, 받은 명령을 기록하며 최근 목록 제거·안내 확인·작업 취소만 흉내 냅니다.

명령 설치는 코어와 무관한 셸 기능이므로 별도 타입입니다.

- `StellarCommandInstaller`(값 타입): `status()`, `install(_:)`, `uninstall()`, `installedLinkPaths()`, `isStellarOwnedLink(atPath:)`.
- `StellarCommandFileSystem` 프로토콜로 파일 시스템을 주입합니다. 실제 구현은 `LocalStellarCommandFileSystem`(`lstat`, `symlink`, `rename`, `unlink`), 검사용은 메모리 구현 `StellarCommandFileSystemStub`입니다.
- `StellarCommandSearchPath.system(fileSystem:environmentPath:)`: 셸 실행 없이 PATH 후보를 모읍니다.
- `WelcomeFirstLaunchPolicy`: 첫 실행 제안 여부를 사용자 기본값에 기록합니다. 프로젝트 파일과 Git에는 넣지 않습니다.

## 12. 성능과 스레드

- snapshot 반영은 바뀐 경우에만 하고, 연속 변경은 다음 런 루프에서 한 번만 반영합니다. 최근 목록은 배열이 바뀔 때만 표를 다시 읽습니다.
- 검사에서 측정한 표본 snapshot 반영 시간은 약 0.17~0.18ms였습니다(Apple Silicon 개발 기기, Debug, 화면 밖 창). 판정 기준이 아니라 참고값이며 UI 스레드 단일 점유 상한 2ms(아키텍처 §6.3) 안입니다.
- 파일 I/O(설치 상태 판정, 설치·제거, `/etc/paths` 읽기)는 별도 큐에서 합니다. 새 프로젝트 패널의 같은 이름 검사만 메인 스레드에서 `stat` 한 번을 합니다.
- 프로젝트 생성·열기·Clone·Git 탐색은 코어가 메인 스레드 밖에서 하고 결과를 snapshot으로 돌려줘야 합니다.

## 13. 검사

`WelcomeUICheck.run() -> Bool`이 화면 밖에서 검사합니다. 통합 시 `--welcome-ui-check` 같은 실행 인자로 `scripts/macos/smoke.sh`에 연결합니다.

- 시작 창 6개 상태(목록, 빈 목록, 읽는 중, 실패, Git 없음, Clone 진행 중)와 가장 긴 내용(쓸 수 없는 Git + 진행 중), Clone sheet, 설치 sheet 13개 단계를 Light, Dark, 대비 높이기 Light·Dark에서 레이아웃합니다.
- 모호한 Auto Layout, 세로로 잘린 여러 줄 레이블, 직접 넣은 반투명 표면이 없는지 확인합니다(`NSScrollView`가 스스로 넣는 표면은 시스템 소유라 제외).
- 왼쪽 열이 safe area 안에 잘리지 않고 들어가는지 확인합니다.
- 접근성 레이블, 기본·취소 키, ⌘N·⌘O, Return·Delete 키 경로, 문맥 메뉴 활성 상태, 상태별 버튼 활성을 확인합니다.
- 설치기를 메모리 파일 시스템으로 검사합니다: 보조 실행 파일 없음, App Translocation, 읽기 전용 볼륨, 새 설치·멱등·제거, 남의 일반 파일·링크·폴더 보존, 대체 이름, 상대·dangling 소유 링크 다시 연결, 확인과 실행 사이 경합, PATH 우선 위치 선택, 쓸 수 있는 위치 없음, 파일 시스템 오류 전달, PATH 구성.
- 설치 sheet는 Install → 작업 큐 → 결과, Remove → 결과까지 실제 비동기 경로로 확인합니다.
- 실제 앱 번들의 설치 상태는 환경마다 다르므로 보고만 합니다. 현재 Debug 빌드는 보조 실행 파일이 없어 `helperUnavailable`입니다.

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
4. **메뉴**: File 메뉴의 New Project…(⌘N), Open Project…(⌘O), Clone Project…를 같은 selector에 연결하고, 앱 메뉴에 명령 설치·제거 항목(예: "Set Up “stellar” Command…")을 두어 같은 sheet를 관리 모드로 엽니다(XII.6.5, 가이드 8.1).
5. **smoke 연결**: `main.swift`에 검사 실행 인자를 추가하고 `scripts/macos/smoke.sh`에서 실행합니다.
6. **보조 실행 파일**: 앱 번들 `Contents/Helpers/stellar-cli` 배치와 서명(ADR-0005 결정 7, 계획 XVI.9.3)이 생기기 전에는 설치 화면이 `helperUnavailable`이고 첫 실행 제안은 뜨지 않습니다.
7. **문서 반영**: 가이드 7.3·10.4와 ADR-0005 Open Questions 7에 제품 책임자 결정(첫 실행 설치 UI, 명령 이름)을 반영하고, 가이드 12장에 14절 수치를 기록합니다. WBS가 없는 시작 창·명령 설치 화면은 계획 개정에서 행을 추가합니다(가이드 10.4).
8. **설정 창**: 명령 설치 상태 보기·제거를 설정 창에도 둘지는 설정 창 작업에서 정합니다.

## 16. 한계

- PATH 확인은 셸 설정 파일을 읽지 않으므로 사용자의 실제 PATH와 다를 수 있습니다. 대신 "확인하지 못함"으로 표시하고 추가할 줄을 안내합니다.
- 설치 위치를 사용자가 직접 고르는 기능은 없습니다.
- 더 앞선 PATH 폴더에 다른 `stellar` 명령이 있어 Stellar 링크를 가리는 경우는 감지하지 않습니다(제품 책임자 결정: 다른 생태계와의 이름 충돌은 고려하지 않음).
- 실제 화면 표시(키 창 기본 버튼 강조, sheet 표시·닫힘, 실제 VoiceOver 읽기)는 화면 밖 검사로 확인하지 않았습니다. 통합 후 실제 앱에서 확인해야 합니다.
- 새 프로젝트 패널은 이미 있는 이름을 `panel(_:validate:)`에서 막습니다. 시스템이 띄우는 "바꾸기" 확인 대화상자와 이 검사의 순서는 실제 앱에서 확인하지 않았습니다. 코어도 대상 폴더가 있으면 `folderAlreadyExists`로 거부해야 합니다.
