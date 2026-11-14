# 실사용 QA

macOS 앱을 직접 써 보며 화면과 동작을 확인하는 절차입니다. 자동 검사(`scripts/macos/smoke.sh`, 영역별 `--*-ui-check`)가 화면 밖에서 확인하는 구조·접근성 값과 달리, 여기서는 사람이 실제 창에서 보고 느끼는 것을 확인합니다.

- 대상: macOS 앱(`apps/macos/Stellar`). Windows는 보류 중입니다.
- 기준: [macOS 셸 디자인 가이드](../design/macos-shell-design.md)와 영역별 명세(`docs/design/ui/*.md`). 확인 목록의 "보여야 하는 것"은 이 명세에서 왔습니다. 명세가 이상하다고 느끼면 그것도 피드백으로 남깁니다.
- 확인 목록은 "무엇을 해 보고 → 무엇이 보여야 하는지"로 씁니다. 아직 구현하지 않은 항목은 **구현 대기**와 해당 WBS ID(구현 계획 `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`의 행)로 표시하고, 지금 빌드에서 대신 보이는 모습을 함께 적습니다.

## 1. 단계

| 단계 | 문서 | 내용 | 지금 확인할 수 있는 범위 |
|---|---|---|---|
| QA0 | [qa0-shell.md](qa0-shell.md) | 셸 체감: 시작 창, Open Project, 창 구성, 하단 도구 막대, 상단 바, 메뉴·단축키, 다크 모드, 창 크기, 접근성 기본 | 대부분 |
| QA1 | [qa1-view.md](qa1-view.md) | 문서 보기: 페이지·View·레이어 목록, 캔버스 렌더, 이동·확대, 선택, 인스펙터 | 활성 페이지·View와 Layers, 고정 Frame 표시·탐색 |
| QA2 | [qa2-edit.md](qa2-edit.md) | 편집: 만들기 도구, 이동·크기, Undo, 이름·순서, 텍스트, 저장 | Free Frame 작성·이동·치수, 레이어 이름, Undo·Redo와 저장·재열기 |

QA0부터 진행합니다. 앞 단계에서 막히는 문제(앱이 뜨지 않음, 프로젝트가 열리지 않음)가 있으면 다음 단계로 넘어가지 않고 먼저 알려 줍니다.

## 2. 준비

- Apple Silicon Mac, macOS 26 이상, macOS 26 SDK가 있는 Xcode, Rust(`rust-toolchain.toml` 버전), 시스템 Git
- 샘플 프로젝트의 커밋을 만들려면 Git `user.name`·`user.email`이 설정되어 있어야 합니다. 없으면 커밋 없이 샘플만 만듭니다.

## 3. 실행

저장소 루트에서 실행합니다.

```bash
./scripts/macos/qa.sh
```

1. 앱을 Debug로 빌드합니다(`scripts/macos/smoke.sh`와 같은 xcodebuild 설정, 결과는 `apps/macos/build/`).
2. 저장소 밖 임시 폴더 `${TMPDIR}/stellar-qa/sample`에 샘플 프로젝트를 만듭니다. 이미 있으면 그대로 씁니다. 생성 중 실패하면 폴더를 남기지 않습니다. 있던 폴더에 `stellar.yaml`·`.git`·`design/current` 중 하나라도 없으면 멈추고 `--fresh`를 안내합니다.
3. 샘플 프로젝트로 앱을 엽니다. 출력의 "샘플 프로젝트:" 줄이 샘플 폴더 경로입니다(Open Project에서 이 폴더를 고릅니다). 새로 만들 때와 있던 샘플을 쓸 때 모두 이 줄이 나옵니다.

| 옵션 | 동작 |
|---|---|
| `--fresh` | 샘플을 지우고 다시 만듭니다. 앞선 QA에서 샘플을 바꿨거나 생성 코드가 바뀌었을 때 씁니다. |
| `--no-build` | 다시 빌드하지 않고 마지막 Debug 빌드를 씁니다. 같은 빌드로 다시 실행할 때 씁니다. |
| `--no-launch` | 빌드와 샘플 준비까지만 합니다. |
| `--welcome` | 샘플 없이 앱만 띄웁니다(시작 창 확인). |
| `--name <이름>` | 샘플 폴더 이름을 바꿉니다(기본 `sample`). 샘플을 여러 개 만들 때 씁니다. 영문·숫자·`.`·`_`·`-`만 쓰고, `.`이나 `-`로 시작할 수 없습니다. |

- 앱이 이미 실행 중이면 스크립트가 빌드·샘플 삭제(`--fresh`)·앱 실행 전에 멈추고, 실행 중인 바이너리 경로를 보여 줍니다. 앱을 종료(⌘Q)한 뒤 다시 실행합니다. 실행 중에 다시 빌드하면 앱 번들을 덮어쓰고, `--fresh`는 열려 있는 샘플을 지우며, 열기 요청이 이미 떠 있는 다른 빌드로 갈 수 있기 때문입니다. 이 worktree 빌드뿐 아니라 같은 번들 ID(`xyz.taeyoon.stellar`)를 쓰는 다른 빌드(main checkout, 다른 worktree)와 `smoke.sh` 검사 중인 프로세스도 실행 중으로 봅니다. `--no-build --no-launch`이고 `--fresh`가 없을 때만 확인하지 않습니다.
- 앱은 마지막에 열려 있던 프로젝트 창을 복원합니다. 시작 창부터 보려면 프로젝트 창을 모두 닫고 종료한 뒤 `--welcome`으로 실행합니다.

