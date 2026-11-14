# Git 패널 UI 명세 (git_panel)

- 상태: 초안(2026-11-14). 코어 연결 전 Stub 단계입니다.
- 범위: 계획 XIV.13.5의 macOS 최소 Git 패널 중 셸 UI 부분. 코어 API(XIV.13.1~XIV.13.4)와 연결은 남은 일입니다(11장).
- 코드: `apps/macos/StellarMac/UI/Git/`
- 기준: `docs/design/macos-shell-design.md`(이하 "가이드"), `docs/PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md`(이하 "아키텍처"), Apple HIG.

## 1. 범위

초기 Git UI 범위만 제공합니다. [아키텍처 §24.4] [가이드 6.4] [계획 XIV.13.5]

| 포함 | 근거 |
|---|---|
| Status, Semantic Diff(문서 → 객체 → 변경 묶음 목록과 변경 상세) | 아키텍처 §26.1, §26.2 |
| 객체·속성 단위 Stage·Unstage(체크박스) | 아키텍처 §26.3 |
| Commit(`NSTextView` 메시지) | 아키텍처 §11.2, 계획 XIV.8 |
| Push(이미 등록된 remote, 강제 Push 없음) | 아키텍처 §25.3, 계획 XIV.9.5 |
| Semantic Conflict 해결(항목별 Base·Ours·Theirs) | 아키텍처 §27.1, §27.2, 계획 XIV.12 |

넣지 않은 것: Branch·Switch, Fetch·Pull, Merge·Rebase, Stash, Worktree 관리, Remote 관리, 강제 Push. 명령 enum(`GitPanelCommand`)에도 해당 case가 없습니다. merge·rebase continue와 commit 자동 실행도 하지 않습니다. [아키텍처 §24.4, §27.2] [계획 XIV.12.5, XIV.13.7]

## 2. 위치

- 패널은 독립 `NSViewController`(`GitPanelViewController`)입니다. 사이드바 구획과 인스펙터 탭 중 어디에 둘지는 열린 결정 O4이므로 둘 다 넣을 수 있게 만들었습니다. 새 창 계층을 만들지 않습니다. [가이드 2.3, 6.4, 11장 O4]
- 루트 뷰는 safe area 안에 배치해 full-size content 창의 툴바 아래로 들어가지 않습니다. [가이드 2.1]
- 검증한 기본 크기는 폭 320pt, 높이 760pt입니다(일반 사이드바·인스펙터 폭 가정).

## 3. 화면 구성

위에서 아래로 다음 순서입니다. 숨긴 요소는 `isHidden`으로 감춰 접근성 트리에서도 빠집니다.

1. 머리 줄: 브랜치 아이콘·이름, 진행 중인 Git 작업 표시(Merging 등), 오른쪽에 Push·Pull 대기 수(위·아래 화살표 + 숫자), 새로고침 아이콘 버튼.
2. 요약: "N staged, M unstaged".
3. 화면 전환: `NSSegmentedControl`(Changes | Conflicts (N)).
4. 상태 배너 구획: 저장 대기 편집, semantic status 오류, 필수 동반 변경 확인(창이 보이지 않을 때), 진행 중 작업, 마지막 결과(실패 또는 Commit·Push·해결 적용 성공).
5. Changes 화면
   - 변경 목록(`NSOutlineView`, 가상화). 문서 → 객체 → 변경 3단 계층이며 펼침 표시와 들여쓰기는 outline이 시스템 값으로 그립니다. 행 = Stage 체크박스 · 종류 아이콘 · 제목 · 보조 문구(값 변화 요약 등). 처음에는 문서 행만 보이고(모두 접힘), 펼침은 사용자가 정합니다. [HIG Outline views] [가이드 1.2 원칙 3, 6.2]
   - 상세(Details): Kind, Object, Property(고정폭), HEAD, Staged, Working, Requires, Status. 값이 없는 행은 숨깁니다. HEAD·Index·Working 세 값을 모두 보존해 표시합니다. [계획 XIV.4.2]
   - 커밋: "Commit Message" label, 여러 줄 `NSTextView`, Commit 버튼(이 문맥의 유일한 prominent 버튼), 비활성 이유 또는 "Commit N changes to main." 안내, identity가 없으면 "Set Up Git Identity…" 버튼.
   - Push: Push 버튼과 remote 선택 팝업, 그 아래 안내("2 commits to push to origin/main." 또는 비활성 이유).
6. Conflicts 화면
   - conflict 목록(`NSTableView`, 가상화, 두 줄 행). 1줄: 종류 아이콘·제목·종류 문구. 2줄: Base | Ours | Theirs `NSSegmentedControl`. 허용하지 않는 선택은 비활성 segment로 둡니다(예: 삭제 대 수정은 Base 불가). Stellar 파일이 아닌 raw conflict는 선택 대신 "Resolve this file in Terminal or another tool."을 보여 줍니다. [계획 XIV.11.4, XIV.12.1]
   - 상세: Kind, Path, Base, Ours, Theirs, Choice와 "Show on Canvas" 버튼(캔버스에서 대상 보기). [아키텍처 §27.2]
   - Apply Resolutions 버튼(prominent), 비활성 이유, 진행 중 merge가 있으면 "After applying, continue the merge or rebase in Terminal." [계획 XIV.12.5]

배경·재질: 패널은 자체 배경, 반투명 표면, 직접 쓰는 glass를 두지 않습니다. 목록·입력·버튼은 표준 AppKit 컨트롤이고 재질은 시스템(사이드바·인스펙터 pane)이 정합니다. [가이드 1.2 원칙 1·3, 3.2]

## 4. 상태

