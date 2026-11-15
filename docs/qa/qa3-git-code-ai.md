# QA3 Git·코드·AI 확인 목록

핵심 가치 세 가지를 직접 확인합니다. Git으로 협업할 수 있는지, CSS 속성 그대로 디자인하고 실제 코드로 내보내는지, AI 에이전트가 쓰는 CLI·MCP가 앱과 같은 프로젝트를 다루는지입니다. 진행 방식과 피드백 형식은 [README](README.md)를 따릅니다.

- 기준 명세: [UX 명세](../design/ux/stellar-ux-spec.md) 4장 흐름 3·5·7·8, [컴포넌트 원본](../spec/component-source-v1.md), [웹 Frame 내보내기](../spec/web-frame-export.md), [stellar CLI 안내](../guides/stellar-cli.md)
- 각 절 명령 블록에서 `# QA3-xx` 줄 아래 명령을 차례로 실행하고 표의 "보여야 하는 것"과 비교합니다. hash·generation 번호·시간은 달라도 됩니다.
- 확인 범위: 터미널 명령과 출력은 이 빌드의 CLI로 같은 샘플에서 실제로 실행해 맞췄습니다(앱 단계는 CLI로 대신함). 앱 화면 반영(QA3-06·09·16\~19), 앱으로 만든 `qa-code`의 내보내기(QA3-22\~23)는 코드로만 확인했습니다. 다르면 피드백입니다.

## 0. 준비

앱을 ⌘Q로 끈 뒤 **저장소 루트에서** 아래 블록 전체를 실행합니다. 중간에 새 터미널 창을 열면 저장소 루트로 가서 첫 줄 `qa.sh`만 빼고 블록 전체를 다시 실행합니다.

```bash
./scripts/macos/qa.sh --no-build --fresh        # 깨끗한 샘플을 앱으로 엶
cargo build -q -p stellar-cli -p stellar-mcp     # README 1절에서 했으면 생략
R="$PWD"                                         # 저장소 루트
export PATH="$R/target/debug:$PATH"
export GIT_PAGER=cat                             # git diff가 pager에 멈추지 않게
Q="${TMPDIR%/}/stellar-qa"                       # 요청 파일은 프로젝트 밖에 둠
S="$Q/sample"
P="$Q/sample-peer"
F=design/components/Card-6f960d2d92d4.stellar.yaml   # QA3-10에서 만드는 파일(이름이 매번 같음)
cd "$S"

# 이름이 $1인 레이어를 $2로 바꾸는 stellar apply 요청을 현재 revision 기준으로 만듭니다.
rename_request() {
  local rev doc node
  rev=$(stellar query . project | jq -r '.result.revision.token')
  doc=$(stellar query . project | jq -r '.result.documents[0].id')
  node=$(stellar query . tree "$doc" | jq -r --arg n "$1" '.result.nodes[] | select(.name == $n) | .id')
  printf '{"format":"stellar-apply","format_version":1,"base_revision":"%s","document":"%s","commands":[{"kind":"rename","node":"%s","name":"%s"}]}' "$rev" "$doc" "$node" "$2"
}
# 컴포넌트 원본의 현재 hash(고칠 때마다 바뀜).
source_hash() { stellar query . components | jq -r '.result.entries[0].source_hash'; }
# MCP 메시지 하나를 보내고 그 응답만 보여 줍니다(7절).
mcp() {
  printf '%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"qa","version":"0"}}}' \
    '{"jsonrpc":"2.0","method":"notifications/initialized"}' "$1" \
    | stellar-mcp --headless --project "$S" 2>/dev/null | jq -c 'select(.id == 2) | .result'
}
```

