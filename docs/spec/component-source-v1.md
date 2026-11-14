# 컴포넌트별 텍스트 원본 v1

`stellar component-source`는 빈 공개 인터페이스와 루트 Frame 하나로 이루어진 컴포넌트를 독립 YAML 파일에 저장합니다. 지정한 폴더가 원본이고 `check`는 여기서 실제 `ComponentLibrary`를 재조립합니다. 예를 들어 다음 명령은 이름이 같은 컴포넌트 두 개를 생성합니다.

```sh
mkdir -p design/components
stellar component-source create design/components 버튼 0195c530-3acd-7bc1-8414-c213f61cb965 0195c530-3acd-7bc1-8414-c213f61cb966
stellar component-source create design/components 버튼 0195c530-3acd-7bc1-8414-c213f61cb967 0195c530-3acd-7bc1-8414-c213f61cb968
stellar component-source check design/components
```

위 명령이 만드는 파일은 `버튼-8c5619572809.stellar.yaml`과 `버튼-bc6dfb3d3aea.stellar.yaml`입니다.

- 파일명은 `<컴포넌트 이름>-<접미사>.stellar.yaml`입니다. 접미사는 component StableId 16바이트의 SHA-256 digest 앞 12자리 소문자 hex입니다. 파일 내부의 `component.id`와 `component.root`는 전체 StableId입니다. 이름은 파일 이름으로 쓸 수 있어야 하고 전체 파일명은 UTF-8 255바이트 이하여야 합니다. 중복 ID와 대소문자 무시 파일명 충돌은 거부합니다.
- 최상위 키는 `format: "stellar-component"`, `schema_version: "1"`, `component`, `root_node`입니다. `component`는 `id`, `name`, `root`를 가집니다. `root_node`는 단일 View 원본과 같은 Frame 레코드입니다. 크기·affine·배경색·테두리·모서리·clipping·표시/잠금·순서 키를 보존합니다. 루트는 부모가 없는 Frame이어야 합니다. [Frame 레코드 문법](text-view-source-v1.md)
- `check`는 지정 폴더 바로 아래의 모든 `.stellar.yaml`을 제한 YAML로 검사하고, 이름과 접미사, 전체 component/root ID, tree 소유권이 일치하는 정의를 StableId 순으로 반환합니다. 파일당 1 MiB, 폴더당 4,096개까지 허용합니다. 캐시, 중앙 색인이나 generation은 필요하지 않습니다.
- `resize <파일> <source_hash> <너비> <높이>`는 root Frame의 크기를 논리 픽셀로 바꿉니다. `source_hash`는 `create` 또는 `check` 결과에서 얻습니다. 크기는 유한한 비음수 f32이며 결과 JSON에서는 최단 왕복 문자열입니다. 실제 변경은 해당 파일 하나를 결정론적으로 다시 씁니다. 무변경 요청은 주석과 서식을 포함한 기존 bytes를 보존합니다. 협조적 CLI writer는 원본 폴더 잠금을 공유하고, 외부 변경으로 hash가 다르면 종료 코드 75로 실패합니다.

```sh
source_info=$(stellar component-source check design/components | jq -c '.result.components[0]')
stellar component-source resize "$(printf '%s' "$source_info" | jq -r .file)" "$(printf '%s' "$source_info" | jq -r .source_hash)" 248.5 44
git diff -- design/components
```

파일 하나를 직접 고칠 때도 `component.root`와 `root_node.id`는 같은 전체 ID여야 합니다. 이름을 수동으로 변경하면 같은 변경에서 파일명도 새 이름·기존 접미사로 맞춰야 합니다. `check`는 불일치 상태를 거부합니다. CLI는 파일명 변경이나 여러 파일의 원자 저장을 제공하지 않습니다.

기존 바이너리 프로젝트를 변환하는 경로는 없습니다. 등록된 Property·Slot·token·state machine, 자식 node, 외부 layout 참조, Path clip, 알 수 없는 필드나 버전을 만나면 축소해서 저장하지 않습니다. opt-in 원본을 기본 앱의 컴포넌트 라이브러리로 불러오기와 다중 파일 복구는 후속 범위입니다. [ADR-0059](../adr/ADR-0059-text-source-files-with-chunk-cache.md)

## 검증

`cargo test -p stellar-cli --test component_source`는 기존 generation을 가진 프로젝트에서 위 두 파일을 생성·커밋하고, 각각의 브랜치에서 한 파일만 수정합니다. 일반 `git merge --no-ff` 후 새 clone에서 `check`가 양쪽 root 크기(240.5 × 64, 32 × 55), 전체 ID와 파일 hash를 재현하며 작업 트리는 깨끗합니다. 한 파일의 편집 전후에 다른 파일 bytes를 직접 비교하고, clone 뒤 기존 프로젝트 설정·generation의 hash가 모두 같은지 확인합니다. 같은 테스트가 component/root 교차 중복 ID, 오래된 hash, 미지원 중첩 Property·Slot·state·token·자식·Frame 필드와 잘못된 root 참조의 원본 보존, 이름 변경 후 참조를 확인합니다.

`cargo test -p stellar-project frame_and_component_library_bytes_survive_a_source_round_trip`는 회전·이동, Display P3 배경·테두리, 서로 다른 모서리 반경, 표시·잠금 값을 설정하고 YAML 왕복 전후 실제 라이브러리의 canonical binary bytes를 비교합니다.