| 상태 | 조건(코어 값) | 표시 |
|---|---|---|
| 로딩 | 첫 `.status` 이벤트 전 | 목록 자리에 시스템 진행 표시 + "Loading Git status…", 머리 줄 "Loading…" |
| 빈 상태(변경 없음) | `availability == .ready`, 변경 행 0 | 아이콘 + "No Changes" + 설명 + "Refresh Status" 버튼 [가이드 7.1] |
| conflict 없음 | conflict 행 0 | "No Conflicts" + 설명 + "Refresh Status" |
| 제한 모드 | `.restricted` | 목록 자리에 이유와 "Trust Project…" 버튼(host 동작). Commit·Push·적용 비활성. 브랜치는 `.unknown`이면 "Source Control"로 표시 [아키텍처 §24.3] [가이드 6.4] |
| Git 없음 | `.gitMissing` | 이유와 "Show Git Installation Help…"(host 동작). 자동 설치 없음 [아키텍처 §24.1] |
| 저장소 아님 | `.notRepository` | 이유만 표시(다음 동작은 프로젝트 흐름 WBS의 몫) |
| semantic status 오류 | `semanticStatusError != nil` | 경고 배너 + "Refresh Status". raw 변경 행은 목록에 계속 표시 [계획 XIV.4.5] |
| 저장 대기 편집 | `pendingAutosave` | 배너 "Some edits aren't saved yet…" + "Save Now" / "Use Saved Version". 자동 저장하지 않음 [계획 XIV.2.8] |
| 작업 진행 | `.operationProgress` 수신 ~ `.operationFinished` | 진행 배너(시스템 진행 표시 + 문구, 취소 가능하면 "Cancel"). 목록 조작과 Commit·Push·적용 비활성. Commit·Push·해결 적용·저장의 시작은 VoiceOver에 알림(보통 우선순위), 짧은 Stage·Unstage·새로고침은 알리지 않음 |
| 실패 | `.operationFinished(.failed)` | 오류 배너(빨간 아이콘 + Git 결과 요약 + "Dismiss"). 아이콘 모양과 문구로도 전달 [가이드 4.2]. 같은 문구를 VoiceOver에 높은 우선순위로 알림 |
| 사용자 동작 거부 | 사용자가 시작한 Commit·Push·해결 적용·필수 동반 변경 포함·저장 대기 선택(Save Now, Use Saved Version)의 `.commandRejected` | 실패와 같은 오류 배너에 "실행하지 않았다"는 문장과 이유 문장(다른 작업 진행 중, 그 사이 저장소가 바뀜, Git 사용 불가). 같은 문구를 VoiceOver에 높은 우선순위로 알림. 새 작업 결과가 오거나 "Dismiss"하면 사라짐. 미리 보여 준 값만 되돌리면 되는 거부(Stage 체크박스, 해결 선택, 페이지·상세 요청)와 취소 응답의 거부는 알리지 않음 |
| 성공 | Commit·Push·해결 적용 성공 | 결과 배너 + "Dismiss", 같은 문구를 VoiceOver에 알림. Stage·Unstage 성공은 목록 변화로 충분하므로 배너·알림 없음 |
| 필수 동반 변경 | `.dependencyPrompt` | 창이 보이면 `NSAlert` sheet, 아니면 배너. "Include N Related Changes" / "Cancel". 자동 포함 없음 [계획 XIV.5.3, XIV.7.2] [가이드 5.1] |

## 5. 상호작용

- Stage 체크박스: 꺼짐 = unstaged, 켜짐 = staged, 혼합(mixed) = 부분 staged(HEAD ≠ Index ≠ Working) 또는 묶음의 일부만 staged. 누르면 staged는 Unstage, 그 밖은 Stage합니다. 결과가 올 때까지 목표 상태를 먼저 보여 주고, 새 세대 페이지가 실제 값을 덮어씁니다.
- 묶음(문서·객체) 행의 체크박스는 접혀 있어도 하위 변경 전체에 적용합니다.
- 여러 행 선택 후 Space 또는 문맥 메뉴(Stage / Unstage)로 한꺼번에 적용합니다. 대상은 "부모 노드 + 자식 순번 범위" 목록으로 보내고, 조상이 함께 선택된 행은 뺍니다(묶음은 하위 전체). 전체 선택(⌘A)은 최상위 범위 하나로 코어에 전달합니다.
- 펼침: `NSOutlineView`의 보기 상태입니다. 코어에 명령을 보내지 않습니다. 펼친 묶음은 노드 ID로 기억해, 구조가 바뀐 뒤(커밋으로 노드가 빠짐) 행이 도착하면 다시 펼칩니다. 같은 이유로 선택도 노드 ID로 되살립니다. Option-클릭은 하위까지 펼칩니다(시스템 기본).
  - 기억 정리: 구조가 바뀔 때 나가는 구조에서 행이 온 노드의 펼침은 남기고, 행이 오지 않은 펼침(커밋으로 빠진 노드, 접힌 조상 아래 노드, 아직 복원 전인 노드)은 한 번 더 넘긴 뒤 두 구조 연속으로 오지 않으면 버립니다. 그래서 빠진 노드의 ID가 세션 동안 쌓이지 않고(최근 두 구조에서 만든 항목 수 이하), 복원 도중 구조가 한 번 더 바뀌어도 펼침을 잃지 않습니다. 접힌 조상 아래에서 펼쳐 둔 묶음은 그 조상을 펼치지 않은 채 구조가 두 번 바뀌면 접힌 상태로 돌아옵니다.
- 선택 → 상세: 선택이 연속으로 바뀌면 다음 런 루프에서 한 번만 요청합니다. 새 세대 상세가 올 때까지 이전 값을 유지해 깜빡임과 재배치를 막습니다. [가이드 9장]
- Commit: 비활성 이유가 없을 때만 실행합니다. 보낸 뒤 결과(또는 코어 거부)가 올 때까지 메시지를 편집할 수 없고 Commit도 "작업 진행 중"으로 비활성입니다. 그래서 성공 후 지우는 문장은 보낸 문장과 같습니다. 성공하면 메시지를 텍스트 시스템 경로로 비워 Undo로 되살릴 수 있습니다(검사로 확인). 메시지 초안은 메모리에만 있고 프로젝트에 저장하지 않습니다.
- Push: 팝업에서 고른 remote로 Push합니다. upstream이 없으면 첫 Push 안내를 보여 줍니다. 강제 Push 선택지는 없습니다.
- Conflict: 행의 segment, 문맥 메뉴(Use Base / Use Ours / Use Theirs, Show on Canvas), 키보드로 고릅니다. 선택은 candidate snapshot에만 기록되고, Apply Resolutions가 검증·저장·`git add`를 요청합니다. [계획 XIV.12.2~XIV.12.4]