- 앱 창 부제목이 "Restricted Mode"인지 봅니다. 아니면(앞 단계에서 신뢰함) 상단 바 제목 메뉴 ▸ Revoke Project Trust로 거둡니다. `--fresh`는 신뢰를 지우지 않습니다. 6절까지는 제한 모드 그대로 진행합니다.
- 이 문서 동안 앱에서는 표에 적은 단계에서만 편집합니다. 터미널이 문서를 바꾼 뒤 앱에서 편집하면 저장이 거부됩니다(QA3-09에서 일부러 확인).
- `stellar`는 `design/` 아래 문서를 앱과 같은 저장 방식으로 고칩니다. 열린 앱 창은 터미널이 바꾼 캔버스 문서를 다시 읽지 않으므로, 확인할 때는 창을 닫고(⌘W) File ▸ Open Project…로 다시 엽니다.

## 1. CLI로 읽기

```bash
# QA3-01
stellar version
stellar inspect . | jq -c .result.project
# QA3-02
stellar query . project | jq -r '.result.documents[0].pages[] | .name + ": " + ([.views[].name] | join(", "))'
# QA3-03
stellar validate . | jq -c '{ok, valid: .result.valid}'
```

| ID | 보여야 하는 것 |
|---|---|
| QA3-01 | JSON 한 줄에 `"ok":true`. 프로젝트 `{"id":"019a8279-3001-…","name":"Stellar QA Sample"}` |
| QA3-02 | `Home: Desktop, Mobile`, `Components: Buttons`(앱의 Page › View 팝업과 같음) |
| QA3-03 | `{"ok":true,"valid":true}` |

## 2. 터미널에서 고치고 앱에서 보기

```bash
# QA3-04 (앱에 저장하지 않은 편집이 없을 때)
rename_request "Button" "Primary CTA" > "$Q/rename.json"
stellar apply . "$Q/rename.json" --dry-run | jq -c '.result | {mode, change: .changes.entries[0] | {old: .old.value, new: .new.value}}'
git status --short
# QA3-05
stellar apply . "$Q/rename.json" | jq -c '{mode: .result.mode, generation: .result.revision.generation}'
git status --short
# QA3-07
rename_request "Footer (Locked)" "Footer" | stellar apply . - | jq -c '{ok, code: .error.code}'
# QA3-08
stellar apply . "$Q/rename.json" | jq -c '{exit_code, code: .error.code}'
# QA3-09 (QA3-06 뒤 다시 연 창에서 Hero 이름을 바꾸고 2초 기다린 뒤)
rename_request "Get Started" "Start Now" | stellar apply . - > /dev/null
```

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA3-04 | 미리 보기(`--dry-run`) | `{"mode":"dry-run","change":{"old":"Button","new":"Primary CTA"}}`. `git status`는 출력 없음(아무것도 쓰지 않음) |
| QA3-05 | 실제 적용 | `{"mode":"write","generation":2}`. ` M design/current`와 새 `design/generations/…`, `design/objects/…` |
| QA3-06 | 앱 창은 그대로 둔 채 Mobile 레이어 목록 보기 → ⌘W → File ▸ Open Project…로 샘플 다시 열기 | 닫기 전에는 "Button" 그대로(열린 창은 다시 읽지 않음). 다시 열면 "Primary CTA" |
| QA3-07 | 잠근 레이어 이름 바꾸기 | stderr에 "잠긴 노드 …는 바꿀 수 없습니다(잠금 해제는 그 노드의 set-locked)", 결과 `{"ok":false,"code":"rejected-command"}`. 앱과 같은 잠금 규칙(D8) |
| QA3-08 | 이미 쓴 요청을 다시 적용 | stderr에 "base revision이 현재 revision과 다릅니다", 결과 `{"exit_code":65,"code":"base-revision-mismatch"}`. 다른 쪽이 먼저 저장한 문서를 덮어쓰지 않음 |
| QA3-09 | 앱에서 Hero 이름을 `Hero App`으로 바꾸고 2초 기다림 → 위 명령 → 앱에서 레이어 하나를 골라 화살표로 옮기고 1초쯤 기다림 → ⌘S → ⌘W → Close Without Saving → 다시 열기 | 화살표 이동 뒤 부제목에 "⚠︎ Not Saved — Changed Elsewhere". ⌘S에 "The Design Couldn’t Be Saved"와 "This project was saved from another window or app after this window opened it. This window’s changes weren’t saved." ⌘W에 "The Window Couldn’t Be Closed"와 Try Again·Close Without Saving·Cancel. 다시 열면 "Hero App"과 "Start Now"는 있고 화살표 이동은 없음 |

