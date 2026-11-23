# 컴포넌트별 텍스트 원본(v1~v3, 읽기 전용)

> **2026-11-23 개정(ADR-0077):** 쓰기 버전은 v4입니다. 파일 키는 document 속성 registry 이름·순서이고 값은 CSS 문법입니다. 이 문서의 문법은 읽기 전용 옛 버전의 기록이며, v4 문법은 [`text-source-v4.md`](text-source-v4.md)에 있습니다.

`stellar component-source`는 빈 공개 인터페이스와 자유 배치 Frame 트리로 이루어진 컴포넌트를 독립 YAML 파일에 저장합니다. 지정한 폴더가 원본이고 `check`는 여기서 실제 `ComponentLibrary`를 재조립합니다. 예를 들어 다음 명령은 이름이 같은 컴포넌트 두 개를 생성합니다.

```sh
mkdir -p design/components
stellar component-source create design/components 버튼 0195c530-3acd-7bc1-8414-c213f61cb965 0195c530-3acd-7bc1-8414-c213f61cb966
stellar component-source create design/components 버튼 0195c530-3acd-7bc1-8414-c213f61cb967 0195c530-3acd-7bc1-8414-c213f61cb968
stellar component-source check design/components
```

위 명령이 만드는 파일은 `버튼-8c5619572809.stellar.yaml`과 `버튼-bc6dfb3d3aea.stellar.yaml`입니다.

