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
   - 변경 목록(`NSTableView`, 가상화). 행 = 펼침 표시 · Stage 체크박스 · 종류 아이콘 · 제목 · 보조 문구(값 변화 요약 등). 계층 들여쓰기는 문서 0, 객체 1, 변경 2단계입니다.
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
| 작업 진행 | `.operationProgress` 수신 ~ `.operationFinished` | 진행 배너(시스템 진행 표시 + 문구, 취소 가능하면 "Cancel"). 목록 조작과 Commit·Push·적용 비활성 |
| 실패 | `.operationFinished(.failed)` | 오류 배너(빨간 아이콘 + Git 결과 요약 + "Dismiss"). 아이콘 모양과 문구로도 전달 [가이드 4.2] |
| 성공 | Commit·Push·해결 적용 성공 | 결과 배너 + "Dismiss". Stage·Unstage 성공은 목록 변화로 충분하므로 배너 없음 |
| 필수 동반 변경 | `.dependencyPrompt` | 창이 보이면 `NSAlert` sheet, 아니면 배너. "Include N Related Changes" / "Cancel". 자동 포함 없음 [계획 XIV.5.3, XIV.7.2] [가이드 5.1] |

## 5. 상호작용

- Stage 체크박스: 꺼짐 = unstaged, 켜짐 = staged, 혼합(mixed) = 부분 staged(HEAD ≠ Index ≠ Working) 또는 묶음의 일부만 staged. 누르면 staged는 Unstage, 그 밖은 Stage합니다. 결과가 올 때까지 목표 상태를 먼저 보여 주고, 새 세대 페이지가 실제 값을 덮어씁니다.
- 묶음(문서·객체) 행의 체크박스는 접혀 있어도 하위 변경 전체에 적용합니다.
- 여러 행 선택 후 Space 또는 문맥 메뉴(Stage / Unstage)로 한꺼번에 적용합니다. 전체 선택(⌘A)은 행 범위 하나로 코어에 전달합니다.
- 펼침 표시: 묶음을 접고 펼칩니다. 평탄화는 코어가 하고 새 세대를 돌려줍니다.
- 선택 → 상세: 선택이 연속으로 바뀌면 다음 런 루프에서 한 번만 요청합니다. 새 세대 상세가 올 때까지 이전 값을 유지해 깜빡임과 재배치를 막습니다. [가이드 9장]
- Commit: 비활성 이유가 없을 때만 실행합니다. 성공하면 메시지를 텍스트 시스템 경로로 비워 Undo로 되살릴 수 있습니다(검사로 확인). 메시지 초안은 메모리에만 있고 프로젝트에 저장하지 않습니다.
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
| ← / → | 선택한 묶음 행 접기 / 펼치기 | 변경 목록 |
| B / O / T | 선택 conflict에 Base / Ours / Theirs | conflict 목록 |
| ⌘↩ | Commit(버튼 키 등가; 메시지 입력 중에도 동작) | Changes 화면 |
| Tab / ⇧Tab | 커밋 메시지에서 다음·이전 컨트롤로 이동. 탭 문자는 ⌥Tab | 커밋 메시지 |

- 수식키 없는 한 글자 키는 메뉴 key equivalent가 아니라 목록 `keyDown`에서만 처리하므로 텍스트 입력 중에는 발동하지 않습니다. [가이드 8.2]
- 목록은 type select를 끕니다(100만 행을 훑지 않게).
- 포커스 순서: 화면 전환 → 목록 → (Changes) 메시지 → Commit → identity → Push → remote → 새로고침 / (Conflicts) 목록 → Show on Canvas → Apply → 새로고침. Full Keyboard Access로 모든 컨트롤에 도달합니다. [가이드 8.2]
- Commit·Push·Apply는 통합 시 메뉴 막대 명령으로도 등록해야 합니다(11장). [가이드 6.1, 8.1]

## 7. 접근성과 외형

