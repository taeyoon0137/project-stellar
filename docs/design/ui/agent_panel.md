# 에이전트 패널 UI 명세(agent_panel)

- 상태: 초안(2026-11-14). 코어 미연결, Stub 데이터로만 동작합니다. UX 명세 반영(2026-11-15)
- 영역: 오른쪽 영역의 "에이전트" 탭. attached `stellar-mcp`(XIX.9.4)나 내장 세션 실행 경로(XXIX.2·XXIX.3)가 연결된 뒤에만 탭이 생기고, 그 전에는 탭이 없습니다. 탭을 넣기 전 계획 XVI.11.8 개정(UX.0.3)이 필요합니다. [가이드 6.5] [계획 XVI.11.8] [결정 U13]
- 코드: 없음. Stub 데이터로만 동작하던 이전 패널 코드(`apps/macos/Stellar/UI/Agent/`)와 그 UI 검사(`--agent-panel-ui-check`)는 UX.1.1에서 Agent 탭 연결을 지운 뒤 제품 어디에서도 쓰이지 않아 2026-11-15 삭제했습니다. 오른쪽 에이전트 탭은 실행 경로가 연결될 때 이 명세와 UX 명세 9장으로 새로 구현합니다(UX.10).
- 상위 기준: [macOS 셸 디자인 가이드](../macos-shell-design.md), [UX 명세](../ux/stellar-ux-spec.md) 9장, 기술 아키텍처 명세, ADR-0005(Proposed), ADR-0009. `UX.n.m`은 [UX WBS 초안](../ux/ux-wbs-rows.md)의 행입니다.

## 0. 근거 표기

가이드 0.1절의 표기를 그대로 씁니다. 이 문서에서 추가로 쓰는 표기는 아래와 같습니다.

| 표기 | 의미 |
|---|---|
| `[가이드 n]` | `docs/design/macos-shell-design.md`의 해당 절 |
| `[PO 2026-11-14 에이전트]` | 제품 책임자 결정(2026-11-14): 에이전트 출력은 채팅 보기를 기본으로 하고 원문 텍스트 보기를 대안으로 둠. 세션 중지·연결 끊김에서 자동 롤백하지 않고, 에이전트 변경을 표시한 채 사용자가 되돌리게 함 |

이 문서는 아키텍처 명세, 구현 계획, ADR, 가이드를 바꾸지 않습니다. 어긋나는 곳은 13장에 적습니다. [가이드 0.2]

---

## 1. 목적과 위치

- 사용자가 이미 설치하고 직접 인증한 CLI 에이전트로 열린 프로젝트를 편집할 때 쓰는 패널입니다. 패널은 세션, 입력, 승인, 미리보기 요청을 제공하고 문서 의미를 소유하지 않습니다. [ADR-0005 Context, 결정 4.1] [가이드 1.2 원칙 4]
- 자리는 오른쪽 영역(속성·코드·에이전트)의 에이전트 탭입니다. 인스펙터와 동시에 보는 별도 trailing 열은 두지 않습니다. 패널은 `NSViewController` 하나(`AgentPanelViewController`)이고 탭 연결은 UX.10.3입니다. [가이드 2.3, 6.3, 6.5] [결정 U13]
- 기준선 `02397601`에서는 캔버스 아래 보조 패널에 `DisconnectedAgentPanelSource` 탭(버튼 없이 안내만)이 붙어 있습니다. 보조 패널과 이 탭은 UX.1.1에서 지우고, 실행 경로가 연결되기 전에는 에이전트 탭을 만들지 않습니다. [결정 U1] [UX 명세 9.1]
- 툴바 뒤쪽 에이전트 상태 항목(세션 진행, 승인 대기 수, 외부 MCP 연결 수)은 세션·연결·승인 대기가 있을 때만 보이고, 누르면 이 탭을 엽니다. 탭을 숨겨도 세션과 승인 대기는 유지됩니다. [결정 U4] [UX 명세 3.3, 9.3]
- 구획 구성의 목표는 UX 명세 9.2절(머리, 대화, 범위 승인 카드, batch 카드, 이 세션의 변경, 연결, 제한 모드)입니다. 이 문서의 구성과 다른 부분은 UX.10 행에서 맞춥니다.
- 승인 화면은 네이티브 chrome(패널, 창 sheet)에만 둡니다. 캔버스와 WebView에는 그리지 않습니다. [ADR-0005 결정 4.5] [가이드 6.5]
- 전부 AppKit으로 만들었고 SwiftUI는 쓰지 않습니다. 대화 기록은 갱신이 잦은 목록이기 때문입니다. [가이드 5.3]