## 3. 사람이 읽는 컴포넌트 원본

컴포넌트는 `design/components/`에 컴포넌트 하나가 파일 하나이고, 속성 하나가 한 줄입니다. 앱에서 만드는 화면은 아직 없어서(D4) CLI로 만듭니다.

```bash
# QA3-10
git add -A && git commit -qm "QA3 앞 단계"
mkdir -p design/components
stellar component-source create design/components Card 019a8279-4000-7000-8000-000000000001 019a8279-4000-7000-8000-000000000002 | jq -c '.result | {name, file}'
cat "$F"
# QA3-11
git add design/components && git commit -qm "Card 컴포넌트 추가"
git status --short
# QA3-12
stellar component-source resize "$F" "$(source_hash)" 240 120 > /dev/null
git diff
# QA3-13
git commit -qam "Card 크기"
stellar component-source set-fill "$F" "$(source_hash)" srgb 0.18 0.44 0.92 1 > /dev/null
git diff
git commit -qam "Card 채우기"
```

| ID | 보여야 하는 것 |
|---|---|
| QA3-10 | `{"name":"Card","file":"design/components/Card-6f960d2d92d4.stellar.yaml"}`. 파일은 `format: stellar-component`, `schema_version: "3"`, `payload:` 아래 `background`, `border-…-radius` 4개, `border-color`, `border-width`, `height`, `layout-mode`, `width`, `clipping`, `transform`이 한 줄씩. 알려진 차이: 키와 값이 아직 CSS 이름·문법이 아님(`background`↔`background-color`, `clipping`↔`overflow`, 색은 `srgb` 성분, 위치는 `transform` 행렬) |
| QA3-11 | 커밋 하나. `git status --short`는 출력 없음 |
| QA3-12 | 바뀐 줄은 `height: "0"`→`"120"`, `width: "0"`→`"240"` 두 줄뿐 |
| QA3-13 | 바뀐 줄은 `background: none`→`"srgb 0.18 0.44 0.92 1"` 한 줄 |

## 4. 앱에 배치하고 바로 반영(제한 모드 그대로)

```bash
# QA3-14
rev=$(stellar query . project | jq -r '.result.revision.token')
doc=$(stellar query . project | jq -r '.result.documents[0].id')
root=$(stellar query . project | jq -r '.result.documents[0].pages[] | select(.name == "Components") | .views[0].root_frame')
printf '{"format":"stellar-apply","format_version":1,"base_revision":"%s","document":"%s","commands":[{"kind":"insert-component-instance","parent":"%s","component":"019a8279-4000-7000-8000-000000000001","source_hash":"%s","x":440,"y":400}]}' "$rev" "$doc" "$root" "$(source_hash)" \
  | stellar apply . - | jq -c '{ok, kinds: [.result.changes.entries[].object_kind]}'
git add -A && git commit -qm "Card 인스턴스 배치"
# QA3-16 (QA3-15에서 창을 다시 연 뒤, 창을 연 채로)
stellar component-source resize "$F" "$(source_hash)" 320 160 > /dev/null
git status --short
sleep 3
git checkout -- design/components
```

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA3-14 | Components ▸ Buttons의 root에 Card 인스턴스를 (440, 400)에 넣기 | `"ok":true`, `kinds`에 `node:component-instance`와 `modifier:transform`(순서는 바뀔 수 있음) |
| QA3-15 | 앱 창을 닫고 다시 열어 Components ▸ Buttons 보기 | 부제목은 여전히 "Restricted Mode". 오른쪽 아래 빈 곳에 파란 240 × 120 사각형. 레이어 목록에 "Card" 행 |
| QA3-16 | 터미널에서 원본 크기를 바꿨다가 Git으로 되돌리기 | `git status`에 ` M design/components/Card-….stellar.yaml`. 창을 다시 열지 않아도 캔버스의 Card가 320 × 160으로 커졌다가 240 × 120으로 돌아옴. 앱의 선택은 그대로 |