- 루트 뷰는 "Source Control" 그룹입니다. 목록(Changes, Conflicts), 커밋 메시지, remote 팝업, 화면 전환에 접근성 레이블이 있습니다.
- 아이콘만 있는 새로고침 버튼은 접근성 레이블과 도움말 태그를 함께 둡니다. [가이드 5.4, 8.1]
- 목록 행: 체크박스 "Stage <제목>", 펼침 "Expand/Collapse <제목>", 종류 아이콘은 종류 이름(예: Property Changed), conflict segment는 "Resolution for <제목>", segment별 도움말 태그에 각 쪽 값을 둡니다.
- Push·Pull 화살표 아이콘은 옆 숫자 label이 "2 to push, 0 to pull"로 읽히므로 접근성 요소에서 뺍니다.
- 비활성 버튼은 이유를 `accessibilityHelp`로 읽게 합니다.
- 상태는 색에만 의존하지 않습니다(체크박스 모양, 아이콘 모양, 문구). [가이드 4.2]
- 색은 시스템 의미 색만 씁니다(label 계층, `systemOrange`·`systemRed` 상태 아이콘). 하드코딩 RGB가 없습니다. [가이드 4.2]
- 글꼴은 텍스트 스타일(body, callout, headline)과 시스템 고정폭(속성 경로·conflict 경로)만 씁니다. [가이드 4.1]
- Light·Dark: 시스템 외형을 따르며 두 외형에서 offscreen 렌더를 눈으로 확인했습니다.
- 대비 높이기: 보조 문구를 `secondaryLabelColor`에서 `labelColor`로 올립니다. 투명도 줄이기: 패널은 직접 그리는 반투명 표면이 없어 바꿀 대상이 없습니다. 동작 줄이기: 상태 배너 등장·퇴장 애니메이션 시간을 0으로 합니다. 실행 중 변경은 `NSWorkspace` 접근성 표시 옵션 알림으로 반영합니다. [가이드 8.4, 8.5]

## 8. 성능과 가상화

- 목록은 세대(generation)와 행 범위로 페이지 단위 요청합니다. 페이지 256행, 메모리 상한 32페이지(LRU). 보이는 행의 페이지만 요청합니다. [계획 XIV.13.1] [가이드 6.4, 9장]
- 행 높이는 고정(셀 내용에서 계산)이라 100만 행에서도 위치 계산이 O(1)입니다. 행 수가 같은 세대 변경은 이미 만든 셀을 그 자리에서 다시 구성하고 선택을 건드리지 않습니다. 행 수가 바뀌면 행 수만 알리고 다시 배치합니다. `reloadData()`는 처음 채울 때만 씁니다.
- 세대가 바뀌어도 행 수가 같으면 직전 페이지를 새 페이지가 올 때까지 stale 값으로 보여 줍니다(조작 불가).
- 이벤트 드레인 한 번의 예산은 1.5ms이고, 넘으면 다음 런 루프로 넘깁니다. 상태 반영은 묶어서 구획마다 제약을 한 번만 다시 만듭니다. [아키텍처 §6.3]
- 측정(2026-11-14, Debug 빌드, offscreen 창, 이 개발 Mac): 100만 change 시나리오(1,101,000행)에서 이벤트 드레인 최대 약 0.9\~1.1ms, 페이지 요청 4회, 메모리 페이지 4개, 행 뷰 17개. 일반 시나리오의 최대 드레인은 약 2.2\~2.4ms이며 프로세스 첫 상태 반영(첫 셀·글꼴·심볼 생성) 한 번입니다. 드레인 밖의 AppKit layout·display 시간은 측정하지 않았고, 기준 하드웨어 Release 측정이 아닙니다.
- 간격 상수(바깥 여백 12, 행 간격 6, 줄 안 간격 4, 들여쓰기 14, 행 위아래 여백 3, 단위 pt)는 잠정값입니다. 구획은 묶음 간격 대신 구분선과 행 간격으로 나눕니다. 가이드 4.4는 모든 패널이 공유하는 상수를 요구하므로 통합 시 공통 상수로 옮기고 가이드 12장에 기록해야 합니다.

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