### 5.1 비활성 이유

버튼 아래 문구와 `accessibilityHelp`에 첫 번째 이유를 표시합니다. 순서는 아래와 같습니다(`GitPanelRules`).

| 버튼 | 이유(우선순위 순) |
|---|---|
| Commit | 상태 로딩 중 → Git 사용 불가 / 제한 모드 → 작업 진행 중 → identity 없음 → unmerged conflict 남음 → Stage한 변경 없음 → staged snapshot 무효 → 메시지 비어 있음 |
| Push | 상태 로딩 중 → Git 사용 불가 / 제한 모드 → 작업 진행 중 → remote 없음(터미널 안내) → detached HEAD → Push할 커밋 없음 |
| Apply Resolutions | 상태 로딩 중 → Git 사용 불가 / 제한 모드 → 작업 진행 중 → semantic conflict 없음 → 미선택 conflict 남음 |

셸 규칙은 안내용이며 최종 판정은 코어가 합니다(예: 커밋 전 staged snapshot 재검증, 계획 XIV.8.1).

## 6. 키보드

| 키 | 동작 | 범위 |
|---|---|---|
| Space | 선택 행 Stage 토글(마지막 선택 행 상태 기준) | 변경 목록 |
| ← / → | 선택한 묶음 행 접기 / 펼치기, 펼쳐지지 않은 행에서 ←는 부모로 이동(NSOutlineView 기본) | 변경 목록 |
| B / O / T | 선택 conflict에 Base / Ours / Theirs | conflict 목록 |
| ⌘↩ | Commit(메시지 입력 중에도 동작). 패널 안에 포커스가 있을 때만 패널 루트 뷰(`GitPanelRootView`)가 처리합니다. 버튼 key equivalent는 창 전체(캔버스·다른 패널)에서 발동하므로 두지 않습니다 | Changes 화면, 패널 안 포커스 |
| Tab / ⇧Tab | 커밋 메시지에서 다음·이전 컨트롤로 이동. 탭 문자는 ⌥Tab | 커밋 메시지 |

- 수식키 없는 한 글자 키는 메뉴 key equivalent가 아니라 목록 `keyDown`에서만 처리하므로 텍스트 입력 중에는 발동하지 않습니다. [가이드 8.2]
- 목록은 type select를 끕니다(100만 행을 훑지 않게).
- 포커스 순서: 패널은 `nextKeyView`를 직접 잇지 않습니다(닫힌 고리를 만들지 않음). 셸 창은 `autorecalculatesKeyViewLoop`로 시스템이 위치 순서로 고리를 계산하므로, 머리 줄(새로고침) → 화면 전환 → 보이는 배너의 버튼 → 목록 또는 빈 상태 동작(예: Trust Project…) → 상세 값(선택 가능한 텍스트) → (Changes) 메시지 → Commit → identity → Push → remote / (Conflicts) Show on Canvas → Apply 순서로 닿고, 마지막 컨트롤 다음에는 패널 밖의 다음 뷰로 나갑니다. 숨긴 화면·배너는 건너뜁니다. [가이드 8.2]
- 버튼에서 실제로 멈추는지는 시스템 "키보드 탐색"(Full Keyboard Access) 설정을 따릅니다. 켜면 위 모든 컨트롤에 닿습니다.
- 패널을 셸 창이 아닌 다른 창에 넣을 때도 그 창에 `autorecalculatesKeyViewLoop`를 켭니다. 배너 버튼처럼 나중에 계층에 들어오는 컨트롤이 고리에 들어가려면 필요합니다.
- 목록 행 안 체크박스는 Tab 순서에 넣지 않습니다. 키보드는 Space·문맥 메뉴, VoiceOver는 행 custom action으로 같은 동작에 닿습니다.
- Commit·Push·Apply는 통합 시 메뉴 막대 명령으로도 등록해야 합니다(11장). [가이드 6.1, 8.1]

## 7. 접근성과 외형

