# 실사용 QA

macOS 앱과 `stellar` CLI·`stellar-mcp`를 직접 써 보며 화면과 동작을 확인하는 절차입니다. 이 문서만 보고 혼자 진행할 수 있게 썼습니다. "보여야 하는 것"은 이 빌드의 Swift·Rust 코드에서 문자열과 동작을 확인해 적었습니다. 다르게 보이면 그것이 피드백입니다.

- 대상: macOS 앱(`apps/macos/Stellar`), `stellar` CLI, `stellar-mcp`. Windows는 보류 중입니다.
- 기준 명세: [Stellar UX 명세](../design/ux/stellar-ux-spec.md), [macOS 셸 디자인 가이드](../design/macos-shell-design.md), 영역별 명세(`docs/design/ui/*.md`). 명세가 이상하다고 느끼면 그것도 피드백으로 남깁니다.
- 확인 범위: QA3의 터미널 명령과 출력은 이 빌드의 CLI로 실제로 실행해 맞췄습니다. 앱 화면의 문구와 동작은 앱을 띄우지 않고 코드로만 확인했습니다.
- **아직 안 됨**, **구현 대기** 표시가 있는 항목은 이 빌드에 없는 기능입니다. 해 보지 않아도 됩니다. 다만 적힌 "지금:" 모습과도 다르면(예: 비활성이어야 할 메뉴가 눌림, 앱 종료) 피드백으로 남깁니다.

## 1. 준비

- Apple Silicon Mac, macOS 26 이상, macOS 26 SDK가 있는 Xcode, Rust(`rust-toolchain.toml` 버전), 시스템 Git, `jq`
- Git `user.name`·`user.email`이 설정되어 있어야 샘플과 새 프로젝트에 커밋이 생깁니다. 없으면 커밋 없이 만들어집니다.

저장소 루트에서 한 번 실행합니다.

```bash
# 앱 Debug 빌드 + 샘플 프로젝트 생성 + 앱 실행
./scripts/macos/qa.sh --fresh

# QA3에서 쓰는 CLI·MCP 빌드(Debug 앱 번들에는 CLI가 들어 있지 않습니다)
cargo build -q -p stellar-cli -p stellar-mcp

# 이 터미널에서 git 출력이 pager(less)에 멈추지 않게 합니다
export GIT_PAGER=cat
```

- `qa.sh` 출력의 "샘플 프로젝트:" 줄이 샘플 폴더 경로입니다. 기본값은 `${TMPDIR}/stellar-qa/sample`입니다.
- 앱이 이미 실행 중이면 `qa.sh`가 빌드·샘플 삭제·앱 실행 전에 멈추고 실행 중인 바이너리 경로를 보여 줍니다. 앱을 ⌘Q로 끈 뒤 다시 실행합니다. 같은 번들 ID(`xyz.taeyoon.stellar`)를 쓰는 다른 빌드(main checkout, 다른 worktree)와 `smoke.sh` 검사 중인 프로세스도 실행 중으로 봅니다.
- 앱은 마지막에 열려 있던 프로젝트 창을 복원합니다. 시작 창부터 보려면 프로젝트 창을 모두 닫고 ⌘Q 한 뒤 `--welcome`으로 실행합니다.
- **신뢰는 `--fresh` 뒤에도 이어집니다.** 샘플은 같은 경로에 같은 프로젝트 ID로 다시 만들어지고, 신뢰 목록은 경로와 ID가 같으면 신뢰로 봅니다. 각 단계를 시작할 때 창 부제목이 "Restricted Mode"인지 보고, 아니면 상단 바 제목 메뉴 ▸ Revoke Project Trust로 거둡니다.

| 옵션 | 동작 |
|---|---|
| `--fresh` | 샘플을 지우고 다시 만듭니다. 앞선 QA에서 샘플을 바꿨으면 씁니다. 신뢰 목록은 건드리지 않습니다. |
| `--no-build` | 다시 빌드하지 않고 마지막 Debug 빌드를 씁니다. |
| `--no-launch` | 빌드와 샘플 준비까지만 합니다. `--no-build --no-launch`이고 `--fresh`가 없으면 앱이 떠 있어도 됩니다(샘플을 하나 더 만들 때). |
| `--welcome` | 샘플 없이 앱만 띄웁니다(시작 창 확인). |
| `--name <이름>` | 샘플 폴더 이름(기본 `sample`). 영문·숫자·`.`·`_`·`-`만 쓰고 `.`이나 `-`로 시작할 수 없습니다. |