- 명령(`GitPanelCommand`): `refreshStatus`, `loadChangePage(generation, rows)`, `loadChangeDetail(generation, row)`, `setStaged(generation, rows: IndexSet, staged)`, `setGroupExpanded(generation, row, expanded)`, `answerDependencyPrompt(requestID, includeRequired)`, `commit(message)`, `push(remote)`, `loadConflictPage(generation, rows)`, `chooseResolution(generation, rows, choice)`, `applyResolutions`, `revealConflict(generation, row)`, `resolvePendingAutosave(saveFirst)`, `cancelOperation(id)`.
- 이벤트(`GitPanelEvent`): `status(GitStatusSnapshot)`, `changePage`, `changeDetail`, `conflictPage`, `dependencyPrompt`, `operationProgress`, `operationFinished`.
- 값 타입: `GitStatusSnapshot`(가용성, HEAD, upstream, remote 이름, identity 여부, staged·unstaged 수, merge 상태, conflict 수, staged snapshot 문제, semantic status 오류, 저장 대기 여부, 두 목록의 세대·행 수), `GitChangeRow`, `GitChangeDetail`, `GitConflictRow`, `GitOperationProgress`, `GitOperationResult`, `GitDependencyPrompt`.
- 행 → 변경 ID 해석은 코어가 같은 세대 기준으로 합니다. 세대가 다르면 코어는 요청을 거부하고, 셸은 이전 세대 결과를 버립니다. 100만 행 선택도 `IndexSet` 범위로 보냅니다.
- 표시 문자열(값 요약, 경로, 하위 변경 수 문구 등)은 코어가 만듭니다. 셸은 의미를 계산하지 않습니다. [가이드 1.2 원칙 4]
- remote URL과 자격 증명은 값에 담지 않습니다. 인증은 Git에 위임합니다. [아키텍처 §24.5] [계획 XIV.9.1, XIV.9.6]

FFI 대응 방향: `send`는 명령별 C 함수 또는 버전형 명령 메시지(UTF-8 `(ptr, len)`, 행 범위는 POD 배열)로, `pollEvent`는 `stellar_runtime_poll_event`의 kind + 버전형 payload 디코딩으로 옮깁니다. callback으로 코어가 셸을 재진입하지 않습니다. 이벤트 kind 번호와 payload 레이아웃은 XIV.13.1~XIV.13.4에서 정합니다.

Stub: `GitPanelStubDataSource`(메인 스레드 adapter) + `GitStubBackend`(백그라운드 큐에서 상태 변경, 이벤트 큐만 lock). 실제 Git·파일·FFI를 쓰지 않습니다. 시나리오: `typical`, `nothingStaged`, `identityMissing`, `restricted`, `clean`, `conflicts`, `million`(1,000 문서 × 100 객체 × 10 변경). 새 객체 속성의 Stage 의존성(AddObject 필요)과 그 역(Unstage)을 흉내 냅니다.

## 11. 코어 연결 전 남은 일

- FFI: status·change page(XIV.13.1), stage·unstage + 진행·결과 이벤트(XIV.13.2), commit·push(XIV.13.3), conflict·resolution(XIV.13.4). 이 패널의 `GitPanelDataSource`를 FFI adapter로 구현하고 Stub을 교체합니다.
- 셸 runtime 이벤트 펌프: `stellar_runtime_poll_event`를 메인 런 루프에서 읽어 `onEventsAvailable`을 호출하는 경로.
- 배치: 사이드바 구획 또는 인스펙터 탭(O4), 창 session과 연결(XII.2.2).
- 메뉴 막대 명령(Commit, Push, Refresh, Apply Resolutions)과 command ID 공유(XII.6.5). 툴바에는 Branch·Pull·Fetch를 두지 않습니다(가이드 6.1).
- host 동작 화면: 신뢰 부여·철회 UI, Git identity 안내, Git 설치 안내(가이드 10.4 "전용 WBS 없음").
- 캔버스 연결: `revealConflict`의 선택·카메라 이동과 Base·Ours·Theirs 캔버스 비교(아키텍처 §27.2).
- GitHub 원격 연결(빈 저장소 URL 연결, 계획 XIV.9.2~XIV.9.4)은 이 패널 범위 밖입니다. Remote가 없으면 터미널을 안내합니다.
- 계획 XIV.13.5 완료 기준인 UI automation smoke, XIV.13.7 feature inventory test.
- 간격 상수의 공통화와 가이드 12장 기록, 문구 지역화.
- `GitPanelUICheck.run()`을 `scripts/macos/smoke.sh`에 연결(통합 시).