- 파일명은 `<컴포넌트 이름>-<접미사>.stellar.yaml`입니다. 접미사는 component StableId 16바이트의 SHA-256 digest 앞 12자리 소문자 hex입니다. 파일 내부의 `component.id`와 `component.root`는 전체 StableId입니다. 이름은 파일 이름으로 쓸 수 있어야 하고 전체 파일명은 UTF-8 255바이트 이하여야 합니다. 중복 ID와 대소문자 무시 파일명 충돌은 거부합니다.
- 쓰기 버전은 `schema_version: "3"` 하나입니다([ADR-0061](../adr/ADR-0061-text-source-format-and-layout.md) 결정 4). 최상위 키는 `format: "stellar-component"`, `schema_version`, `component`, `root_node`와 자식이 있을 때의 `children`입니다. `component`는 `id`, `name`, `root`를 가집니다. `root_node`와 `children`의 각 항목은 공통 노드 필드(`id`, `parent`, `order_key`, `visible`, `locked`, `display_name`, `kind`, `payload`)를 가진 Frame 레코드입니다. 루트는 부모가 없는 Frame이어야 합니다.
- `children`은 루트 아래 모든 Frame을 전체 StableId 오름차순으로 기록합니다. 부모는 `parent`, 형제의 화면 순서는 `order_key`로 정하므로 배열에서 손자가 부모보다 먼저 나와도 같은 트리가 됩니다. 중복 ID, 다른 컴포넌트의 부모, 순환, 분리된 노드, 같은 부모의 중복 순서 키는 오류입니다.
- Frame `payload`는 영속 속성 하나를 Stellar 파일 키 하나·한 줄로 다음 고정 순서에 씁니다(키와 속성 패널 행·웹 CSS 출력의 대응은 [텍스트 View 원본](text-view-source-v1.md)의 대응 표): `background`, `border-bottom-left-radius`, `border-bottom-right-radius`, `border-color`, `border-top-left-radius`, `border-top-right-radius`, `border-width`, `height`, `layout-mode`, `width`, `clipping`, `transform`. 모양과 크기 키는 이름 오름차순이고 `clipping`·`transform`은 끝입니다. 길이와 반경은 논리 픽셀 f32, `transform`은 `a b c d tx ty`, `clipping`은 `disabled` 또는 `frame-bounds`, `background`와 `border-color`는 `none` 또는 `srgb 1 0 0 1`처럼 색 공간과 비연관 RGBA, `border-width`는 `none` 또는 비음수 f32, `layout-mode`는 `none`입니다. `border-color`와 `border-width`는 함께 `none`(테두리 없음)이거나 함께 값이어야 합니다.
- 공통 조상에서 서로 다른 속성을 바꾼 두 브랜치는, 바뀐 두 줄 사이에 바뀌지 않은 줄이 있으면 일반 `git merge`로 합쳐집니다. 위 순서는 자주 따로 바뀌는 속성 사이에 다른 줄이 오도록 정했습니다. 예를 들어 이동(`transform`)과 폭·높이, 폭과 높이, 테두리 색과 두께, 왼쪽 위와 왼쪽 아래 모서리가 그렇습니다. 바로 이웃한 줄(예: 왼쪽 위와 오른쪽 위 모서리)을 양쪽에서 바꾸면 Git 텍스트 충돌로 남습니다. `stellar semantic-merge`는 세 작업 트리를 비교해 두 변경이 속성 단위로 합쳐지는지 보고하는 읽기 전용 명령이며 파일을 고치지 않으므로, 이 충돌은 지금 사용자가 직접 풀어야 합니다. 자동 해결은 Git merge driver(XXIV.2.1) 이후입니다. 충돌 마커가 남은 원본은 제한 loader가 읽지 않습니다. 같은 속성을 서로 다른 값으로 바꾸면 양쪽 모두 충돌입니다.
- 이미 커밋된 v1(루트 하나)과 v2(`children` 포함)는 읽기 전용입니다. Frame은 `size: "너비 높이"`, `corner_radius: "좌상 우상 우하 좌하"`, 색·clipping mapping으로 기록되어 있습니다. 무변경 요청은 기존 bytes를 보존하고, 실제로 편집하면 v3로 다시 씁니다. 같은 번호를 재정의하기 전 작업 트리에서만 쓰인 v3 문법(`corner_radius` 한 줄, 두께와 색을 묶은 `border` 한 줄)은 알 수 없는 키로 거부합니다.
- `check`는 지정 폴더 바로 아래의 모든 `.stellar.yaml`을 제한 YAML로 검사하고, 이름과 접미사, 전체 component·Frame ID, tree 소유권이 일치하는 정의를 StableId 순으로 반환합니다. `node_count`에는 루트도 포함됩니다. 파일당 1 MiB, 폴더당 4,096개까지 허용합니다. 캐시, 중앙 색인이나 generation은 필요하지 않습니다.
- `resize <파일> <source_hash> <너비> <높이>`는 root Frame의 크기를 논리 픽셀로 바꿉니다. `source_hash`는 `create` 또는 `check` 결과에서 얻습니다. 크기는 유한한 비음수 f32이며 결과 JSON에서는 최단 왕복 문자열입니다. 실제 변경은 해당 파일 하나를 결정론적으로 다시 씁니다. 무변경 요청은 주석과 서식을 포함한 기존 bytes를 보존합니다. 협조적 CLI writer는 원본 폴더 잠금을 공유하고, 외부 변경으로 hash가 다르면 종료 코드 75로 실패합니다. 대상 폴더가 Stellar 프로젝트 안에 있으면(조상 폴더에 `stellar.yaml`) `create`·`resize`·`set-fill`·`add-frame`은 그 프로젝트의 저장 잠금을 먼저 잡고, generation에 component-library chunk가 있으면 이중 원본이 되므로 종료 코드 69로 거부합니다. 프로젝트 manifest를 읽을 수 없으면 쓰지 않습니다.

```sh
source_info=$(stellar component-source check design/components | jq -c '.result.components[0]')
stellar component-source resize "$(printf '%s' "$source_info" | jq -r .file)" "$(printf '%s' "$source_info" | jq -r .source_hash)" 248.5 44
git diff -- design/components
```

새 자식은 지정한 부모의 마지막 형제로 추가합니다. `parent-id`는 같은 파일의 루트 또는 기존 자식이고, 새 ID는 라이브러리 전체에서 고유해야 합니다. 반환한 hash로 이어서 손자를 추가할 수 있습니다.

```sh
file=design/components/버튼-8c5619572809.stellar.yaml
root=0195c530-3acd-7bc1-8414-c213f61cb966
group=0195c530-3acd-7bc1-8414-c213f61cb970
leaf=0195c530-3acd-7bc1-8414-c213f61cb969
hash=$(stellar component-source check design/components | jq -r '.result.components[0].source_hash')
hash=$(stellar component-source add-frame "$file" "$hash" "$root" "$group" Content 180 48 | jq -r '.result.source_hash')
stellar component-source add-frame "$file" "$hash" "$group" "$leaf" Icon 24 24
```

