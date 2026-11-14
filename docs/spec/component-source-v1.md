# 컴포넌트별 텍스트 원본 v1·v2

`stellar component-source`는 빈 공개 인터페이스와 자유 배치 Frame 트리로 이루어진 컴포넌트를 독립 YAML 파일에 저장합니다. 지정한 폴더가 원본이고 `check`는 여기서 실제 `ComponentLibrary`를 재조립합니다. 예를 들어 다음 명령은 이름이 같은 컴포넌트 두 개를 생성합니다.

```sh
mkdir -p design/components
stellar component-source create design/components 버튼 0195c530-3acd-7bc1-8414-c213f61cb965 0195c530-3acd-7bc1-8414-c213f61cb966
stellar component-source create design/components 버튼 0195c530-3acd-7bc1-8414-c213f61cb967 0195c530-3acd-7bc1-8414-c213f61cb968
stellar component-source check design/components
```

위 명령이 만드는 파일은 `버튼-8c5619572809.stellar.yaml`과 `버튼-bc6dfb3d3aea.stellar.yaml`입니다.

- 파일명은 `<컴포넌트 이름>-<접미사>.stellar.yaml`입니다. 접미사는 component StableId 16바이트의 SHA-256 digest 앞 12자리 소문자 hex입니다. 파일 내부의 `component.id`와 `component.root`는 전체 StableId입니다. 이름은 파일 이름으로 쓸 수 있어야 하고 전체 파일명은 UTF-8 255바이트 이하여야 합니다. 중복 ID와 대소문자 무시 파일명 충돌은 거부합니다.
- v1(루트 하나)의 최상위 키는 `format: "stellar-component"`, `schema_version: "1"`, `component`, `root_node`입니다. `component`는 `id`, `name`, `root`를 가집니다. `root_node`는 단일 View 원본과 같은 Frame 레코드입니다. 크기·affine·배경색·테두리·모서리·clipping·표시/잠금·순서 키를 보존합니다. 루트는 부모가 없는 Frame이어야 합니다. [Frame 레코드 문법](text-view-source-v1.md)
- v2는 v1의 루트와 컴포넌트 필드를 그대로 쓰고 최상위 `children`에 루트 아래 모든 Frame을 기록합니다. 각 항목은 `root_node`와 같은 필드를 가집니다. 파일의 배열은 전체 StableId 오름차순으로 씁니다. 부모는 `parent`, 형제의 화면 순서는 `order_key`로 정하므로 배열에서 손자가 부모보다 먼저 나와도 같은 트리가 됩니다. 중복 ID, 다른 컴포넌트의 부모, 순환, 분리된 노드, 같은 부모의 중복 순서 키는 오류입니다. 첫 자식을 추가한 파일만 v2가 됩니다. v1 전용 클라이언트는 v2를 거부합니다.
- `check`는 지정 폴더 바로 아래의 모든 `.stellar.yaml`을 제한 YAML로 검사하고, 이름과 접미사, 전체 component·Frame ID, tree 소유권이 일치하는 정의를 StableId 순으로 반환합니다. `node_count`에는 루트도 포함됩니다. 파일당 1 MiB, 폴더당 4,096개까지 허용합니다. 캐시, 중앙 색인이나 generation은 필요하지 않습니다.
- `resize <파일> <source_hash> <너비> <높이>`는 root Frame의 크기를 논리 픽셀로 바꿉니다. `source_hash`는 `create` 또는 `check` 결과에서 얻습니다. 크기는 유한한 비음수 f32이며 결과 JSON에서는 최단 왕복 문자열입니다. 실제 변경은 해당 파일 하나를 결정론적으로 다시 씁니다. 무변경 요청은 주석과 서식을 포함한 기존 bytes를 보존합니다. 협조적 CLI writer는 원본 폴더 잠금을 공유하고, 외부 변경으로 hash가 다르면 종료 코드 75로 실패합니다.

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