## 12. 검사

`GitPanelUICheck.run() -> Bool`은 Stub으로 패널을 offscreen에서 320×760pt로 만들고 layout을 강제한 뒤 확인합니다.

- SF Symbols 이름 24개 존재.
- 표시 중인 뷰에 모호한 Auto Layout 없음(Light·Dark, 제한 모드, 빈 상태, identity, conflict, 100만 행).
  - 예외: `NSTextView` 안의 비공개 TextKit 2 하위 뷰(`_NSTextContentView` 등)는 검사하지 않습니다. 제약으로 배치한 scroll view 안의 기본 `NSTextView`만 둔 별도 프로그램에서도 같은 보고가 나와 셸 코드와 무관함을 확인했습니다. `NSTextView` 자체는 검사합니다.
- 핵심 컨트롤과 보이는 목록 행의 접근성 레이블, 아이콘 버튼 도움말 태그.
- 구조: 목록은 `NSTableView`, 커밋 메시지는 plain text·Undo `NSTextView`, Commit 키 등가 ⌘↩.
- 흐름: 저장 대기 배너와 Save Now, 빈 메시지 비활성, 체크박스 Stage → 필수 동반 변경 확인 → 포함 Stage, 상세 HEAD·Working 값, Space Unstage·Stage, ←/→ 접기·펼치기, Commit(메시지 비움·Undo 복원·staged 0), Push 진행 배너·취소 버튼·결과.
- 비활성 이유: Stage 없음, identity 없음(+ host 호출), 제한 모드(+ 신뢰 부여 host 호출).
- Conflict: 허용 선택 제한, raw conflict 표시, O 키 선택, segment 선택, 적용 후 raw conflict와 continue 안내 유지, raw conflict가 Commit을 막음.
- 100만 change: 행 수 1,101,000, 끝·중간·임의 위치 스크롤 시 보이는 페이지만 요청(요청 16회 이하), 캐시 32페이지 이하, 행 뷰 200개 이하, 접기·펼치기, 전체 선택 Stage, 이벤트 드레인 최대 한 프레임(1/60초) 미만(회귀 방지용 상한).

검사하지 않은 것: 실제 Git·코어 FFI, 화면 표시(on-screen) 렌더, VoiceOver 실제 낭독 순서, sheet 경로의 필수 동반 변경 확인, 대비 높이기·투명도 줄이기 켠 상태의 시각 결과, Release 빌드·기준 하드웨어 성능.

## 13. 근거

- 가이드: 1.2(원칙), 2.1·2.3(창·영역), 3.2(재질), 4.1\~4.4(글꼴·색·외형·간격), 5.1\~5.4(컨트롤·텍스트·AppKit 경계·아이콘), 6.1·6.4(툴바·Git 패널), 7.1\~7.2(빈 상태·흐름 안내), 8.1\~8.5(키보드·포커스·VoiceOver·접근성 설정·모션), 9장(성능), 11장 O4.
- 아키텍처: §6.3·§6.4(성능 계약), §8.1·§8.2(셸·코어 경계), §9.2(스레드), §11.2(UI 텍스트), §24.1\~§24.5(시스템 Git·신뢰·초기 UI·인증), §25.1·§25.3(identity·Push), §26(Status·Diff·Staging), §27.1\~§27.2(Merge·Conflict 해결 UI).
- 계획: XIV.2.8, XIV.4.2, XIV.4.4, XIV.4.5, XIV.5.3, XIV.7.2, XIV.8.1\~XIV.8.5, XIV.9.1, XIV.9.5, XIV.9.6, XIV.11.4, XIV.12.1\~XIV.12.5, XIV.13.1\~XIV.13.5, XIV.13.7.
- ADR-0009(FFI C ABI, pull 이벤트 큐).
- Apple HIG: Sidebars, Lists and tables, Outline views, Buttons, Alerts, Sheets, Toolbars, Writing(빈 상태·오류), Color, Typography, Materials, Accessibility, Keyboard, Focus and selection, Motion, Dark Mode.