`set-fill <파일> <source_hash> display-p3 1.2 -0.1 0.5 0`처럼 색 공간과 비연관 RGBA를 지정하면 컴포넌트 루트의 Fill을 저장합니다. `srgb`와 `display-p3`를 지원합니다. RGB는 유한한 확장 범위 값을 허용하고 알파는 0~1입니다. `set-fill <파일> <source_hash> none`은 Fill을 제거합니다. 알파가 0인 색도 원래 색 공간과 RGB를 보존합니다. `resize`와 `set-fill`은 root가 잠겨 있으면 무변경 요청도 종료 코드 65로 거부하고 원본 bytes를 보존합니다. `add-frame`도 parent Frame이나 그 조상이 잠겨 있으면 종료 코드 65로 거부하고 원본 bytes를 보존합니다(사용자 결정 D8, `stellar apply`와 같은 잠금 규칙). 원본 hash가 바뀌면 잠금 검사보다 먼저 종료 코드 75로 실패합니다.

파일 하나를 직접 고칠 때도 `component.root`와 `root_node.id`는 같은 전체 ID여야 합니다. 이름을 수동으로 변경하면 같은 변경에서 파일명도 새 이름·기존 접미사로 맞춥니다. 이름 부분만 다르고 접미사가 정의 ID와 맞는 파일은 ID로 조립하고, `check`와 `query <프로젝트> components`가 `component-file-name-mismatch` 경고로 정규 파일명을 알려 줍니다. 접미사가 다르면 다른 정의의 파일일 수 있으므로 거부합니다. CLI는 파일명 변경이나 여러 파일의 원자 저장을 제공하지 않습니다.

일반 캔버스 문서는 아직 generation과 chunk에 저장되며, 기존 바이너리 라이브러리 전체 변환과 다중 파일 복구는 후속 범위입니다. 등록된 Property·Slot·token·state machine, Frame 이외의 자식 종류, 외부 layout 참조, Path clip, 알 수 없는 필드나 버전을 만나면 쓰기를 거부합니다. [ADR-0059](../adr/ADR-0059-text-source-files-with-chunk-cache.md)

## 앱에서 생성

macOS 앱에는 컴포넌트를 만드는 메뉴가 없습니다. 선택에서 만들던 **Object → Create Component from Selection…**은 원본 형식이 Text·Shape를 담기 전까지 제거했습니다(사용자 결정 D3). 앱에서 찾고 배치하고 편집하는 UX는 별도 명세(D4) 뒤에 구현합니다. 빈 컴포넌트는 CLI `component-source create`로 만듭니다.

## 배치된 인스턴스

macOS 프로젝트 창은 신뢰 여부와 무관하게 프로젝트 session마다 코어의 파일 감시기(FSEvents) 하나로 `design/components/`와 바로 아래의 `*.stellar.yaml`을 감시합니다. 앱·CLI·내장 터미널의 편집이나 Git 복원 뒤 그 프로젝트의 모든 창이 원본을 다시 읽어 이미 배치한 인스턴스의 표시와 클릭 영역을 갱신합니다. 창이 계속 key 창이어도 반영되며, 문서의 미저장 편집·선택·Undo 기록은 유지됩니다. 감시는 프로젝트 폴더 안만 봅니다. 내부 심볼릭 링크의 실제 대상만 감시 경로 밖에서 바꾼 경우에는 신뢰를 다시 확인하거나 프로젝트를 다시 열 때 반영됩니다.

인스턴스 payload에는 정의 참조와 override가 있고, 위치는 해당 노드 소유의 Transform modifier에 있습니다. 원본을 찾을 수 없으면 `stellar validate`가 인스턴스와 정의 ID를 포함하는 `missing-component-definition` 진단을 냅니다. 정의 이름을 다른 파일에 다시 사용하는 것은 이 참조를 바꾸지 않습니다.

## 프로젝트에서 조회

프로젝트 루트 아래 `design/components/`에 원본을 두면 `stellar query . components`와 신뢰된 headless MCP의 `stellar://components`가 같은 라이브러리를 읽습니다. 다른 폴더의 원본은 기존 `component-source check <폴더>`로 조회할 수 있습니다. 각 항목은 전체 StableId, 이름, 루트 ID, 루트의 논리 픽셀 크기와 `background`(채우기가 없으면 JSON `null`, 색이 있으면 YAML과 같은 한 줄 색 문자열. 예: `"display-p3 1.2 -0.1 0.5 0"`), 루트를 포함한 Frame 수, 원본 bytes의 `source_hash`를 담습니다. `source_hash`는 그대로 `resize`와 `add-frame`에 전달할 수 있습니다.