## 2. 화면 구성

```text
┌──────────────── 에이전트 패널 ────────────────┐
│ [CLI 팝업 ················] [… 더 보기]        │  헤더
│ (기호) 연결 상태            Model: (CLI 보고값) │
│ [Chat | Raw Text]           [모드 팝업]         │
├───────────────────────────────────────────────┤
│ 대화 기록(채팅 보기, 기본) 또는 원문 텍스트 보기  │  남는 높이 전부
│  · 사용자 메시지 / 에이전트 메시지              │
│  · 도구 단계(Stellar 기록)                      │
│  · 승인 카드(코어 요약 + 에이전트 설명)          │
│  · 에이전트 적용 변경(출처 표시 + 되돌리기)      │
│  · Stellar 알림(세션 시작·종료 등)               │
│ 또는 빈·진행·오류 상태                           │
├───────────────────────────────────────────────┤
│ (기호) N changes applied by the agent [Revert All…] │  세션 막대(있을 때만)
│ (기호) Session ended …          [Start New Session…] │
├───────────────────────────────────────────────┤
│ [입력창 NSTextView ··············] [■] [↑]      │  입력창·중지·보내기
└───────────────────────────────────────────────┘
```

- 세션 막대의 동작은 더 보기 메뉴에도 있습니다. 막대에만 있는 기능은 없습니다. [가이드 2.4]
- 수치(여백, 반경, 줄 수)는 `AgentPanelMetrics` 한곳에 잠정값으로 둡니다(12장). [가이드 0.2, 4.4]

## 3. 표시 내용

### 3.1 헤더

| 요소 | 내용 | 근거 |
|---|---|---|
| CLI 팝업 | 설치된 CLI 표시 이름과 CLI가 보고한 버전. 버전 미지원·인증 필요는 항목 제목에 덧붙임. 세션 중에는 비활성 | [ADR-0005 결정 4.1] |
| 연결 상태 | 기호 + 문구: No session, Starting…, Connected, Connected — working, Reconnecting…, Session ended, Sign-in required, Unsupported CLI version, No agent CLI found | [ADR-0005 결정 4.1, 4.2] |
| 모델 | 세션이 시작된 뒤 CLI가 보고한 모델 이름을 그대로 표시. Stellar가 고르지 않음 | [ADR-0005 결정 4.1] |
| 보기 전환 | `NSSegmentedControl` Chat / Raw Text. Chat이 기본 | [PO 2026-11-14 에이전트] |
| 모드 팝업 | Read Only / Review Before Applying(기본) / Apply Within Approved Scope. 사용자 UI에서만 바꿈 | [ADR-0005 결정 4.5] |
| 더 보기 | Start Session…, Stop Session, Revert All Agent Changes in Session…, Release All Agent Leases in Project… | [ADR-0005 결정 4.5] [가이드 6.5] |

- 비용·사용량·한도는 표시하지 않습니다. 스냅샷 타입에도 그런 필드가 없습니다. [ADR-0005 결정 4.1]

### 3.2 대화 기록(채팅 보기)

| 항목 | 출처 | 표시 |
|---|---|---|
| 사용자 메시지 | 사용자 입력. 그대로 CLI에 전달 | "You" + 본문 |
| 에이전트 메시지 | CLI 출력의 표시 전용 디코드 | "Agent"(응답 중이면 "Agent — responding") + 본문 |
| 도구 단계 | **Stellar 자체 tool 호출 기록**. CLI 출력을 해석한 것이 아님. 문서 변경은 Stellar MCP tool로만 일어나고 셸에는 공급자 출력 해석 코드가 없기 때문입니다 [ADR-0005 결정 4.3 불변식 1, 4.4 디코더 위치·문서 변경 경로 금지] | 상태 기호 + "Done · Stellar record" + 단계 이름 + 요약 |
| 승인 카드 | 주 내용은 **코어 생성**(요약, semantic diff, validation, 잠금 범위). 에이전트 설명은 보조 | 3.3절 |
| 에이전트 적용 변경 | 코어 변경 기록 | "Applied by agent"(+ Reverted / Can’t revert automatically) + 요약 + [Revert] |
| Stellar 알림 | 셸 고정 문구 | "Stellar" + 세션 시작·모드 변경·세션 종료·lease 해제 |

- 근거: [ADR-0005 결정 4.4, 4.5] [PO 2026-11-14 에이전트]
- 목록은 `NSTableView`(열 하나, 자동 행 높이)로 보이는 행만 만듭니다. 새 항목이 오면 바닥에 붙어 있던 경우에만 따라 내려갑니다. [아키텍처 §6.3] [가이드 9]