- 루트 뷰는 "Source Control" 그룹입니다. 목록(Changes, Conflicts), 커밋 메시지, remote 팝업, 화면 전환에 접근성 레이블이 있습니다.
- 아이콘만 있는 새로고침 버튼은 접근성 레이블과 도움말 태그를 함께 둡니다. [가이드 5.4, 8.1]
- 변경 행은 하나의 접근성 요소입니다. [가이드 8.3] 셀 레이블이 "제목, 보조 문구, 종류(Document·Object·Property Changed 등), Stage 상태"를 한 번에 읽고, Stage·Unstage는 셀의 custom action입니다. 체크박스·아이콘·두 label은 view와 cell 모두 접근성 요소에서 뺍니다(NSControl의 접근성은 cell이 노출하므로 둘 다). 체크박스는 마우스용이며 Tab 순서에 넣지 않습니다(`refusesFirstResponder`). 키보드는 Space·문맥 메뉴로 같은 동작에 닿습니다.
- 계층 수준·펼침·선택은 `NSOutlineView`가 행 요소로 노출합니다. 2026-11-14 이 Mac에서 legacy 속성 API로 한 번 읽어 확인한 트리: `AXRow`(AXDisclosureLevel 0~2, AXDisclosing, AXSelected) → `AXCell`(AXDescription = 셀 레이블, custom action "Stage"/"Unstage") → `AXDisclosureTriangle`. 이 확인은 deprecated API를 써서 검사 코드에는 넣지 않았습니다(12장).
- 선택 강조(키 창의 강조 색 배경)에서는 종류 아이콘 tint를 비워 시스템 대비 색으로 그립니다.
- conflict segment는 "Resolution for <제목>", segment별 도움말 태그에 각 쪽 값을 둡니다.
- Push·Pull 화살표 아이콘은 접근성 요소에서 빼고, 두 숫자 label이 각각 "2 to push", "0 to pull"로 읽힙니다(같은 문장을 두 번 읽지 않음). 도움말 태그는 "2 to push, 0 to pull" 전체 문장입니다.
- 상태 알림: 배너가 새로 나타나거나 작업 결과가 오면 배너와 같은 문구를 `NSAccessibility` announcement로 보냅니다. 오류·사용자 동작 거부·semantic status 오류·필수 동반 변경 확인(배너일 때)은 높은 우선순위, 저장 대기·진행·성공은 보통 우선순위입니다. sheet로 묻는 필수 동반 변경 확인은 시스템이 읽으므로 따로 알리지 않습니다. [가이드 8.3]
- 비활성 버튼은 이유를 `accessibilityHelp`로 읽게 합니다.
- 상태는 색에만 의존하지 않습니다(체크박스 모양, 아이콘 모양, 문구). [가이드 4.2]
- 색은 시스템 의미 색만 씁니다(label 계층, `systemOrange`·`systemRed` 상태 아이콘). 하드코딩 RGB가 없습니다. [가이드 4.2]
- 글꼴은 텍스트 스타일(body, callout, headline)과 시스템 고정폭(속성 경로·conflict 경로)만 씁니다. [가이드 4.1]
- 셸 문구(버튼, 안내, 비활성 이유, 아이콘 접근성 설명, 확인 sheet 제목)는 `GitPanelStrings` 한곳에 두고 사이드바·시작 창과 같이 `String(localized:comment:)` 지역화 키로 만듭니다. 문장 조각을 이어 붙이지 않고 문장 단위로 둡니다(예: Stage·Unstage 필수 동반 변경 문장을 따로 둠). 코어가 만든 표시 문자열(값 요약, 경로, Git 결과 요약)은 코어가 지역화합니다.
- Light·Dark: 시스템 외형을 따르며 두 외형에서 offscreen 렌더를 눈으로 확인했습니다.
- 대비 높이기: 보조 문구와 보조 색 상태 아이콘(머리 줄 브랜치·Push·Pull 아이콘, 진행·결과 배너 아이콘, 빈 상태 아이콘, 목록 행 종류 아이콘)을 `secondaryLabelColor`에서 `labelColor`로 올립니다. 주황·빨강 시스템 색 아이콘은 시스템이 대비 높이기 변형을 그리므로 그대로 둡니다. 투명도 줄이기: 패널은 직접 그리는 반투명 표면이 없어 바꿀 대상이 없습니다. 동작 줄이기: 새로 나타난 배너의 페이드 시간을 0으로 합니다. 실행 중 변경은 `NSWorkspace` 접근성 표시 옵션 알림으로 반영합니다. [가이드 8.4, 8.5]
- 모션: 배너가 나타나고 사라질 때 배치는 즉시 바꾸고, 새로 나타난 배너만 0.2초 페이드합니다. 배치 변경 자체를 애니메이션 묶음 안에서 강제 배치하면 패널 전체의 frame 변경이 애니메이션이 되어 화면에 보이는 창에서 한 번에 약 19~20ms를 점유했습니다(2026-11-14 Debug 측정, 같은 변경의 즉시 배치는 1ms 미만). [가이드 1.2 원칙 6, 8.5]

## 8. 성능과 가상화

- 변경 목록은 부모 노드마다 자식 순번을 페이지로 요청합니다(`loadChangePage(generation, parent, children)`). 페이지 256행, 메모리 상한은 모든 부모를 합쳐 32페이지(LRU). 보이는 행의 페이지만 요청합니다. conflict 목록은 평면 `NSTableView`로 행 범위 페이지를 씁니다. [계획 XIV.13.1] [가이드 6.4, 9장]
- outline 항목은 "부모 항목 + 순번"만 가진 작은 객체이고 행 값은 캐시에 있습니다. NSOutlineView는 보이는 최상위 항목만 요청하지만, 펼친 묶음의 바로 아래 자식 항목은 한 번에 만듭니다. 따라서 펼치기 비용은 그 묶음의 자식 수에, 행 값 요청은 화면 행 수에 비례합니다. 행 높이는 고정(셀 내용에서 계산)입니다.
- 세대만 바뀌면(구조 번호 같음) 항목·펼침·선택을 유지하고 보이는 셀만 다시 구성합니다. 직전 페이지는 새 페이지가 올 때까지 stale 값으로 보여 줍니다(조작 불가). 구조 번호가 바뀌면 캐시와 항목을 버리고 `reloadData()`로 다시 읽은 뒤 펼침·선택을 노드 ID로 복원합니다.
- 펼침·선택 복원은 이벤트 드레인과 셀 구성(outline layout) 도중에 하지 않고 다음 런 루프에서 하며, 한 런 루프에 펼침 하나만 합니다. NSOutlineView의 펼침 한 번은 AppKit 내부 작업이 수 ms이기 때문입니다.
- 측정(2026-11-14, 이 개발 Mac, offscreen 창): `NSOutlineView.expandItem` 한 번 약 3\~9ms(별도 -O 시험 프로그램에서 자식 100개 펼침 6\~8.5ms, 접기 약 3ms; 앱 Debug 빌드의 복원 펼침 5.8\~8.5ms). 구조 변경 뒤 `reloadData()` 약 2.8ms(행 13개 표시 중). 처음 채울 때 `reloadData()`는 약 0.1ms. 이 값들은 AppKit 내부 비용이며 아키텍처 §6.3 단일 점유 2ms를 넘습니다. 이 측정의 펼침·접기는 §6.3 단일 인터랙션 상한 10ms 안이었지만, 화면에 새 행이 많이 생기는 사용자 펼침은 그 상한을 넘습니다(아래 사용자 펼침 측정). 기준 하드웨어 Release 측정은 하지 않았습니다(11장).
- 메인 스레드 점유는 런 루프 차례 단위로 나눕니다. [아키텍처 §6.3]
  - 이벤트 처리: 한 차례에 1.0ms(`eventBudget`, 잠정값)까지 하고 남은 이벤트는 다음 차례로 넘깁니다.
  - UI 반영(상태·배너·목록 세대·conflict 상세): 이벤트 처리가 0.5ms(`inlineUIThreshold`, 잠정값) 안에 끝났을 때만 같은 차례에서 하고, 아니면 다음 차례로 넘깁니다. 상태 반영은 묶어서 구획마다 제약을 한 번만 다시 만듭니다.
  - 변경 상세와 outline 구조 다시 읽기(`reloadData()`), 펼침·선택 복원은 각각 다음 차례에서 합니다.
  - `GitPanelViewController.Diagnostics`(드레인·UI 반영·상세)와 `GitChangeListController`(미룬 outline 다시 읽기, 펼침·선택 복원)가 차례 종류별 최대 점유를 기록하고 검사가 보고합니다. 복원 차례는 outline 구조를 바꾼 횟수(규칙상 차례당 1)도 기록합니다.
  - 사용자 펼침(펼침 표시 클릭, →)은 패널이 나눈 차례가 아니라 AppKit이 한 번에 처리하는 인터랙션입니다. 검사는 `expandItem`을 직접 불러 그 시간(뒤따르는 layout 포함)을 따로 잽니다.