`remove-frame <파일> <source_hash> <node-id>`는 지정한 자식 Frame과 모든 하위 Frame을 같은 파일에서 삭제합니다. root 또는 없는 ID는 거부하며, 오래된 hash는 종료 코드 75로 실패합니다. 남은 Frame의 속성과 순서는 유지합니다. 마지막 자식을 삭제하면 root만 담는 v1으로 저장합니다.

파일 하나를 직접 고칠 때도 `component.root`와 `root_node.id`는 같은 전체 ID여야 합니다. 이름을 수동으로 변경하면 같은 변경에서 파일명도 새 이름·기존 접미사로 맞춰야 합니다. `check`는 불일치 상태를 거부합니다. CLI는 파일명 변경이나 여러 파일의 원자 저장을 제공하지 않습니다.

기존 바이너리 프로젝트를 변환하는 경로는 없습니다. 등록된 Property·Slot·token·state machine, Frame 이외의 자식 종류, 외부 layout 참조, Path clip, 알 수 없는 필드나 버전을 만나면 축소해서 저장하지 않습니다. opt-in 원본을 기본 앱의 컴포넌트 라이브러리로 불러오기와 다중 파일 복구는 후속 범위입니다. [ADR-0059](../adr/ADR-0059-text-source-files-with-chunk-cache.md)

## macOS 프로젝트 창에서 생성

열린 프로젝트의 Git 패널에서 **Trust Project…**로 폴더를 신뢰한 뒤 **Insert → New Component…**에서 이름을 입력합니다. **Create**는 `design/components/`에 독립된 v1 정의와 기본 root Frame을 파일 하나로 저장합니다. 같은 이름을 다시 입력하면 별도의 UUIDv7 두 개(component와 root)를 생성하므로 두 정의가 공존합니다. 완성된 파일의 경로를 창에 표시하며, Git staging과 commit은 사용자가 진행합니다.

빈 컴포넌트 생성은 생성 직전의 문서 generation·선택·Undo 기록을 보존합니다. 생성 결과는 프로젝트를 다시 열거나 다른 브랜치를 checkout한 뒤에도 현재 폴더에서 CLI와 MCP로 조회할 수 있습니다. 캔버스 Undo는 이 파일 생성을 취소하지 않습니다. 신뢰를 해제한 프로젝트, 기존 component-library chunk가 있는 프로젝트, 잘못된 이름·원본 또는 외부나 Git 내부를 가리키는 쓰기 폴더에서는 새 파일을 기록하지 않습니다. 같은 사용자로 동작하는 프로젝트 writer와 source CLI의 잠금을 사용합니다.

**Object → Create Component from Selection…**은 선택한 Frame 한 개와 그 아래의 모든 Free Frame을 현재 창의 미저장 편집까지 포함해 복사합니다. 정의와 모든 노드에 새 UUIDv7을 부여하고 내부 부모를 새 ID로 잇습니다. 선택 Frame은 부모가 없는 정의 root가 되고 모든 Frame의 기존 로컬 affine(이동 성분 포함), 크기, 색·테두리·모서리·clipping, 표시·잠금, 이름과 순서 키를 그대로 저장합니다. 이전 부모의 world transform은 곱하지 않습니다. 자식이 있으면 v2, 없으면 v1입니다.

선택 트리 안에 다른 종류의 노드, modifier stack, layout 참조 또는 Path clip이 있으면 전체 생성을 거부합니다. 이름 sheet가 열려 있던 선택·View·문서 revision이 다르거나 Git checkout으로 창이 읽은 generation의 hash가 달라져도 기록하지 않습니다. 캔버스의 원래 Frame·자식과 Undo 기록은 유지합니다. 이 명령은 독립 정의를 생성하며 캔버스에 인스턴스를 삽입하지 않습니다.

**View → Component Library**는 열린 프로젝트의 컴포넌트 이름·파일명·Frame 수·root 크기를 보여줍니다. 행을 선택하면 프로젝트 내 경로와 전체 component ID가 표시됩니다. 다른 창에서 작업하다 목록으로 돌아오거나 **Refresh**를 누르면 디스크의 현재 원본을 다시 읽습니다. Git checkout으로 같은 generation에서 원본 파일만 바뀐 경우도 반영합니다. 잘못된 파일이나 이중 원본에서는 이전 목록을 비우고 오류를 표시하고, 신뢰를 철회하거나 프로젝트를 닫으면 목록 창도 닫힙니다.