### 3.3 승인 카드

위에서 아래로 다음 순서입니다. 콘텐츠 계층이므로 glass를 쓰지 않습니다. [가이드 3.1]

1. 제목: 방패 기호 + "Approval Needed · Edit / Git Commit / Git Push". 민감 확인은 기호를 주황 시스템 색으로 칠하지만 종류는 문구로도 구분합니다. [가이드 4.2 "색에만 의존하지 않음"]
2. 코어 요약 한 줄.
3. Changes: 코어 semantic diff 요약 줄(추가·삭제·수정은 서로 다른 모양의 기호와 VoiceOver 문구로 구분). 카드에 싣지 않은 줄 수는 "N more changes"로 표시합니다.
4. Validation: 오류·경고 수와 코어 진단 메시지.
5. Lock Scope: 코어가 계산한 lease 범위.
6. Agent’s Note: 에이전트 설명이 있을 때만. "Provided by the agent. Stellar didn’t check it." 출처 문구를 함께 둡니다. [ADR-0005 결정 4.5]
7. 대기 중이면 버튼 줄: [Preview] … [Decline] [Approve] (민감 확인은 [Approve…]). 처리되면 상태 줄(Approved / Declined / Expired — treated as declined).

- 렌더 프리뷰는 카드에 이미지를 싣지 않고 "Preview" 명령으로 코어 transient preview 경로에 요청합니다(14장).

### 3.4 원문 텍스트 보기

- 읽기 전용 `NSTextView`. CLI 출력 원문을 해석 없이 시스템 고정폭 글꼴로 보여 줍니다. 선택·복사·찾기 막대를 씁니다. [ADR-0005 결정 4.4] [가이드 4.1, 5.2]
- 새 텍스트는 끝에 이어 붙이고, 바닥에 있던 경우에만 따라 내려갑니다.

### 3.5 세션 막대

- 에이전트 변경이 하나 이상이면 "N changes applied by the agent"와 [Revert All…](파괴적 동작 표시)을 보여 줍니다.
- 기록이 있는 채 세션이 끝났거나 실패했으면 종료 문구와 [Start New Session…]을 보여 줍니다.
- 표시할 줄이 없으면 막대를 계층에서 빼서 공간을 기록에 돌려줍니다.

### 3.6 입력창

- 여러 줄 `NSTextView`(plain text, undo, IME, 맞춤법). 내용에 맞춰 2~8줄 사이로 높이를 바꿉니다. [아키텍처 §11.2] [가이드 5.2, 6.5]
- 보내기(아이콘 버튼)와 중지(아이콘 버튼). 비어 있을 때 안내 문구를 보이고, 접근성으로는 placeholder 값으로 노출합니다.
- 안내 문구는 hit test에서 빠져 클릭을 받지 않습니다. 문구 위를 눌러도 입력창이 포커스를 받습니다.
- 보낸 뒤 입력창은 텍스트 시스템 경로(`shouldChangeText` → 교체 → `didChangeText`)로 비웁니다. 지우기도 Undo 기록에 남으므로 Command-Z는 보낸 문장을 입력창에 되살립니다. `string`을 직접 바꾸면 이전 타이핑 Undo가 없는 범위를 가리켜 Undo에서 앱이 종료되므로 쓰지 않습니다. [아키텍처 §11.2] [가이드 5.2]

## 4. 상태

| 상태 | 조건(대화 기록이 비어 있을 때) | 화면 | 다음 동작 |
|---|---|---|---|
| 진행 | phase = loading(CLI 확인, 세션 시작) | 시스템 spinning `NSProgressIndicator` + 문구 | 없음 |
| 오류 | phase = failed | 오류 기호 + 원인 + 해결 문구 | Check Again 또는 Start New Session… |
| CLI 없음 | 설치된 CLI 없음 | 설치·로그인은 CLI 자체 안내를 따르라는 문구 | Check Again |
| 버전 미지원 | 선택 CLI가 검증되지 않은 버전 | 업데이트 안내 | Check Again |
| 인증 필요 | CLI가 인증 필요를 보고 | Stellar는 로그인에 참여하지 않음을 안내 | Open Sign-In Terminal… |
| 신뢰 안 됨 | 프로젝트 미신뢰 | 신뢰된 프로젝트에서만 시작함을 안내 | Review Project Trust… |
| 세션 없음 | 연결 없음 또는 종료 | 시작 안내 | Start Session… |