### 3.1 샘플 프로젝트

`crates/project/examples/qa_sample_project.rs`가 코어 생성 API로 만듭니다(`stellar` 명령에 아직 프로젝트 생성 기능이 없음). 폴더 구조는 실제 새 프로젝트와 같습니다(`stellar.yaml`, `stellar.lock.yaml`, `design/`, `assets/`, Git 저장소, 커밋 2개).

| 페이지 | View | 주요 레이어 |
|---|---|---|
| Home | Desktop(1440 × 1024) | Header(Logo 타원, Title 텍스트), Hero(Hero Image, Headline 두 줄 한글·영문, Get Started 사각형), Cards(Card 1\~3: 그림자, Thumbnail, Caption), Accent (Hidden) 숨김 타원, Footer (Locked) 잠금 사각형 |
| Home | Mobile(390 × 844) | Header(Title), Hero Image, Headline("안녕하세요, Stellar"), Button |
| Components | Buttons(800 × 600) | Button / Primary·Secondary(테두리 Frame + Label), Icon Group(Fragment: Dot, Square), Divider 선(Stroke), Hexagon 다각형(Fill·Stroke), Star path |

- 도형·텍스트·이미지는 Transform·Fill modifier로 위치와 색을 가집니다. Divider 선은 Fill 없이 Stroke(두께 1), Hexagon은 Fill과 Stroke(두께 2)를 함께 가집니다. 인스펙터 Modifiers 섹션 확인용입니다.
- 이미지 원본은 저장소의 README 히어로 이미지(`assets/docs/hero.jpg`)를 `assets/<prefix>/<hash>.jpg`로 복사한 것입니다.
- 한계: Component 정의·인스턴스는 문서 저장 포맷에 아직 표현할 수 없어 넣지 못했습니다. "Components" 페이지의 버튼은 일반 Frame입니다. 에셋 catalog(asset id → 파일)도 아직 프로젝트에 저장되지 않아 이미지 레이어의 asset 참조는 복사한 파일과 이어져 있지 않습니다.

## 4. 진행 방식

1. 단계 문서를 열고 항목 순서대로 해 봅니다.
2. "보여야 하는 것"과 같으면 넘어갑니다. 통과 항목은 따로 적지 않아도 됩니다.
3. 다르면 5절 형식으로 피드백을 남깁니다. 한 항목에 문제가 여럿이면 문제마다 하나씩 남깁니다.
4. **구현 대기** 항목은 건너뜁니다. 다만 "지금:"에 적은 모습과도 다르게 동작하면(예: 비활성이어야 할 메뉴가 눌림, 앱 종료) 피드백으로 남깁니다.
5. 목록에 없는 문제나 어색한 점은 `QA0-기타`처럼 단계 이름에 "기타"를 붙여 남깁니다.

## 5. 피드백 형식

스크린샷 하나와 한 줄 설명을 한 묶음으로 남깁니다.

```text
QA0-17 · 사이드바를 닫으면 하단 도구 막대가 캔버스 가운데에서 왼쪽으로 치우침
[스크린샷: QA0-17.png]
```

- 첫 부분은 확인 목록의 항목 ID입니다. 목록에 없는 문제는 `QA0-기타`입니다.
- 한 줄 설명은 "무엇이 기대와 다른지"만 씁니다. 재현 순서가 항목 설명과 다르면 한 줄을 더 붙입니다.
- 스크린샷은 창 하나만 찍습니다(⇧⌘4 → Space → 창 클릭). 메뉴가 펼쳐진 모습이 필요하면 ⇧⌘5의 "선택 부분 캡처"를 씁니다. 파일 이름은 항목 ID로 합니다.
- 다크 모드·손쉬운 사용 설정에서만 생기는 문제는 그 설정을 한 줄에 적습니다(예: "Dark + 대비 높이기").

## 6. 정리

- 샘플 프로젝트 지우기: `rm -rf "${TMPDIR%/}/stellar-qa"`
- 창 상태(프레임, pane 폭, 툴바 구성)와 첫 실행 기록은 앱 사용자 기본값(`xyz.taeyoon.stellar`)에, 창 복원 상태는 `~/Library/Saved Application State/xyz.taeyoon.stellar.savedState`에 있습니다. 첫 실행 상태부터 다시 보려면 앱을 종료한 뒤 `defaults delete xyz.taeyoon.stellar`와 위 폴더 삭제를 합니다. 같은 번들 ID를 쓰는 다른 Stellar 빌드의 설정도 함께 지워집니다.