- 측정(2026-11-14, Debug 빌드, 이 개발 Mac, 검사 6회): 100만 change 시나리오(문서 1,000 × 객체 100 × 변경 10)에서 문서 2개와 그 안의 객체 2개를 펼치고 끝·처음으로 스크롤했을 때 페이지 요청 8회, 메모리 페이지 8개, 행 뷰 33개, 차례 최대 약 1.0\~1.3ms. 일반 시나리오의 차례 최대는 프로세스 첫 상태 반영(첫 셀·글꼴·심볼 생성) 약 2.2\~3.1ms와 커밋 뒤 outline 다시 읽기 약 2.2\~3.0ms입니다. 화면에 보이는 창(화면 밖 좌표, 배너 페이드 경로 실행)에서는 차례 최대 약 0.9\~1.9ms였습니다. 첫 상태 반영과 outline 다시 읽기는 §6.3 단일 점유 2ms를 넘습니다(AppKit 내부 비용 포함). 드레인 밖의 AppKit display 시간은 측정하지 않았고, 기준 하드웨어 Release 측정이 아닙니다.
- 측정(2026-11-14, Debug 빌드, 이 개발 Mac, offscreen 창, 검사 여러 회): 펼침·선택 복원 차례 최대(일반 시나리오, 커밋 뒤 복원 펼침 포함) 약 9.4\~11.9ms. 사용자 펼침 한 번은 100만 change의 가운데 문서(자식 100개, 화면에 새 행 16개) 약 34\~46ms, 끝 문서(화면에 새로 보이는 행 없음) 약 0.5ms, wide 문서(자식 2,000개) 약 39\~47ms, 일반 시나리오 문서·객체(새 행 3\~4개) 약 6\~10ms. 임시 계측으로 한 번 나눠 본 결과 가운데 문서 펼침에서 새 셀 16개 생성(`GitChangeCellView` 초기화, 재사용할 셀 없음)이 약 16ms, 셀 구성은 0.4ms 미만, 나머지는 AppKit 펼침 작업이었습니다. 따라서 펼치기 비용은 자식 수보다 화면에 새로 보이는 행 수의 영향이 크고, 이 Mac Debug에서는 §6.3 단일 점유 2ms뿐 아니라 통제 가능한 단일 인터랙션 상한 10ms도 넘습니다(11장).
- 간격 상수(바깥 여백 12, 행 간격 6, 줄 안 간격 4, 행 위아래 여백 3, 단위 pt)는 잠정값입니다. 계층 들여쓰기는 NSOutlineView 시스템 값입니다. 구획은 묶음 간격 대신 구분선과 행 간격으로 나눕니다. 가이드 4.4는 모든 패널이 공유하는 상수를 요구하므로 통합 시 공통 상수로 옮기고 가이드 12장에 기록해야 합니다.

## 9. SF Symbols

이미지 asset 없이 이름으로만 씁니다. 검사가 모든 이름의 존재를 확인합니다.

| 용도 | 이름 |
|---|---|
| 브랜치, Push/Pull 대기, 새로고침 | `arrow.triangle.branch`, `arrow.up`, `arrow.down`, `arrow.clockwise` |
| 문서·객체 묶음 | `doc.text`, `square.on.square.dashed` |
| 변경 종류 | `plus.square`(객체 추가), `minus.square`(객체 삭제), `arrow.up.and.down.and.arrow.left.and.right`(이동), `arrow.up.arrow.down`(재정렬), `slider.horizontal.3`(속성), `tag`(토큰 추가·변경), `tag.slash`(토큰 삭제), `puzzlepiece`(컴포넌트 인터페이스), `photo`(에셋 참조), `doc`(Stellar 외 파일) |
| 상태 | `lock.shield`(제한 모드), `exclamationmark.triangle`(경고·semantic conflict), `xmark.octagon`(오류), `info.circle`(진행·결과), `checkmark.circle`(빈 상태·성공), `square.and.arrow.down`(저장 대기), `link`(동반 변경) |
| 기타 | `scope`(캔버스에서 보기) |

## 10. 코어 경계(protocol)

코어가 없으므로 셸이 필요한 데이터·동작을 protocol로 정의했습니다. Rust 타입을 노출하지 않는 순수 Swift 값 snapshot이 들어오고, 명령이 나갑니다. [아키텍처 §8.2] [ADR-0009]

```swift
@MainActor protocol GitPanelDataSource: AnyObject {
    var onEventsAvailable: (() -> Void)? { get set }  // 셸 이벤트 펌프가 메인 스레드에서 호출
    func send(_ command: GitPanelCommand)              // 즉시 반환, Git 작업은 메인 스레드 밖
    func pollEvent() -> GitPanelEvent?                 // FIFO pull, 비면 nil
}

@MainActor protocol GitPanelHostActions: AnyObject {  // 코어가 아닌 다른 셸 화면
    func gitPanelRequestsProjectTrust()
    func gitPanelRequestsIdentitySetup()
    func gitPanelRequestsGitInstallHelp()
}
```