- 빈 상태는 다음 할 일과 그 동작을 함께 둡니다. [가이드 7.1] [HIG Writing]
- **대화 기록이 있으면 빈·오류 상태로 기록을 가리지 않습니다.** 에이전트 변경을 계속 보고 되돌릴 수 있어야 하기 때문입니다. 이때 실패·종료는 세션 막대 문구와 헤더 상태로 알립니다. [PO 2026-11-14 에이전트]
- 세션 종료(중지·연결 끊김·lease 해제): 대기 중 승인은 거부로 정리되고(Expired), 적용된 변경은 롤백하지 않고 표시한 채 남습니다. [ADR-0005 결정 4.5 "승인 요청 저장"] [PO 2026-11-14 에이전트]

## 5. 상호작용

| 동작 | 결과 명령 | 확인 |
|---|---|---|
| 보내기 | `sendMessage(text)` (앞뒤 공백 제거, 빈 문장은 보내지 않음) | 없음 |
| 중지 | `stopSession` | 없음(자동 롤백이 없어 되돌릴 수 없는 결과가 없음) |
| 일반 승인·거부 | `respondToApproval(id, approve/decline)`. 보낸 뒤 코어 갱신(`itemUpdated`)이 올 때까지 카드의 승인·거부 버튼과 접근성 동작을 끕니다(빠른 두 번 클릭이 여러 응답이 되지 않게) | 없음. 버튼에 키 등가 없음 |
| 민감 승인(Git commit·push) | `respondToApproval(id, approve)` | 경고 sheet: Cancel이 Return 기본, 확인 버튼은 `hasDestructiveAction`·키 등가 없음 |
| 미리보기 | `previewApproval(id)` | 없음 |
| 변경 하나 되돌리기 | `revertChange(id)` | 없음 |
| 세션 변경 모두 되돌리기 | `revertAllSessionChanges` | 파괴적 확인 sheet |
| 세션 시작 | `startSession(cli, mode)` | 시작 확인 sheet: 대상 프로젝트, 전송 대상 CLI, 모드 표시. 첫 연결 승인으로 기록 |
| 프로젝트 lease 모두 해제 | `releaseAllProjectLeases` | 파괴적 확인 sheet |
| 모드 변경 | `setMode(mode)` | 없음 |
| CLI 변경 | `selectCLI(id)` (세션 없을 때만) | 없음 |
| 빈 상태 동작 | `refreshInstalledCLIs`, `openSignInTerminal(id)`, `reviewProjectTrust`, 시작 확인 | 위와 같음 |
| 복사 | 선택한 행의 plain text(에이전트 설명은 출처와 함께) | 없음 |
| 문맥 메뉴 | Copy, 대기 승인 행은 Approve/Decline/Preview on Canvas, 적용 변경 행은 Revert This Agent Change | 위와 같음 |

- 근거: [ADR-0005 결정 4.5] [가이드 5.1(파괴적 버튼은 Return 기본 아님), 6.5] [HIG Alerts, HIG Buttons]
- 확인 sheet는 창이 없으면 거부로 끝냅니다(fail closed). [ADR-0005 Security "모든 판단은 fail closed"]
- 활성 세션이 있으면 `requiresConfirmationBeforeClosing`이 true입니다. 창 닫기·앱 종료 확인은 통합 시 창 delegate가 이 값을 씁니다. [ADR-0005 결정 4.5]

## 6. 키보드

| 키 | 동작 |
|---|---|
| Return | 보내기. IME 조합 중에는 조합 확정이 우선 |
| Shift-Return, Option-Return | 줄바꿈 |
| Command-Return | 보내기. 패널 안에 포커스가 있을 때만 |
| Command-Period | 세션 중지(macOS 취소 관례). 패널 안에 포커스가 있고 세션이 활성일 때만 |
| Tab / Shift-Tab | CLI 팝업 → 더 보기 → 보기 전환 → 모드 → 대화 기록 → (원문 텍스트) → (빈 상태 버튼) → 입력창 → 중지 → 보내기. 숨긴 뷰는 건너뜀. 입력창에서도 Tab은 포커스를 옮기고, 탭 문자는 Option-Tab으로 넣음 |
| 화살표 | 대화 기록 행 이동(시스템 목록 동작, 행 전체 하이라이트) |
| Command-C | 선택한 행 복사(입력창·원문 보기에서는 시스템 텍스트 복사) |