## 5. 브랜치·병합·팀원 복제본

```bash
# QA3-17 (앱을 연 채로. 한 브랜치는 폭, 다른 브랜치는 높이를 바꿈)
git switch -c wide
stellar component-source resize "$F" "$(source_hash)" 320 120 > /dev/null && git commit -qam "Card 폭 320"
git switch -
git switch -c tall
stellar component-source resize "$F" "$(source_hash)" 240 160 > /dev/null && git commit -qam "Card 높이 160"
git merge --no-edit wide
stellar query . components | jq -c '.result.entries[0] | {width, height}'
# QA3-18 (다른 폴더의 복제본에서 고쳐 커밋하고, 샘플이 받아 옴)
git clone -q "$S" "$P"
(cd "$P" && stellar component-source resize "$F" "$(source_hash)" 360 200 > /dev/null && git commit -qam "Card 360x200")
git pull --ff-only -q "$P" HEAD
stellar query . components | jq -c '.result.entries[0] | {width, height}'
```

| ID | 보여야 하는 것 |
|---|---|
| QA3-17 | `git merge`가 충돌 없이 끝남("Auto-merging design/components/Card-….stellar.yaml", "Merge made by the 'ort' strategy."). 결과 `{"width":"320","height":"160"}`. 앱 캔버스의 Card도 브랜치를 바꿀 때마다 따라 바뀌고 마지막에 320 × 160 |
| QA3-18 | `git pull`이 오류 없이 끝나고 결과 `{"width":"360","height":"200"}`. 열린 앱의 Card가 다시 열지 않아도 360 × 200 |

## 6. CSS 속성과 웹 코드

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA3-19 | File ▸ New Project…로 `$Q` 폴더(`…/stellar-qa`) 안에 `qa-code`를 만들고, 흰 Frame 안에 F로 Frame 하나, 그 Frame 안에 R로 사각형 하나 | 레이어 목록에 Frame ▸ Rectangle |
| QA3-20 | 속성 패널에서 새 Frame `background-color` `#2f6feb`, `border-radius` `16px`, 사각형 `fill` `#ffffff` → ⌘S | 캔버스에 바로 반영 |

```bash
# QA3-21, QA3-23
cd "$Q/qa-code"
doc=$(stellar query . project | jq -r '.result.documents[0].id')
view=$(stellar query . project | jq -r '.result.documents[0].pages[0].views[0].id')
stellar export-web . "$doc" "$view" > "$Q/view.json"; echo "exit=$?"
jq -jr '.result.html' "$Q/view.json" > "$Q/view.html" && open "$Q/view.html"
# QA3-22
grep -E '^ *(left|top|width|height|background-color|border-top-left-radius):' "$Q/view.html"
cd "$S"
```

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA3-21 | 내보내기 | `exit=0`. stderr의 `warning(fixed-free-frame-only)`(둥근 모서리가 있으면 `warning(browser-rectangle-curve)`도)는 정상. 브라우저에 캔버스와 같은 배치·크기·색·모서리가 보임 |
| QA3-22 | 내보낸 CSS 보기 | 속성 패널 행과 같은 CSS 이름과 값. root는 `width: 800px`·`height: 600px`, 새 Frame은 `background-color`와 `border-top-left-radius: 16px` 같은 longhand, 자식은 부모 기준 `left`·`top` |
| QA3-23 | `qa-code`에 Ellipse 또는 Text를 하나 더 그리고 ⌘S → 위 블록 다시 실행 | `exit=1`, `view.html`은 `null` 한 단어. stderr에 그 레이어 ID와 `error(unsupported-node)`(예: "Shape의 시각·텍스트 스타일·의미는 이 출력 범위에서 해석할 수 없습니다"). 지원 범위는 Frame·Fragment·Rectangle·배치한 Frame 컴포넌트 |

## 7. MCP(에이전트가 읽는 경로)