라이브러리에서 행을 선택하고 **Width**, **Height**에 논리 픽셀 크기를 입력한 뒤 **Apply Size** 또는 Return을 누르면 해당 컴포넌트의 root Frame을 즉시 저장합니다. 유한한 0 이상의 크기를 받습니다. 코어는 행에 표시된 전체 component ID와 원본 hash를 확인합니다. Git checkout·수동 편집·CLI 저장으로 같은 파일의 내용이 바뀌었거나 root가 잠겼으면 값을 적용하지 않고 현재 목록을 다시 읽습니다. 무변경은 주석을 포함한 bytes를 보존합니다. 다른 원본의 변경은 선택한 정의가 유효한 한 충돌로 간주하지 않습니다. 이 편집은 문서 generation이나 캔버스 Undo 기록을 만들지 않습니다.

## 라이브러리에서 인스턴스 배치

신뢰한 프로젝트에서 **View → Component Library**로 정의를 선택하고 **Place Instance…**를 누릅니다. 활성 View의 root Frame을 기준으로 X·Y를 논리 픽셀로 입력합니다. 음수와 소수 좌표를 사용할 수 있습니다. **Place**는 새 StableId를 가진 ComponentInstance 한 개를 추가하고 선택합니다. 기존 정의를 여러 번 배치하면 각 인스턴스는 같은 component ID를 참조합니다. 표시되었던 원본의 hash, 활성 View, 문서 revision 또는 기반 generation이 바뀌면 배치하지 않습니다.

각 배치는 Undo 한 단계이며 **Save Now**가 기존 문서 저장 절차로 기록합니다. 정의 YAML은 쓰지 않습니다. 인스턴스 payload에는 정의 참조와 override가 있고, 위치는 해당 노드 소유의 Transform modifier에 있습니다. 원본을 찾을 수 없으면 `stellar validate`가 인스턴스와 정의 ID를 포함하는 `missing-component-definition` 진단을 냅니다. 정의 이름을 다른 파일에 다시 사용하는 것은 이 참조를 바꾸지 않습니다.

## 프로젝트에서 조회

프로젝트 루트 아래 `design/components/`에 원본을 두면 `stellar query . components`와 신뢰된 headless MCP의 `stellar://components`가 같은 라이브러리를 읽습니다. 다른 폴더의 원본은 기존 `component-source check <폴더>`로 조회할 수 있습니다. 각 항목은 전체 StableId, 이름, 루트 ID, 루트의 논리 픽셀 크기, 루트를 포함한 Frame 수, 원본 bytes의 `source_hash`를 담습니다. `source_hash`는 그대로 `resize`와 `add-frame`에 전달할 수 있습니다.

`revision`은 프로젝트의 바이너리 generation을 가리킵니다. Git checkout이나 미커밋 YAML 편집은 generation을 올리지 않으므로 같은 MCP 연결에서도 매 요청마다 파일을 다시 읽습니다. 독립된 파일별 버전은 각 항목의 `source_hash`를 사용합니다. 여러 파일을 동시에 편집할 때 단일 시점의 스냅샷을 보장하려면 다중 파일 journal이 필요합니다.

프로젝트 단위 조회는 프로젝트 내부에서 해석되는 원본만 읽습니다. 정의 폴더가 없거나 해당 확장자의 파일이 없으면 `entries`는 빈 배열이고 `chunk_count`는 generation의 `component-library` 포인터 수입니다. 바이너리 라이브러리와 YAML 원본이 함께 있으면 오류를 반환합니다. 한 요청에 읽는 전체 원본은 64 MiB까지이고, MCP 리소스는 기존 512 KiB 본문 상한을 적용합니다. 파일·캐시·generation을 작성하지 않습니다.