- Command-Return·Command-Period는 버튼 키 등가로 두지 않습니다. 버튼 키 등가는 창의 key equivalent 탐색이 창 전체 뷰를 훑으므로 캔버스·레이어 목록·다른 패널에서 눌러도 발동합니다. 패널 루트 뷰(`AgentPanelRootView`)가 first responder가 패널 하위일 때만 처리하고, 아니면 탐색을 넘깁니다. [가이드 8.1, 8.2]
- 승인·되돌리기에는 키 등가를 두지 않습니다. 키보드 사용자는 포커스한 버튼을 Space로 누르거나 Full Keyboard Access를 씁니다. 키보드 기본 동작만으로 승인되지 않게 하기 위해서입니다. [ADR-0005 결정 4.5] [가이드 8.1, 8.2]
- 메뉴 막대 연결(통합 시): 컨트롤러가 first responder action과 `NSMenuItemValidation`을 이미 구현합니다. `showAgentChatView:`, `showAgentRawTextView:`, `startAgentSession:`, `stopAgentSession:`, `revertAllAgentSessionChanges:`, `releaseAllAgentLeases:`, `copy:`. 메뉴 위치·단축키는 메뉴 WBS가 정합니다. [가이드 8.1] [계획 XII.6.5]

## 7. 접근성과 외형

- 아이콘만 있는 컨트롤(보내기, 중지, 더 보기)에 접근성 레이블과 도움말 태그를 둡니다. CLI 팝업, 모드 팝업, 보기 전환, 대화 기록, 원문 텍스트, 입력창, 패널 전체에도 레이블을 둡니다. [가이드 5.4, 8.1, 8.3]
- 메시지·도구 단계·알림 행은 하위 레이블을 숨기고 행 하나를 "역할: 본문" 요소로 읽습니다. 승인 카드와 변경 행은 그룹으로 두어 안의 버튼을 그대로 노출합니다. [가이드 8.3]
- 승인 카드와 변경 행은 접근성 custom action(Approve/Approve…, Decline, Revert This Agent Change)을 제공합니다. 민감 승인 action도 같은 확인 sheet를 거치므로 보조 기술로 확정할 수 있습니다. [ADR-0005 결정 4.5] [가이드 8.3]
- 상태는 색만으로 나타내지 않고 기호 모양과 문구를 함께 씁니다. 색은 시스템 의미 색(label 계층, separator, control background, quaternary system fill, placeholder text)과 시스템 색(초록·주황·빨강)만 씁니다. 하드코딩 RGB가 없습니다. [가이드 4.2]
- 시스템 접근성 표시 설정(가이드 8.4):
  - 투명도 줄이기: 승인 카드 배경을 반투명 fill에서 불투명 control background로 바꿉니다.
  - 대비 높이기: 카드 테두리를 굵게 그립니다. separator 색은 시스템이 대비에 맞춥니다.
  - 동작 줄이기: 행 삽입 fade를 끕니다. 그 밖의 장식 애니메이션은 없습니다.
  - 실행 중 변경은 `NSWorkspace.accessibilityDisplayOptionsDidChangeNotification`으로 받아 보이는 카드를 다시 그립니다.
- Light·Dark: 카드 배경은 draw 시점에 동적 색을 해석하므로 외형 전환에 자동으로 맞습니다. 검사는 두 외형 모두에서 레이아웃과 그리기를 실행합니다(11장).

## 8. SF Symbols

| 이름 | 용도 |
|---|---|
| `ellipsis.circle` | 더 보기 메뉴 |
| `arrow.up` | 보내기 |
| `stop.fill` | 중지 |
| `person.crop.circle` | 사용자 메시지 |
| `sparkles` | 에이전트 메시지, 에이전트 적용 변경(출처 표시), 세션 변경 막대 |
| `info.circle` | Stellar 알림 |
| `clock` / `checkmark.circle` / `xmark.octagon` / `hand.raised` | 도구 단계 진행·완료·실패·거부 |
| `checkmark.shield` | 승인 카드 제목 |
| `plus.circle` / `minus.circle` / `pencil.circle` | diff 추가·삭제·수정 |
| `checkmark.seal` / `exclamationmark.triangle` / `xmark.octagon` | validation 통과·경고·오류 |
| `lock` | 잠금 범위 |
| `quote.bubble` | 에이전트 설명 |
| `checkmark.circle` / `xmark.circle` / `clock.badge.xmark` | 승인됨·거부됨·만료 |
| `arrow.uturn.backward.circle` / `exclamationmark.circle` | 되돌림·자동 되돌리기 불가 |
| `link` / `arrow.triangle.2.circlepath` / `circle.dashed` / `stop.circle` | 연결됨·연결 중·세션 없음·종료 |
| `person.crop.circle.badge.exclamationmark` / `exclamationmark.triangle` / `terminal` | 인증 필요·버전 미지원·CLI 없음 |
| `lock.shield` / `exclamationmark.octagon` / `play.circle` | 신뢰 안 됨·실패·세션 시작 |