`revision`은 프로젝트의 바이너리 generation을 가리킵니다. Git checkout이나 미커밋 YAML 편집은 generation을 올리지 않으므로 같은 MCP 연결에서도 매 요청마다 파일을 다시 읽습니다. 응답의 `component_source_hash`는 읽은 원본 파일 이름과 bytes 전체의 hash이므로, generation이 같아도 YAML이 바뀌면 달라집니다. 독립된 파일별 버전은 각 항목의 `source_hash`를 사용합니다. 여러 파일을 동시에 편집할 때 단일 시점의 스냅샷을 보장하려면 다중 파일 journal이 필요합니다.

정의별 `stellar query <프로젝트> component <id>`와 headless `stellar://components/{component}{?cursor,limit}`는 같은 저장 트리를 읽습니다. 응답은 `revision`, `component`, `root`, `name`, `source_hash`, `component_source_hash`, `nodes`를 담습니다. 각 노드의 메타데이터와 Frame payload는 v1·v2 저장 데이터에도 v3의 한 속성 한 줄 키와 값 표현을 사용합니다. 표시·잠금은 JSON bool이고 루트 부모는 JSON null입니다. payload의 수치와 색은 원본과 같은 문자열입니다. MCP는 기존 페이지 개수·바이트 상한을 적용하고 세대·파일 hash·프로젝트 및 컴포넌트 ID에 cursor를 묶습니다. 읽기 전용 요청에는 파일 경로가 포함되지 않습니다.

symlink 읽기는 프로젝트의 신뢰 상태 하나로 정합니다(기술 아키텍처 §19.3, [ADR-0061](../adr/ADR-0061-text-source-format-and-layout.md) 결정 8). 신뢰한 프로젝트는 `design/components/`나 그 안 파일이 프로젝트 밖을 가리켜도 읽으며, 이때 결과는 `component-source check <폴더>`와 같습니다. 신뢰하지 않은 프로젝트는 프로젝트 밖 target의 내용을 읽지 않고 오류를 반환합니다. CLI와 headless MCP는 앱과 같은 OS 사용자 데이터의 신뢰 저장소를, 앱은 창의 신뢰 상태를 씁니다. 신뢰하지 않은 프로젝트의 앱 창도 프로젝트 안 원본은 읽어 인스턴스를 그리므로 CLI·MCP와 같은 정의를 봅니다. 정의 폴더가 없거나 해당 확장자의 파일이 없으면 `entries`는 빈 배열이고 `chunk_count`는 generation의 `component-library` 포인터 수입니다. 바이너리 라이브러리와 YAML 원본이 함께 있으면 오류를 반환합니다. 한 요청에 읽는 전체 원본은 64 MiB까지이고, MCP 리소스는 기존 512 KiB 본문 상한을 적용합니다. 파일·캐시·generation을 작성하지 않습니다.

MCP에서 `stellar://components`를 읽으면 StableId 순서의 첫 페이지를 받습니다. `entries`는 현재 페이지이고 `total`은 전체 개수입니다. `next_cursor`가 문자열이면 `stellar://components?cursor=<값>`으로 이어 읽고, `null`이면 마지막 페이지입니다. `limit`을 생략하면 기본 500개이며, 명시할 수 있는 최댓값은 2,000개입니다. 모든 경우에 응답 바이트 상한을 적용합니다. 도중에 generation 또는 컴포넌트 파일 이름·내용이 바뀌면 이전 커서를 거부하므로 첫 페이지부터 다시 읽습니다.

## 두 작업 트리 비교

`stellar semantic-diff <base> <target>`은 같은 프로젝트의 두 Git 작업 트리에서 저장 문서와 `design/components/`의 공유 정의를 함께 비교합니다. 각 변경의 `object`는 전체 StableId이고, 공유 컴포넌트에 속한 변경은 `component`에 정의의 전체 ID, `document`에 `null`을 반환합니다. 이름과 파일명을 함께 바꾸면 같은 컴포넌트의 이름 변경 한 건입니다. Frame 크기는 속성 변경, Frame 생성·삭제는 해당 노드의 추가·삭제로 표시합니다. 주석·YAML 서식만 다르면 의미 변경은 없습니다.