## 두 작업 트리 비교

`stellar semantic-diff <base> <target>`은 같은 프로젝트의 두 Git 작업 트리에서 저장 문서와 `design/components/`의 공유 정의를 함께 비교합니다. 각 변경의 `object`는 전체 StableId이고, 공유 컴포넌트에 속한 변경은 `component`에 정의의 전체 ID, `document`에 `null`을 반환합니다. 이름과 파일명을 함께 바꾸면 같은 컴포넌트의 이름 변경 한 건입니다. Frame 크기는 속성 변경, Frame 생성·삭제는 해당 노드의 추가·삭제로 표시합니다. 주석·YAML 서식만 다르면 의미 변경은 없습니다.

`diff-summary`와 두 작업 트리를 공통 base와 비교하는 `semantic-merge`도 이 정보를 사용합니다. 같은 Frame 크기를 양쪽에서 다르게 바꾸면 속성 충돌이 반환됩니다. 이 명령들은 비교할 때 작업 파일이나 Git index를 수정하지 않습니다. headless MCP의 `document.semantic_diff`도 고정 HEAD의 Git blob과 현재 프로젝트의 YAML을 같은 규칙으로 비교합니다. 커서는 기준 commit, 저장 generation, 현재 컴포넌트 원본 집합의 hash와 결합됩니다. HEAD 안의 컴포넌트 폴더나 파일이 symlink이면 비교를 거부합니다. 신뢰한 macOS 프로젝트의 Git 패널에서 **Components → 파일명 → Component source**의 체크박스로 추가되거나 수정된 원본 하나를 Stage/Unstage할 수 있습니다. Stage는 YAML 서식과 주석까지 현재 파일 그대로 Git index에 넣습니다. Unstage는 HEAD의 파일로 index를 되돌리며 새 파일이면 index에서 제거합니다. 작업 파일과 형제 파일의 staging 상태는 보존합니다. 표시한 뒤 HEAD나 선택한 index 항목이 달라지면 새로고침해야 합니다. 삭제 및 이름 변경은 이 체크박스에서 지원하지 않습니다.

### MCP 비교 성능 재현

아래 명령은 한 프로세스에서 100·1,000·4,096개의 독립 root Frame YAML 파일을 쓰고 한 번에 Git에 커밋합니다. 각 크기마다 새 stdio 프로세스 첫 요청, 동일 요청, 파일 하나의 root 크기 편집 후 요청, 변경 상태의 반복 요청을 3회씩 측정합니다. 첫 요청 시간에는 서버 프로세스 생성과 initialize가 포함되지 않습니다. 매 요청에서 반환된 변경 개수도 검증합니다.

```sh
cargo test --locked --offline -p stellar-mcp --test headless_read component_diff_rtt -- --ignored --nocapture
```

`STELLAR_MCP_BENCH_BASELINE`에 이전 stellar-mcp 실행 파일 경로를 지정하면 같은 fixture에서 실행 순서를 교대로 바꾸어 비교합니다. 2026-11-14 Apple M3 Max의 로컬 Debug 빌드에서 측정한 중앙값은 다음과 같습니다. 기준은 커밋 `c15e5fe2`이며 단위는 ms입니다.

| 컴포넌트 수 | 첫 조회 기준 → 변경 | 동일 조회 기준 → 변경 | 파일 하나 편집 기준 → 변경 |
|---:|---:|---:|---:|
| 100 | 349 → 270 | 135 → 68 | 344 → 74 |
| 1,000 | 851 → 581 | 248 → 120 | 860 → 178 |
| 4,096 | 2,721 → 1,750 | 695 → 320 | 2,782 → 558 |

세션은 검증한 고정 HEAD의 의미 객체를 재사용합니다. 파일 이름 집합과 저장 generation이 같고 한 파일의 bytes만 바뀌면 그 정의만 다시 투영하고 나머지 객체와의 ID 충돌을 검사합니다. 이름 변경, 파일 추가·삭제, 여러 파일 편집은 전체 검증을 수행합니다. 현재 파일 내용과 HEAD를 매번 재확인하므로 반복 요청에도 bytes를 읽는 비용이 포함됩니다. 측정은 파일시스템 캐시를 비우지 않은 로컬 개발 환경에서 수행했습니다.