- 이미지 asset은 쓰지 않고 `NSImage(systemSymbolName:)`만 씁니다. 검사가 모든 이름이 시스템에서 해석되는지 확인합니다. [가이드 5.4]

## 9. 데이터 계약(프로토콜)

`AgentPanelModel.swift`에 정의합니다. 코어 FFI의 pull 이벤트·스냅샷 경계에 맞춘 값 타입이며 Rust 타입을 노출하지 않습니다. [아키텍처 §8.2] [ADR-0009]

```swift
protocol AgentPanelSource: AnyObject {
    func currentSnapshot() -> AgentPanelSnapshot    // 처음 붙일 때 전체 상태
    func pollEvent() -> AgentPanelEvent?            // 대기 이벤트 하나(없으면 nil)
    func perform(_ command: AgentPanelCommand)      // 사용자 의도
}
```

- **스냅샷** `AgentPanelSnapshot`: 세대 번호, 헤더(`AgentSessionHeader`), 단계(`AgentPanelPhase`), 대화 기록 항목 배열, 원문 텍스트, 세션 변경 집계.
- **이벤트** `AgentPanelEvent`(세대 번호 + 변경): `reset(스냅샷)`, `headerChanged`, `phaseChanged`, `itemsAppended`, `itemUpdated`, `rawTextAppended`, `changeSummaryChanged`, `attentionRequested(kind)`.
  - 세대가 현재와 다른 증분 이벤트는 버립니다. `reset`은 스냅샷 세대가 현재보다 새롭고 이벤트 세대와 같을 때만 받습니다. 같은 세대 `reset`을 다시 받으면 증분으로 쌓은 기록과 원문 텍스트가 지워지기 때문입니다. [아키텍처 §9.2] [가이드 9]
  - 셸은 콜백을 등록하지 않고 꺼내 씁니다. 통합 시 앱 이벤트 펌프가 `AgentPanelViewController.pumpEvents()`를 부릅니다. [ADR-0009 "callback 등록 API 없음"]
- **명령** `AgentPanelCommand`: 5장 표의 명령들. 결과는 이벤트로 돌아옵니다.
- **식별자**: `AgentItemID`, `AgentApprovalID`, `AgentChangeID`, `AgentCLIID`는 코어가 발급하는 불투명 `UInt64`입니다.
- **문구 소유**: 단계·실패·알림 종류는 열거형으로 받고 문구는 셸이 정합니다. 코어가 넘기는 문자열은 표시 전용 값(요약, diff 줄, 진단, 이름, 에이전트 텍스트)뿐입니다.
- **알림**: `AgentAttentionKind`는 연관값이 없어 OS 알림에 프로젝트 데이터를 실을 수 없습니다. 고정 문구는 `AgentPanelText.notificationTitle/Body`에 있습니다. [ADR-0005 결정 4.5] [아키텍처 §39]
- **Stub**: `AgentSessionStub`은 의도적 placeholder입니다. CLI를 실행하지 않고, 메시지를 어디에도 보내지 않고, 문서를 바꾸지 않습니다. 받은 명령을 기록하고 코어가 보낼 법한 이벤트를 큐에 넣습니다. 시나리오: 활성 세션, 세션 없음, CLI 없음, 인증 필요, 버전 미지원, 신뢰 안 됨, 진행, 오류.

## 10. 성능

- 모든 프로토콜 호출은 메인 스레드에서 즉시 반환해야 합니다. CLI 프로세스, IPC, diff 계산은 구현체가 다른 스레드에서 처리합니다. [아키텍처 §6.3, §6.4, §9.2] [ADR-0005 결정 6]
- 한 번의 런 루프에서 이벤트를 최대 `maxEventsPerPass`개 적용하고 나머지는 다음 런 루프로 넘깁니다. [아키텍처 §6.3 "단일 점유 2ms"]
- 대화 기록은 보이는 행만 만들고, 패널 폭이 바뀌면 다음 런 루프에 한 번만 보이는 행을 다시 잽니다. [가이드 9]
- 입력창 높이 계산은 짧은 입력에만 전체 레이아웃을 강제합니다. 긴 붙여넣기는 최대 높이를 바로 씁니다. text view가 TextKit 1로 내려간 경우(`textLayoutManager`가 nil)에도 layout manager의 사용 영역으로 같은 방식으로 잽니다.
- 원문 텍스트는 끝에 이어 붙이기만 합니다. 보관 상한(오래된 줄 정리)은 코어 연결 때 정합니다(14장).