`diff-summary`와 두 작업 트리를 공통 base와 비교하는 `semantic-merge`도 이 정보를 사용합니다. 같은 Frame의 폭과 높이를 별개로 바꾸면 두 변경을 함께 반환하고, 한 축을 양쪽에서 서로 다른 값으로 바꾸면 해당 축의 속성 충돌을 반환합니다. 이 명령들은 비교할 때 작업 파일이나 Git index를 수정하지 않습니다. headless MCP의 `document.semantic_diff`도 고정 HEAD의 Git blob과 현재 프로젝트의 YAML을 같은 규칙으로 비교합니다. 커서는 기준 commit, 저장 generation, 현재 컴포넌트 원본 집합의 hash와 결합됩니다. HEAD 안의 컴포넌트 폴더나 파일이 symlink이면 비교를 거부합니다. 앱의 Stage/Unstage는 현재 저장 generation의 문서를 대상으로 합니다.

headless MCP 세션은 검증한 고정 HEAD의 의미 객체를 재사용합니다. 파일 이름 집합과 저장 generation이 같고 한 파일의 bytes만 바뀌면 그 정의만 다시 투영하고 나머지 객체와의 ID 충돌을 검사합니다. 이름 변경, 파일 추가·삭제, 여러 파일 편집은 전체 검증을 수행합니다.

## 검증

`cargo test -p stellar-cli --test component_source`는 기존 generation을 가진 프로젝트에서 위 두 파일을 생성·커밋하고, 각각의 브랜치에서 한 파일만 수정합니다. 일반 `git merge --no-ff` 후 새 clone에서 `check`가 양쪽 root 크기(240.5 × 64, 32 × 55), 전체 ID와 파일 hash를 재현하며 작업 트리는 깨끗합니다. 한 파일의 편집 전후에 다른 파일 bytes를 직접 비교하고, clone 뒤 기존 프로젝트 설정·generation의 hash가 모두 같은지 확인합니다. 같은 테스트가 component/root 교차 중복 ID, 오래된 hash, 미지원 중첩 Property·Slot·state·token·알 수 없는 Frame 필드와 잘못된 root 참조의 원본 보존, 이름 변경 후 참조를 확인합니다.

`git_and_semantic_merge_of_two_clones_separate_frame_width_and_height`는 같은 v3 Card의 폭을 90에서 144로, 높이를 60에서 96으로 두 clone에서 각각 바꿉니다. 기본 `git merge` 뒤 새 clone의 `stellar query . components`에서 같은 ID와 144 × 96 크기를 확인하고, 두 브랜치가 같은 폭을 144와 128로 바꾼 경우에는 해당 파일에 충돌이 남는지 확인합니다. `frame_corners_merge_by_line_in_git_and_by_property_in_semantic_merge`는 이웃하지 않은 두 모서리 변경이 일반 `git merge`로 합쳐지고, 이웃한 두 모서리 변경은 Git 충돌로 남으며 읽기 전용 `semantic-merge` 보고에서는 두 속성 변경으로 합쳐지고, 같은 모서리의 서로 다른 값은 그 속성만 충돌로 보고되는지 확인합니다. `semantic-merge`는 Git 충돌 파일을 고치지 않습니다.

`cargo test -p stellar-project frame_and_component_library_bytes_survive_a_source_round_trip`는 회전·이동, Display P3 배경·테두리, 서로 다른 모서리 반경, 표시·잠금 값을 설정하고 YAML 왕복 전후 실제 라이브러리의 canonical binary bytes를 비교합니다.

`nested_frames_survive_a_component_rename_and_fresh_clone`은 형제 두 개와 손자 하나를 CLI로 생성합니다. 루트 resize 시 바꾼 치수의 줄만 달라지는지, 두 번째 컴포넌트 파일의 bytes가 같은지 검사합니다. 이름 변경을 실제 Git rename commit으로 만들고 새 clone의 라이브러리 결과를 비교합니다. `nested_frame_errors_preserve_every_component_file`은 중복 ID·다른 파일의 부모·미지원 종류·오래된 hash 요청 전후 모든 원본 hash를 검사합니다. 도메인 검사에서는 재조립한 정의를 참조하는 두 개의 scene 인스턴스에서 동일한 자식 순서와 손자의 Frame payload를 각각 확인합니다.