- 명령(`GitPanelCommand`): `refreshStatus`, `loadChangePage(generation, parent: GitChangeNodeID?, children: Range)`, `loadChangeDetail(generation, node)`, `setStaged(generation, targets: [GitChangeRange], staged)`, `answerDependencyPrompt(requestID, includeRequired)`, `commit(message)`, `push(remote)`, `loadConflictPage(generation, rows)`, `chooseResolution(generation, rows, choice)`, `applyResolutions`, `revealConflict(generation, row)`, `resolvePendingAutosave(saveFirst)`, `cancelOperation(id)`. 펼침 명령은 없습니다(펼침은 셸 보기 상태).
- 이벤트(`GitPanelEvent`): `status(GitStatusSnapshot)`, `changePage`, `changeDetail`, `conflictPage`, `dependencyPrompt`, `operationProgress`, `operationFinished`, `commandRejected(GitCommandRejection)`.
- 값 타입: `GitStatusSnapshot`(가용성, HEAD, upstream, remote 이름, identity 여부, staged·unstaged 수, merge 상태, conflict 수, staged snapshot 문제, semantic status 오류, 저장 대기 여부, 변경 계층 정보 `GitChangeTreeInfo(generation, structure, documentCount)`, conflict 목록의 세대·행 수), `GitChangeRow`(노드 ID, 단계, 문구, 종류, Stage 상태, 자식 수, 하위 변경 수), `GitChangePage`(세대, 부모, 시작 순번, 행), `GitChangeRange`(부모, 자식 순번 `IndexSet`), `GitChangeDetail`, `GitConflictRow`, `GitOperationProgress`, `GitOperationResult`, `GitDependencyPrompt`, `GitCommandRejection`(명령, 이유).
- 노드 ID(`GitChangeNodeID`)는 코어가 문서 경로·객체 StableId·변경 정렬 키에서 만든 값으로, 같은 대상이면 세대·구조 번호가 바뀌어도 같습니다. 부모·순번 → 변경 해석은 코어가 같은 세대 기준으로 합니다. 100만 change 전체 선택도 최상위 범위 하나로 보냅니다.
- 세대 번호(`generation`)는 값이나 구조가 바뀌면, 구조 번호(`structure`)는 노드가 추가·삭제·이동될 때만 올라갑니다.
- 거부: 세대가 다르거나(`staleGeneration`), 대상이 없거나(`unknownTarget`), 지금 실행할 수 없거나(`unavailable`), 다른 작업이 진행 중이면(`busy`) 코어는 명령을 실행하지 않고 `commandRejected`로 알립니다. 조용히 버리지 않습니다. 셸은 미리 보여 준 값(체크박스, 해결 선택)을 캐시 값으로 되돌리고, 페이지 요청이면 요청 중 표시를 지워 새 세대에서 다시 요청할 수 있게 합니다. 사용자가 시작한 작업 명령(Commit, Push, 해결 적용, 필수 동반 변경 포함, 저장 대기 선택)이 거부되면 실행하지 않았다는 문장과 이유를 오류 배너와 높은 우선순위 알림으로 전합니다(4장). Git 실행 결과의 실패(identity 없음, hook 실패 등)는 거부가 아니라 `operationFinished(.failed)`입니다.
- 표시 문자열(값 요약, 경로, 하위 변경 수 문구 등)은 코어가 만듭니다. 셸은 의미를 계산하지 않습니다. [가이드 1.2 원칙 4]
- remote URL과 자격 증명은 값에 담지 않습니다. 인증은 Git에 위임합니다. [아키텍처 §24.5] [계획 XIV.9.1, XIV.9.6]

FFI 대응 방향: `send`는 명령별 C 함수 또는 버전형 명령 메시지(UTF-8 `(ptr, len)`, 행 범위는 POD 배열)로, `pollEvent`는 `stellar_runtime_poll_event`의 kind + 버전형 payload 디코딩으로 옮깁니다. callback으로 코어가 셸을 재진입하지 않습니다. 이벤트 kind 번호와 payload 레이아웃은 XIV.13.1~XIV.13.4에서 정합니다.

Stub: `GitPanelStubDataSource`(메인 스레드 adapter) + `GitStubBackend`(백그라운드 큐에서 상태 변경, 이벤트 큐만 lock). 실제 Git·파일·FFI를 쓰지 않습니다. 시나리오: `typical`, `nothingStaged`, `identityMissing`, `restricted`, `clean`, `conflicts`, `million`(1,000 문서 × 100 객체 × 10 변경), `wide`(1 문서 × 2,000 객체 × 2 변경, 한 부모 아래 여러 페이지). 새 객체 속성의 Stage 의존성(AddObject 필요)과 그 역(Unstage)을 흉내 냅니다. 노드 ID는 생성 순번에서 만들어 커밋으로 노드가 빠져도 남은 노드의 ID가 바뀌지 않고, 커밋에서만 구조 번호를 올립니다. 세대 불일치·없는 대상, 그리고 Commit·Push·해결 적용이 진행 중일 때 들어온 이 세 명령은 거부 이벤트(`busy`)로 알립니다. 검사용으로 이벤트를 꺼낼 때마다 메인 스레드를 정해진 시간 점유하는 설정(`pollDelay`, 느린 디코딩 모사)과 큐에 남은 이벤트 수를 제공합니다.

## 11. 코어 연결 전 남은 일