## 11. 검사(`AgentPanelUICheck`)

`enum AgentPanelUICheck { static func run() -> Bool }`. 화면에 표시하지 않은 창(360×760pt)에 Stub 데이터로 패널을 만들고 레이아웃을 강제해 다음을 확인합니다.

- 모든 SF Symbol 이름이 해석됨. 알림 문구에 샘플 프로젝트 데이터가 없음.
- 확인 alert 구성: 민감 확인은 Cancel이 Return 기본, 확인 버튼은 파괴적 표시·키 등가 없음. 시작 확인에 프로젝트·CLI·모드 포함.
- 활성 세션(Light, Dark 각각): 채팅 보기 기본, 처음 표시에서 마지막 항목이 보임, 헤더 값, 핵심 컨트롤 접근성 레이블·도움말 태그, 보내기·중지 버튼에 창 전체 키 등가 없음, 모든 행을 차례로 화면에 들여와 모호한 Auto Layout 없음·여러 줄 레이블 폭과 잘림·행 여백 과다 없음·행 접근성 레이블, 승인 카드 구성(민감·일반·처리됨, 에이전트 설명 출처 표시), 변경 행 되돌리기 버튼, 접근성 표시 설정 경로와 그리기.
- 상호작용: 민감 승인은 확인 거부 시 명령 없음·확인 시 명령, 일반 승인은 확인 없이 명령·같은 카드 두 번째 응답 없음, 개별·세션 되돌리기, Return 보내기와 IME 조합 중 Return 제외, 보낸 뒤 Undo가 종료 없이 보낸 문장을 되살림(회귀 검사), 안내 문구 위 hit test가 입력창, 입력창 Tab이 탭 문자를 넣지 않음, 원문 텍스트 보기 전환·이어 붙이기, 문맥 메뉴, 중지 후 변경 집계 유지(자동 롤백 없음)·주의 요청, 새 세션 시작 확인과 세대 교체.
- 중지 시 대기 승인 만료: 승인하지 않은 두 카드(Git commit, 편집)가 Expired가 되고 응답 버튼·접근성 동작이 사라짐.
- 단축키 범위: 같은 창의 패널 밖 텍스트 필드에 포커스가 있으면 Command-Return·Command-Period를 처리하지 않고 명령도 없음. 입력창 포커스에서 Command-Return 보내기, 대화 기록 포커스에서 Command-Period 중지.
- reset 세대 규칙: 같은 세대 reset과 이벤트·스냅샷 세대가 어긋난 reset은 버리고, 새 세대 reset만 적용.
- 빈·진행·오류 7종: 상태 뷰 표시, 기대 동작 버튼, 동작 버튼이 명령을 보냄, 모호성 없음, 그리기.

실행 인자와 smoke 연결은 `main.swift`·`scripts/macos/smoke.sh`를 고치는 통합 작업에서 합니다(14장). 그 전에는 이 폴더 파일만 함께 컴파일한 별도 실행 파일로 돌립니다. 결과 예(2026-11-14): `agent-panel-ui ok: light 12행·레이블 36개; dark 12행·레이블 36개; 상호작용 7개 명령; 중지 시 대기 승인 2건 만료; 단축키 범위(패널 밖 무시, 안에서 보내기·중지); reset 세대 규칙; 빈·진행·오류 7종`.

회귀 검사 확인(2026-11-14): 수정마다 하나씩 되돌린 사본에서 검사가 실패함을 확인했습니다. 입력창을 `string = ""`로 비우면 Undo에서 NSException으로 종료, 안내 문구 hit test를 되살리면 "안내 문구 위 클릭 대상이 입력창이 아님", 포커스 확인을 빼면 "패널 밖 포커스에서 패널 단축키가 처리됨", 같은 세대 reset을 받으면 "같은 세대 reset이 증분 기록을 지움", 응답 대기 확인을 빼면 "같은 승인에 응답이 여러 번 나감".

## 12. 잠정 수치

가이드 12장 기록 대상입니다. 통합 WBS에서 HIG Layout과 Stellar 측정으로 확정한 뒤 가이드에 옮깁니다. [가이드 0.2, 12]