### 1.1 샘플 프로젝트

`crates/project/examples/qa_sample_project.rs`가 코어 생성 API로 만듭니다(`stellar` 명령에는 프로젝트 생성 기능이 없음). 폴더 구조는 새 프로젝트와 같고(`stellar.yaml`, `stellar.lock.yaml`, `design/`, `assets/`, Git 저장소), 커밋이 2개 있습니다. ID는 폴더 이름에서 정해지므로 같은 이름이면 매번 같습니다.

| 페이지 | View | 주요 레이어 |
|---|---|---|
| Home | Desktop(1440 × 1024) | Header(Logo 타원, Title 텍스트), Hero(Hero Image, Headline 두 줄 한글·영문, Get Started 사각형), Cards(Card 1\~3: 그림자, Thumbnail, Caption), Accent (Hidden) 숨긴 타원, Footer (Locked) 잠근 사각형 |
| Home | Mobile(390 × 844) | Header(Title), Hero Image, Headline("안녕하세요, Stellar"), Button 사각형 |
| Components | Buttons(800 × 600) | Button / Primary·Secondary(테두리 Frame + Label), Icon Group(Fragment: Dot, Square), Divider 선, Hexagon 다각형(채우기·선), Star path |

샘플에는 이 빌드의 캔버스가 그리지 않는 내용이 일부러 들어 있습니다. 레이어 목록에는 보이지만 캔버스에는 없는 것이 정상입니다.

- Card 1\~3과 그 안 내용: 그림자(Shadow) modifier가 있는 Frame은 그리지 않습니다.
- Hero Image: 샘플 생성기가 이미지 레이어와 복사한 JPEG를 잇지 않고, 캔버스는 PNG만 그립니다. 이미지는 QA2-06에서 직접 PNG를 넣어 확인합니다.
- Star: path는 그리지 않습니다.
- Components 페이지의 두 버튼은 일반 Frame입니다. 공유 컴포넌트는 QA3에서 CLI로 만듭니다.

## 2. 진행 순서

| 단계 | 문서 | 내용 | 걸리는 시간(대략) |
|---|---|---|---|
| QA0 | [qa0-shell.md](qa0-shell.md) | 시작 창, 새 프로젝트·열기·Clone, 창 구성, 도구 막대, 상단 바, 메뉴, 설정, 다크 모드, 창 크기·복원, 접근성 | 40분 |
| QA1 | [qa1-view.md](qa1-view.md) | 샘플 보기: Page › View 전환, 레이어 목록, 캔버스 표시, 화면 이동·확대, 선택, 속성 패널 값 | 20분 |
| QA2 | [qa2-edit.md](qa2-edit.md) | 편집(제한 모드 그대로): 도구 10개로 만들기, 이동·크기·반경, 복제·잘라내기·붙여넣기, 순서·숨김·잠금, 이름 변경, 속성 값 입력, 텍스트·한글, 이미지, 자동 저장·실패 표시·복구 | 60분 |
| QA3 | [qa3-git-code-ai.md](qa3-git-code-ai.md) | Git 협업(컴포넌트 원본 diff·병합, 팀원 복제본), CSS 속성과 웹 코드 내보내기, CLI·MCP로 같은 프로젝트 다루기 | 40분 |

QA0부터 진행합니다. 앱이 뜨지 않거나 프로젝트가 열리지 않으면 다음 단계로 넘어가지 말고 그 문제부터 알려 줍니다. QA2와 QA3는 시작할 때 앱을 끄고 `./scripts/macos/qa.sh --no-build --fresh`로 샘플을 새로 만듭니다(각 문서 첫머리에 있음).

## 3. 피드백 형식

스크린샷 하나와 한 줄 설명을 한 묶음으로 남깁니다.

```text
QA2-15 · Option을 누른 채 끌었는데 복제본이 아니라 원본이 옮겨짐
[스크린샷: QA2-15.png]
```