`stellar-mcp`는 에이전트 CLI가 붙는 MCP 서버입니다. 이 빌드에서는 **신뢰한** 프로젝트 하나를 읽기 전용으로 엽니다. 여기서 처음으로 신뢰를 줍니다.

```bash
# QA3-25
mcp '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | jq -r '.tools[].name'
# QA3-26
mcp '{"jsonrpc":"2.0","id":2,"method":"resources/read","params":{"uri":"stellar://components"}}' | jq -c '.contents[0].text | fromjson | .entries[] | {name, width, height}'
# QA3-27 (앱에서 레이어 이름 하나를 바꾸고 2초 기다린 뒤)
mcp '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"document.semantic_diff","arguments":{"base":"HEAD"}}}' | jq -c '.content[0].text | fromjson | .items[] | {kind, old: .old.value, new: .new.value}'
# QA3-28
mcp '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"document.validate","arguments":{}}}' | jq -c '.content[0].text | fromjson | {valid, errors}'
# QA3-29 (앱에서 Revoke Project Trust를 고른 뒤)
stellar-mcp --headless --project "$S" < /dev/null; echo "exit=$?"
# QA3-30
stellar-mcp --connection-info | jq -c .
```

| ID | 해 볼 것 | 보여야 하는 것 |
|---|---|---|
| QA3-24 | 앱에서 제목 메뉴 ▸ Trust Project… ▸ Trust Project | 부제목 "Restricted Mode"가 사라짐. 메뉴 항목이 Revoke Project Trust로 바뀜 |
| QA3-25 | tool 목록 | `document.semantic_diff`, `document.validate` |
| QA3-26 | 컴포넌트 읽기 | `{"name":"Card","width":"360","height":"200"}`(QA3-18 결과와 같음) |
| QA3-27 | 앱 편집과 HEAD 비교 | `{"kind":"set-property","old":"<이전 이름>","new":"<새 이름>"}`. 앱이 자동 저장한 내용을 Git HEAD와 비교 |
| QA3-28 | 검증 | `{"valid":true,"errors":0}` |
| QA3-29 | 신뢰를 거둔 뒤 MCP 열기 | "읽기 프로젝트를 선택할 수 없습니다: 절대 경로와 프로젝트 신뢰를 확인하세요", `exit=77` |
| QA3-30 | 연결 정보 | `"transport":"stdio"`, `command`(이 빌드 `stellar-mcp` 절대 경로), `"args":[]`. 알려진 문제: 이 출력 그대로 등록하면 인자 없는 서버가 tools/list에 "알 수 없는 method입니다"(-32601)로 답함. 에이전트 CLI에 등록할 때는 인자 `--headless --project <샘플 절대 경로>`를 직접 붙임 |

## 8. 아직 안 됨

- 앱 안 Git 화면: 변경 목록, Stage·Unstage, 커밋, Push, 충돌 해결(UX 명세 8장)
- 캔버스 문서의 사람이 읽는 원본과 Git 병합: 문서는 `design/objects/` 바이너리이고, 두 브랜치가 문서를 고치면 `design/current`가 충돌로 남음. `stellar semantic-merge`는 합친 결과를 보고만 하고 파일을 쓰지 않음(V.10.7, 이번 테스트 제외)
- 열린 창이 터미널·Git이 바꾼 캔버스 문서를 다시 읽기(XIII.3.1). 컴포넌트 원본만 바로 반영됨
- 컴포넌트 원본의 키·값을 CSS 이름·문법으로 쓰기(QA3-10의 알려진 차이)
- 앱에서 컴포넌트 만들기·찾기·배치·편집(Assets 탭, D4), 인스턴스 override
- 내보내기: Text·Ellipse·Line·Polygon·Image·path, 그림자 등 modifier, flex·grid 레이아웃, 반응형, 앱 안 코드 보기
- MCP: 쓰기 tool, 실행 중인 앱에 붙는 `--attach`(지금은 "실행 중인 Stellar 앱이 없습니다", exit 69), 앱 안 에이전트 탭과 내장 터미널
- `stellar text-source export`: 페이지가 하나인 문서만, 앱이 읽지 않는 opt-in 출력