| 항목 | 잠정값 | 비고 |
|---|---|---|
| 패널 바깥 여백 `edgeInset` | 12pt | 가이드 4.4 공통 단위로 통합 필요 |
| 요소 간격 `itemSpacing` | 6pt | |
| 행 위아래 여백 `rowVerticalPadding` | 6pt | |
| 아이콘 열 폭 `symbolColumnWidth` | 18pt | |
| 카드 반경 `cardCornerRadius` | 8pt | 동심 모서리 값으로 재검토(가이드 3.5) |
| 카드 테두리 / 대비 높이기 | 1pt / 2pt | |
| 카드 안 여백 `cardContentInset` | 10pt | |
| 입력창 줄 수 | 최소 2, 최대 8 | |
| 입력창 전체 레이아웃 상한 | 4096 UTF-16 | |
| 런 루프당 이벤트 수 `maxEventsPerPass` | 64 | 실측으로 확정(§6.3) |
| 행 높이 추정값 | 60pt | |
| 바닥 고정 판단 여유 | 24pt | |

## 13. 상위 문서와 어긋나는 점(이 문서는 고치지 않음)

- **자동 롤백**: 아키텍처 §31.5와 ADR-0005 결정 4.2·4.5("lease 모두 해제"의 롤백)는 세션 중지·연결 종료에서 에이전트 변경 전체를 롤백한다고 적습니다. 이 패널은 [PO 2026-11-14 에이전트]에 따라 자동 롤백 없이 변경을 표시하고 사용자가 되돌리게 합니다. ADR-0005 Accepted 전 개정에서 맞춰야 합니다.
- **출력 표시 단계**: ADR-0005 결정 4.4는 1단계에서 텍스트만 표시한다고 적습니다. 이 패널은 채팅 보기를 기본, 원문 텍스트를 대안으로 둡니다([PO 2026-11-14 에이전트]). 채팅 보기의 에이전트 메시지는 2단계의 표시 전용 디코드에 해당하며, 도구 단계와 승인 카드는 CLI 출력이 아니라 Stellar 기록·코어 결과입니다.
- **내장 AI 경계**: 아키텍처 §31.1 본문("내장 AI 없음", 외부 CLI는 터미널에서 실행)은 ADR-0005와 아직 맞춰지지 않았습니다. [가이드 6.5]
- **계획**: 이 패널에 대응하는 행은 UX WBS 초안의 UX.10(계획서 등록 전, UX.0.3)과 계획 XXIX.4입니다. [가이드 10.1, 10.2]

## 14. 코어 연결·통합에 남은 일

1. 패널 data source를 Rust 코어 FFI(pull 이벤트, 스냅샷)로 구현합니다(이전 Stub 코드는 삭제됨). 버전형 메시지 형식과 세대 번호 규칙을 FFI 쪽에서 정합니다. [아키텍처 §8.2] [ADR-0009]
2. 앱 이벤트 펌프에서 `pumpEvents()`를 부르는 시점(코어 이벤트 도착 신호)을 정합니다.
3. 오른쪽 에이전트 탭에 패널을 붙입니다(UX.10.3). 실행 경로가 연결되기 전에는 탭을 만들지 않습니다. [가이드 6.5] [계획 XVI.11.8] [결정 U13]
4. 메인 메뉴 항목(6장 action), 창 닫기·앱 종료 확인(`requiresConfirmationBeforeClosing`)을 연결합니다.
5. 패널을 다시 구현하면 영역 UI 검사(`--agent-panel-ui-check`)를 그 패널과 함께 `main.swift`·`scripts/macos/smoke.sh`에 둡니다. 쓰이지 않는 패널을 검사만으로 남기지 않습니다.
6. OS 알림 게시(UserNotifications 권한 포함)를 `attentionHandler`에 연결합니다. 문구는 고정 문구만 씁니다.
7. 렌더 프리뷰: 카드 안 이미지 표시 여부와 캔버스 transient preview 연동을 정합니다. [ADR-0005 결정 3 "live 미리보기", 결정 8 `render_preview`]
8. 이전 세션의 에이전트 변경: 새 세션을 시작하면 대화 기록이 바뀌므로, 이전 세션 변경의 출처 표시·되돌리기는 문서 쪽(레이어 트리·캔버스) 표시와 함께 코어 기록으로 정합니다.
9. 대화 기록·원문 텍스트 보관 상한과 페이지 단위 로드(오래된 항목). [가이드 9]
10. 승인 카드의 diff 전체 보기(코어 Semantic Diff 화면)로 가는 경로.
11. 지역화: `AgentPanelText`의 문구를 지역화 리소스로 옮깁니다.
12. 12장 잠정 수치를 측정·확정해 가이드 12장에 기록합니다.
13. 문맥 메뉴 외에 키보드만으로 행 동작(승인·되돌리기)을 여는 방식이 더 필요한지 접근성 적합성 검사(XII.9.4)에서 확인합니다.