- FFI: status·change page(XIV.13.1), stage·unstage + 진행·결과 이벤트(XIV.13.2), commit·push(XIV.13.3), conflict·resolution(XIV.13.4). 이 패널의 `GitPanelDataSource`를 FFI adapter로 구현하고 Stub을 교체합니다.
- 셸 runtime 이벤트 펌프: `stellar_runtime_poll_event`를 메인 런 루프에서 읽어 `onEventsAvailable`을 호출하는 경로.
- 배치: 사이드바 구획 또는 인스펙터 탭(O4), 창 session과 연결(XII.2.2).
- 메뉴 막대 명령(Commit, Push, Refresh, Apply Resolutions)과 command ID 공유(XII.6.5). 툴바에는 Branch·Pull·Fetch를 두지 않습니다(가이드 6.1).
- host 동작 화면: 신뢰 부여·철회 UI, Git identity 안내, Git 설치 안내(가이드 10.4 "전용 WBS 없음").
- 캔버스 연결: `revealConflict`의 선택·카메라 이동과 Base·Ours·Theirs 캔버스 비교(아키텍처 §27.2).
- GitHub 원격 연결(빈 저장소 URL 연결, 계획 XIV.9.2~XIV.9.4)은 이 패널 범위 밖입니다. Remote가 없으면 터미널을 안내합니다.
- 계획 XIV.13.5 완료 기준인 UI automation smoke, XIV.13.7 feature inventory test.
- 간격 상수와 잠정 시간 값(이벤트 예산 1.0ms, UI 같은 차례 상한 0.5ms, 페이드 0.2초, 페이지 256행·32페이지)의 공통화와 가이드 12장 기록(가이드 문서는 이 작업의 수정 범위 밖).
- 문자열 카탈로그(`.xcstrings`)와 번역, 언어별 복수 규칙. 지금은 지역화 키만 있고 수에 따른 문장은 1과 그 밖의 두 문장으로 나눠 둡니다.
- `GitPanelUICheck.run()`을 `scripts/macos/smoke.sh`에 연결(통합 시).
- 기준 하드웨어 Release에서 NSOutlineView 펼침·구조 다시 읽기 시간 측정. §6.3 단일 점유 2ms를 넘으면(이 Mac Debug에서는 넘음) 펼친 묶음의 자식 수 상한이나 다른 표현을 ADR로 정합니다. 한 묶음의 바로 아래 자식이 매우 많으면(예: 한 문서에 변경된 객체 수만 개) 펼치기 비용이 자식 수에 비례해 커집니다.
- 사용자 펼침 한 번이 이 Mac Debug에서 약 34~47ms(8장)로 §6.3 단일 인터랙션 상한 10ms를 넘습니다. 화면에 새로 보이는 행마다 셀을 새로 만드는 비용(Debug 약 1ms)과 AppKit 펼침 작업이 대부분입니다. 기준 하드웨어 Release 측정 뒤 셀 생성 비용 축소(가벼운 셀, 미리 만든 셀 재사용)나 펼침 표현 변경을 정하고, 검사에 사용자 펼침 상한을 둡니다(지금은 값만 보고).

## 12. 검사

`GitPanelUICheck.run() -> Bool`은 Stub으로 패널을 offscreen에서 320×760pt로 만들고 layout을 강제한 뒤 확인합니다.

- SF Symbols 이름 24개 존재.
- 표시 중인 뷰에 모호한 Auto Layout 없음(Light·Dark, 제한 모드, 빈 상태, identity, conflict, 100만 행).
  - 예외: `NSTextView` 안의 비공개 TextKit 2 하위 뷰(`_NSTextContentView` 등)는 검사하지 않습니다. 제약으로 배치한 scroll view 안의 기본 `NSTextView`만 둔 별도 프로그램에서도 같은 보고가 나와 셸 코드와 무관함을 확인했습니다. `NSTextView` 자체는 검사합니다.