## 검증

`cargo test -p stellar-cli --test component_source`는 기존 generation을 가진 프로젝트에서 위 두 파일을 생성·커밋하고, 각각의 브랜치에서 한 파일만 수정합니다. 일반 `git merge --no-ff` 후 새 clone에서 `check`가 양쪽 root 크기(240.5 × 64, 32 × 55), 전체 ID와 파일 hash를 재현하며 작업 트리는 깨끗합니다. 한 파일의 편집 전후에 다른 파일 bytes를 직접 비교하고, clone 뒤 기존 프로젝트 설정·generation의 hash가 모두 같은지 확인합니다. 같은 테스트가 component/root 교차 중복 ID, 오래된 hash, 미지원 중첩 Property·Slot·state·token·알 수 없는 Frame 필드와 잘못된 root 참조의 원본 보존, 이름 변경 후 참조를 확인합니다.

`cargo test -p stellar-project frame_and_component_library_bytes_survive_a_source_round_trip`는 회전·이동, Display P3 배경·테두리, 서로 다른 모서리 반경, 표시·잠금 값을 설정하고 YAML 왕복 전후 실제 라이브러리의 canonical binary bytes를 비교합니다.

`nested_frames_survive_a_component_rename_and_fresh_clone`은 형제 두 개와 손자 하나를 CLI로 생성합니다. 루트 resize 시 크기 한 줄만 달라지는지, 두 번째 컴포넌트 파일의 bytes가 같은지 검사합니다. 이름 변경을 실제 Git rename commit으로 만들고 새 clone의 라이브러리 결과를 비교합니다. `nested_frame_errors_preserve_every_component_file`은 중복 ID·다른 파일의 부모·미지원 종류·오래된 hash 요청 전후 모든 원본 hash를 검사합니다. 도메인 검사에서는 재조립한 정의를 참조하는 두 개의 scene 인스턴스에서 동일한 자식 순서와 손자의 Frame payload를 각각 확인합니다.

`set-visible <파일> <source_hash> <node-id> <true|false>`는 root 또는 자식 Frame의 자체 표시 여부를 저장합니다. 하위 Frame의 자체 표시 값은 유지합니다. 무변경은 원본 bytes를 보존하며 없는 ID·잘못된 bool은 거부하고 오래된 hash는 종료 코드 75로 실패합니다.

`set-locked <파일> <source_hash> <node-id> <true|false>`는 root 또는 자식 Frame의 자체 잠금 값을 저장합니다. 하위 Frame의 자체 값은 유지하고 잠금 해제도 같은 명령으로 수행합니다. 무변경은 주석과 서식을 보존하며 없는 ID·잘못된 bool은 거부하고 오래된 hash는 종료 코드 75로 실패합니다. `set-position`, `resize`, `resize-frame`은 대상 또는 조상 Frame이 잠겨 있으면 무변경 요청도 종료 코드 65로 거부하고 원본 bytes를 보존합니다. 오래된 hash 검사는 잠금 검사보다 우선합니다. geometry 편집을 재개하려면 대상과 조상의 잠금을 모두 해제해야 하며, 조상을 해제해도 자식의 자체 잠금 값은 유지합니다. 이름·표시·구조 편집의 잠금 강제와 macOS UI 연결은 별도 범위입니다.

`set-position <파일> <source_hash> <node-id> <x> <y>`는 root 또는 자식 Frame의 부모 좌표계 위치를 유한한 f32로 저장합니다. 음수 좌표를 허용하며 affine 선형부(회전·배율 등)와 하위 Frame의 로컬 좌표는 유지합니다. 무변경은 원본 bytes를 보존하며 없는 ID·비유한 좌표는 거부하고 오래된 hash는 종료 코드 75로 실패합니다.