- 앞부분은 확인 목록의 항목 ID입니다. 목록에 없는 문제나 어색한 점은 `QA2-기타`처럼 단계 이름에 "기타"를 붙입니다.
- 한 줄 설명에는 "무엇이 기대와 다른지"만 씁니다. 재현 순서가 항목 설명과 다르면 한 줄을 더 붙입니다.
- 스크린샷은 창 하나만 찍습니다(⇧⌘4 → Space → 창 클릭). 메뉴가 펼쳐진 모습은 ⇧⌘5의 "선택 부분 캡처"를 씁니다. 파일 이름은 항목 ID로 합니다.
- 터미널 단계(QA3)는 터미널 창을 찍거나 출력 한 줄을 그대로 붙입니다.
- 다크 모드·손쉬운 사용 설정에서만 생기는 문제는 그 설정을 한 줄에 적습니다(예: "Dark + 대비 높이기").
- 통과한 항목은 적지 않아도 됩니다. 한 항목에 문제가 여럿이면 문제마다 하나씩 남깁니다.

## 4. 이번 빌드에서 바뀐 점

UX 1단계와 Codex 작업 정리를 합친 빌드입니다. 사용자에게 보이는 변화만 적습니다.

- **제한 모드에서도 편집·저장이 됩니다(D1).** 부제목 "Restricted Mode"는 Git hook·filter, 내장 터미널, MCP·에이전트, 외부 명령과 프로젝트 밖을 가리키는 symlink를 막는다는 뜻입니다. 신뢰는 상단 바 제목 메뉴나 **File ▸ Trust Project…** 로 줍니다.
- **도구 10개**: Move(V), Frame(F), Rectangle(R), Ellipse(O), Line(L), Polygon(P), Text(T), Image(⇧⌘K), Hand(H), Zoom(Z). Pen은 뺐습니다.
- **새 도형 기본 색(D7)**: **Settings… ▸ Canvas**의 Default Shape Fill(처음 #D9D9D9)과 Default Line Tool Color(처음 #000000, 1px). 앱 설정에만 저장하고 프로젝트 파일에는 쓰지 않습니다.
- Frame 밖에서 그리려 하면 금지 커서와 도구 막대 위 한 줄 "Draw inside a Frame."이 나옵니다.
- 캔버스 편집: Shift·⌘ 클릭, 빈 곳에서 끌어 영역 선택(Option은 완전히 포함된 것만), 여러 개 함께 끌기, Option 끌기 복제, 화살표 1px·Shift 10px, 크기 핸들, 모서리 반경 핸들, Line 끝점 핸들.
- 화면 이동·확대: 두 손가락 스크롤, 핀치, Space를 누르는 동안 임시 Hand, Zoom 도구(클릭 확대, Option 클릭 축소).
- 캔버스 우클릭 메뉴, **Edit ▸ Cut·Select All**, **Object ▸ Lock·Hide**, 레이어 행의 눈·자물쇠 단추가 동작합니다.
- 사이드바: 머리 줄 `Page › View` 팝업으로 View를 바꿉니다. 빈 View에는 "Press F to draw a frame, R for a rectangle, or T for text."가 보입니다. 레이어 필터 필드는 동작하지 않아 숨겼습니다.
- 속성 패널: 행 이름이 CSS 속성 이름(`left`, `width`, `background-color`, `border-radius` 등)이고 값도 CSS 문법으로 넣습니다. 레이블 끌기(scrub), ↑↓(Shift ×10), 색 견본, 잘못된 값 진단이 있습니다.
- 자동 저장: 편집을 멈추고 약 1초 뒤, 또는 다른 앱으로 전환하면 저장합니다. 실패하면 부제목에 "⚠︎ Not Saved", 다른 곳에서 먼저 저장했으면 "⚠︎ Not Saved — Changed Elsewhere"가 보입니다. **File ▸ Save Now**(⌘S)도 있습니다.
- 저장에 실패한 창을 닫으면 Try Again·Close Without Saving·Cancel 중 하나를 고릅니다(D6).
- **Clone Project…** 가 실제로 저장소를 복제하고 엽니다.
- 컴포넌트 원본(`design/components/`)은 제한 모드에서도 읽고 감시해 인스턴스를 그립니다.
- `stellar apply`와 `stellar component-source add-frame`도 앱과 같은 규칙으로 잠긴 레이어(와 그 아래) 수정을 거부합니다(D8).
- 지운 것: Document Font…, Object ▸ Create Component from Selection…(D3), 컴포넌트 라이브러리 창, 캔버스 아래 보조 패널(Git·터미널·에이전트), View ▸ Show/Hide Toolbar, 상단 바의 뒤로·앞으로·검색.

## 5. 이번 테스트에서 제외하는 것

이번에는 보지 않습니다. 되는 것처럼 보이더라도 이번 결과에 넣지 않습니다.

- V.10.7 일반 문서의 텍스트 기본 저장: 캔버스 문서는 아직 `design/objects/`의 바이너리 chunk로 저장됩니다. 사람이 읽는 원본은 컴포넌트(`design/components/*.stellar.yaml`)뿐입니다. `stellar text-source export`는 페이지가 하나인 문서만 내보내는 opt-in 명령이고 앱이 읽지 않습니다.
- Metal 렌더 이관: 지금 캔버스는 CoreGraphics로 그립니다. 렌더 품질과 속도의 최종 판단은 하지 않습니다.
- 앱 안 컴포넌트 UX(D4): Assets 탭, 캔버스로 끌어 배치, 메인 컴포넌트 편집은 명세만 있습니다. 이번에는 CLI로 컴포넌트를 만들고 배치합니다(QA3).
- CSS Inspector 전체(IX.11): 글꼴·크기 같은 텍스트 속성, flex·grid 레이아웃, `%`·`fit-content`·`var()`, 그림자 등은 없습니다. 지금 있는 행만 확인합니다.

## 6. 아직 안 되는 것(공통)

단계 문서마다 해당 영역의 "아직 안 됨"이 따로 있습니다. 여기에는 여러 단계에 걸친 것만 적습니다.

- 앱 안 Git 화면(변경 목록·커밋), 에이전트 탭, 코드 탭, 내장 터미널: 없습니다. Git과 CLI는 터미널에서 씁니다.
- 앱 밖 변경 반영: 터미널의 `stellar apply`나 `git checkout`·`git pull`로 캔버스 문서가 바뀌어도 열린 창은 다시 읽지 않습니다(XIII.3.1). 창을 닫고 다시 열어야 보입니다. 컴포넌트 원본(`design/components/`)만 바로 반영됩니다.
- 캔버스 문서의 Git 병합: 두 브랜치가 문서를 고치면 `design/current`가 Git 충돌로 남습니다. `stellar semantic-merge`는 결과를 보고만 하고 파일을 쓰지 않습니다.
- 최근 프로젝트 목록: 항상 "No Recent Projects"입니다.
- 같은 프로젝트를 창 두 개로 열기: 같은 폴더를 다시 열면 기존 창이 앞으로 옵니다.
- Undo·Redo 메뉴 제목에 동작 이름이 붙지 않습니다(항상 "Undo"·"Redo").
- MCP는 저장된 내용 읽기·비교·검증만 합니다. 쓰기 tool과 실행 중인 앱에 붙는 `--attach`는 아직 없습니다.
- `stellar` 명령 설치: 앱 메뉴 항목 "Set Up the stellar Command…"는 어느 메뉴에도 연결되어 있지 않습니다(모든 빌드). 첫 실행 제안은 Debug 번들에 CLI가 없어 나오지 않습니다. `target/debug/stellar`를 씁니다.

## 7. 정리

- 샘플 지우기: `rm -rf "${TMPDIR%/}/stellar-qa"`
- 창 상태(프레임, pane 폭, 툴바 구성), 첫 실행 기록, 새 도형 기본 색은 앱 사용자 기본값(`xyz.taeyoon.stellar`)에, 창 복원 상태는 `~/Library/Saved Application State/xyz.taeyoon.stellar.savedState`에 있습니다. 처음 상태부터 다시 보려면 앱을 끈 뒤 `defaults delete xyz.taeyoon.stellar`와 위 폴더 삭제를 합니다. 같은 번들 ID를 쓰는 다른 Stellar 빌드의 설정도 함께 지워집니다.
- 프로젝트 신뢰 목록은 `~/Library/Application Support/xyz.taeyoon.stellar/trust/trusted-projects.yaml`에 있습니다. 샘플을 지우기 전에 앱의 **Revoke Project Trust**로 신뢰를 거두는 것이 안전합니다.