- 핵심 컨트롤과 보이는 목록 행의 접근성 레이블, 아이콘 버튼 도움말 태그.
- 구조: 변경 목록은 `NSOutlineView`(outline 열, 접근성 역할 outline, 처음엔 문서 6행), 계층 수준 0·1·2, conflict 목록은 `NSTableView`, 커밋 메시지는 plain text·Undo `NSTextView`, Commit 버튼에 창 전체 키 등가 없음(루트 뷰가 `GitPanelRootView`).
- 변경 행 접근성: 셀 레이블에 제목과 Stage 상태, 체크박스·아이콘·label의 view와 cell이 요소가 아님, 조작 가능할 때 custom action 있음, 체크박스가 Tab 순서에 없음(Light·Dark).
- 흐름: 저장 대기 배너와 Save Now, 빈 메시지 비활성, → 문서 펼침(행 6 → 10), 새 객체를 펼쳐 체크박스 Stage → 필수 동반 변경 확인 → 포함 Stage(펼침 유지), 상세 HEAD·Working 값, Space Unstage·Stage, 세대가 지난 Stage·페이지 요청 거부(체크박스가 코어 값으로 돌아가고 Stage가 실행되지 않음), ←로 부모 이동·접기와 →로 다시 펼치기, Commit(메시지 비움·Undo 복원·staged 0, 구조 변경 뒤 항목을 새로 만들고 문서 펼침을 노드 ID로 복원), Push 진행 배너·취소 버튼·결과.
- 비활성 이유: Stage 없음, identity 없음(+ host 호출), 제한 모드(+ 신뢰 부여 host 호출).
- Conflict: 허용 선택 제한, raw conflict 표시, 세대가 지난 해결 선택 거부(미리 바꾼 segment가 돌아감), O 키 선택, segment 선택, 적용 후 raw conflict와 continue 안내 유지, raw conflict가 Commit을 막음.
- 100만 change: change 1,000,000·최상위 1,000행, 끝·가운데 문서와 그 안 객체 펼치기(행 1,220), 보이는 페이지만 요청(요청 16회 이하), 캐시 32페이지 이하, 행 뷰 200개 이하, 전체 선택 Stage가 최상위 범위 하나로 나감.
- wide: 자식 2,000개 묶음을 펼친 뒤 마지막 자식으로 가면 그 부모의 마지막 페이지(1,792~)만 요청(2회 이하).
- 상태 알림: 저장 대기 배너, 커밋 진행·결과, Push 진행·결과가 알림으로 나가고 짧은 Stage·Unstage는 알리지 않음, identity 없는 커밋 실패는 높은 우선순위.
- 커밋 메시지 잠금: 보낸 직후 편집 불가·Commit 비활성(작업 진행 중), 그 사이 입력이 들어가지 않음, 결과 뒤 다시 편집 가능.
- 표시: Push·Pull 숫자 레이블이 각각 다른 문구, 대비 높이기에서 보조 문구·머리 줄 아이콘·진행·결과 배너 아이콘·행 종류 아이콘이 label 색(주황 배너 아이콘은 그대로)이고 끄면 돌아감, 동작 줄이기에서 전환 시간 0, 선택 강조 행에서 변경·conflict 행 아이콘 tint가 비고 해제하면 돌아감.
- 점유: 패널이 런 루프 차례로 나눈 작업(이벤트 드레인, UI 반영, 변경 상세, 미룬 outline 다시 읽기, 펼침·선택 복원)의 차례 최대가 100만 change와 화면에 보이는 창(배너 페이드 경로가 실제로 실행됨)에서 Debug 상한 4ms(계약 2ms의 두 배) 미만. 이 두 시나리오에서는 복원 펼침이 일어나지 않습니다. 일반 시나리오는 첫 상태 반영과 커밋 뒤 복원 펼침(복원 차례가 실제로 실행됐는지도 확인)이 들어 있어 한 프레임(1/60초) 미만. 측정값은 보고에 남깁니다. 사용자 펼침(검사가 `expandItem`을 직접 부름)은 차례 점유에 넣지 않고 따로 재어 값만 보고합니다(상한 없음, 11장).
- 펼침 기억: 문서 셋을 펼친 뒤 그중 하나를 커밋으로 빼면 남은 두 문서가 다시 펼쳐지고(같은 페이지로 온 두 문서도 복원 차례 하나에 구조 변경 하나), 다른 문서를 두 번 더 커밋하면 빠진 문서의 펼침 ID가 버려지고 남은 두 문서의 ID만 남음.
- 느린 드레인: Stub이 이벤트마다 `inlineUIThreshold`의 1.5배를 점유하게 하고 드레인 한 차례를 직접 실행하면, 그 차례에서는 상태만 받고 UI(요약 문구)는 바꾸지 않으며 미룬 UI 반영 횟수가 하나 늘고, 다음 차례에 UI가 반영됨.
- 사용자 동작 거부: 패널이 아직 모르는 Push가 진행 중일 때 보낸 Commit이 `busy`로 거부되면 메시지 잠금이 풀리고(문장 유지) 오류 배너·높은 우선순위 알림에 거부 문구가 나오며, 새 작업 결과(Push 성공)가 오면 그 결과로 바뀜. 다른 응답이 먼저 처리된 필수 동반 변경 확인에 답하면 `unknownTarget` 거부를 알리고 목록이 다시 조작 가능함. 취소 응답의 거부는 알리지 않음. Dismiss가 거부 알림을 닫음.
- 키보드: 창에 넣기 전 두 화면을 오가도 패널 컨트롤 20개에 직접 이은 `nextKeyView`가 없음. 셸 창처럼 위·아래에 다른 컨트롤이 있고 `autorecalculatesKeyViewLoop`를 켠 창(화면 밖 좌표에 표시)에서, 실제 Tab처럼 창에 다음 key view를 물은 뒤 고리를 따라가면 위 컨트롤 → 패널 → 아래 컨트롤로 나감. 순서: (변경 화면) 새로고침 → 화면 전환 → Save Now → Use Saved Version → 목록 → 메시지 → Commit → Push → remote, (진행 중) Cancel, (결과) Dismiss, (conflict 화면) 화면 전환 → 목록 → Apply, (제한 모드) 화면 전환 → Trust Project…. 고리 소속을 보며, 시스템 키보드 탐색 설정에 따른 실제 정지 여부는 보지 않음.
- ⌘↩ 범위: 패널 밖 포커스에서는 처리하지 않고 커밋 명령도 나가지 않음, 커밋 메시지 포커스에서는 처리하고 `.commit(message:)`이 나감, conflict 화면에서는 처리하지 않음.

검사하지 않은 것: 실제 Git·코어 FFI, 화면 표시(on-screen) 렌더, VoiceOver 실제 낭독 순서와 outline 행 요소 트리(7장의 한 번 확인 외), sheet 경로의 필수 동반 변경 확인, 대비 높이기·투명도 줄이기 켠 상태의 시각 결과, Release 빌드·기준 하드웨어 성능.

## 13. 근거

- 가이드: 1.2(원칙), 2.1·2.3(창·영역), 3.2(재질), 4.1\~4.4(글꼴·색·외형·간격), 5.1\~5.4(컨트롤·텍스트·AppKit 경계·아이콘), 6.1·6.4(툴바·Git 패널), 7.1\~7.2(빈 상태·흐름 안내), 8.1\~8.5(키보드·포커스·VoiceOver·접근성 설정·모션), 9장(성능), 11장 O4.
- 아키텍처: §6.3·§6.4(성능 계약), §8.1·§8.2(셸·코어 경계), §9.2(스레드), §11.2(UI 텍스트), §24.1\~§24.5(시스템 Git·신뢰·초기 UI·인증), §25.1·§25.3(identity·Push), §26(Status·Diff·Staging), §27.1\~§27.2(Merge·Conflict 해결 UI).
- 계획: XIV.2.8, XIV.4.2, XIV.4.4, XIV.4.5, XIV.5.3, XIV.7.2, XIV.8.1\~XIV.8.5, XIV.9.1, XIV.9.5, XIV.9.6, XIV.11.4, XIV.12.1\~XIV.12.5, XIV.13.1\~XIV.13.5, XIV.13.7.
- ADR-0009(FFI C ABI, pull 이벤트 큐).
- Apple HIG: Sidebars, Lists and tables, Outline views, Buttons, Alerts, Sheets, Toolbars, Writing(빈 상태·오류), Color, Typography, Materials, Accessibility, Keyboard, Focus and selection, Motion, Dark Mode.
