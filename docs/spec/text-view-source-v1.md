# 단일 View 텍스트 원본 v1·v2·v3

[ADR-0059](../adr/ADR-0059-text-source-files-with-chunk-cache.md)의 첫 opt-in 구현 형식입니다. 문서 하나, 페이지 하나, View 하나와 그 아래 Frame·Fragment·Rectangle·Ellipse·Line·Polygon·Text·컴포넌트 인스턴스 참조를 사람이 읽는 YAML 파일 하나에 보관합니다. 기존 프로젝트의 기본 열기·저장 경로는 세대 기반 chunk 저장을 계속 사용합니다.

구현 원본은 [`text_codec.rs`](../../crates/project/src/document_io/text_codec.rs), 파일 입출력은 [`text_source.rs`](../../crates/project/src/document_io/text_source.rs), 기존 프로젝트 export는 [`ops/text_source.rs`](../../crates/ops/src/text_source.rs)입니다.

## 파일 이름과 CLI

파일 이름은 `home-00000003.stellar.yaml`처럼 사람이 읽는 View 이름의 slug와 View `StableId`의 짧은 고정 부분을 조합하는 것을 권장합니다. 확장자·slug 생성·충돌 처리를 자동화하지 않으며, CLI에 출력 경로를 직접 지정합니다. 식별 기준은 파일 안의 전체 `StableId`입니다. 이름을 바꿔도 ID는 유지합니다.

```sh
stellar text-source export ./MyProject ./home-00000003.stellar.yaml
stellar text-source check ./home-00000003.stellar.yaml
```

`export <project> <new-file>`은 지원 범위의 기존 저장 프로젝트를 새 파일로 내보냅니다. 출력 파일이 이미 있으면 실패하며 덮어쓰지 않습니다. 기존 프로젝트의 원본과 generation은 변경하지 않습니다. `check <file>`은 텍스트를 읽어 문법·지원 범위·문서 불변식을 검증합니다. `check`와 라이브러리 `open`은 원본·캐시를 쓰지 않는 읽기 전용 작업입니다.

## 전체 예시

아래는 root Frame과 자식 Fragment가 있는 완전한 입력입니다. 모든 표시 이름과 공통 노드 필드, Frame의 일곱 속성을 포함합니다. 예시 ID는 서로 다른 UUIDv7입니다.

```yaml
schema_version: "1"
document:
  id: "019a8180-0001-7000-8000-000000000001"
  display_name: "예제 문서"
page:
  id: "019a8180-0002-7000-8000-000000000002"
  display_name: "첫 페이지"
view:
  id: "019a8180-0003-7000-8000-000000000003"
  display_name: Home
  root_frame: "019a8180-0010-7000-8000-000000000010"
nodes:
  -
    id: "019a8180-0010-7000-8000-000000000010"
    parent: none
    order_key: "80"
    visible: "true"
    locked: "false"
    display_name: "홈 화면"
    kind: frame
    payload:
      size: "1440 900"
      transform: "1 0 0 1 0 0"
      clipping:
        kind: frame-bounds
      background:
        space: srgb
        rgba: "0.95 0.96 1 1"
      border:
        color:
          space: display-p3
          rgba: "0.2 0.3 0.5 1"
        thickness: "1.5"
      corner_radius: "12 12 12 12"
      layout_mode: none
  -
    id: "019a8180-0011-7000-8000-000000000011"
    parent: "019a8180-0010-7000-8000-000000000010"
    order_key: "0080"
    visible: "true"
    locked: "false"
    display_name: "콘텐츠 그룹"
    kind: fragment
    payload: none
```

## 새 편집 문서의 기본 타이포그래피

문서 스타일 표가 없는 기존 원본은 계속 v1으로 저장합니다. 문서의 스타일 표 전체가 아래 v2 규격과 일치할 때 타이포그래피 리소스 ID 하나로 내보냅니다.

```yaml
schema_version: "2"
document:
  id: "019a8180-0001-7000-8000-000000000001"
  display_name: Document
  initial_typography_id: "019a8180-0006-7000-8000-000000000006"
```

v2 리소스의 family 목록은 비고 크기는 CSS `medium`입니다. 두께는 400, 스타일은 normal, stretch는 100%, line-height와 letter-spacing은 normal입니다. 문단 0번은 Start 정렬, Wrap 줄바꿈, Auto 줄 높이로 문서의 타이포그래피를 상속합니다. 문자 0번은 명시적 타이포그래피·채우기·자간·OpenType feature·variation이 없습니다. 다른 문단·문자 항목이나 문서 채우기·프로젝트 글꼴도 없습니다. 이 값들은 v2에 고정되며 앱의 새 문서 기본값과 독립적으로 복원됩니다. 노드의 문단·run 인덱스와 범위는 v1의 표현을 그대로 사용합니다. 리소스 ID의 NIL 값과 문서 내 ID 충돌은 검증 오류입니다. File → Document Font…에서 등록한 프로젝트 글꼴이 있으면 다음 v3 형식을 사용합니다.

## 등록한 문서 글꼴과 Text 위치 (v3)

단일 문서가 소유하는 기본 글꼴을 지정한 경우 `document`에 `id`, `display_name`, `document_font`를 기록합니다. 다음 예시는 기본 색과 텍스트 스타일 리소스, 프로젝트에 포함된 파일의 face 0을 연결합니다. 본문의 글자가 쓰는 문단·문자 0번은 v2와 같은 상속 항목입니다.

```yaml
document_font:
  id: "019a8180-0006-7000-8000-000000000006"
  font-family:
    name: Example
    file_hash: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    face_index: "0"
  font-size: "24px"
  font-weight: "400"
  font-style: normal
  font-stretch: "100%"
  line-height: normal
  letter-spacing: normal
  color:
    space: srgb
    rgba: "0 0 0 1"
  project_fonts:
    -
      content_hash: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
      asset_id: "019a8180-0007-7000-8000-000000000007"
      media_type: "font/ttf"
      byte_size: "8192"
      source:
        kind: project
        path: "fonts/Example.ttf"
      faces:
        -
          family: Example
          subfamily: Regular
```

- `font-size`는 양의 1/64 px 단위를 정확히 표현하는 십진수와 `px`입니다. `font-weight`의 범위는 1~1000, `font-stretch`는 비음수 백분율입니다. 현재 일반 글꼴 경로의 `font-style`, `line-height`, `letter-spacing`은 각각 `normal`을 명시합니다.
- `font-family`는 등록된 프로젝트 글꼴의 SHA-256, 실제 face 인덱스, 해당 face의 family 이름에 일치해야 합니다. `project_fonts`는 hash 순으로 기록하며 이전에 선택해 등록된 글꼴도 유지합니다. 각 face의 `family`, `subfamily`, `postscript` 중 원래 없는 이름은 생략합니다. 등록 레코드의 media type에서 컨테이너 형식을 복원합니다. `source`는 `none`, 프로젝트 안의 상대 경로, 또는 `kind: external`과 `filename`, `origin`(원래의 HMAC 출처 지문)으로 기록합니다. 외부 원본의 절대 경로를 저장하지 않습니다.
- v3 Text 노드는 `content`, `paragraphs`, `style_runs`, `placement`를 모두 가집니다. `placement: none` 또는 기존 단일 활성 Transform의 `id`, `order_key`, 부모 좌표계의 `position`을 기록합니다. 문단/문자 스타일 0을 참조하는 run의 범위와 순서를 보존합니다.
- 다른 스타일 항목, 여러 family와 fallback, variation, 사용자 지정 라이선스 정보, 미지원 modifier 또는 해석할 수 없는 등록 레코드는 저장 전에 거부합니다. 독립 YAML의 `check`는 문서 안의 참조와 등록 메타데이터를 검증합니다. 프로젝트를 내보낼 때 등록된 모든 글꼴 파일의 hash와 byte 수를 asset store에서 확인합니다. 파일을 복사한 프로젝트에서도 같은 등록 face를 열어 사용합니다.

## 스키마와 값 규칙

모든 mapping은 아래 키만 허용하며, 나열한 키는 모두 필수입니다. 값이 기본값이거나 비어 있어도 키를 생략하지 않습니다. 알 수 없는 키, 중복 키, 빠진 키, 잘못된 타입은 오류입니다. `schema_version`은 `1`, `2`, `3`입니다.

| 위치 | 필수 키와 의미 |
|---|---|
| 최상위 | `schema_version`, `document`, `page`, `view`, `nodes` |
| `document` | `id`, `display_name`. v2에는 `initial_typography_id`, v3에는 `document_font`도 필수 |
| `page` | `id`, `display_name` |
| `view` | `id`, `display_name`, `root_frame` |
| `nodes`의 각 항목 | `id`, `parent`, `order_key`, `visible`, `locked`, `display_name`, `kind`, `payload` |
| Frame `payload` | `size`, `transform`, `clipping`, `background`, `border`, `corner_radius`, `layout_mode` |
| 도형 `payload` | 종류별 기하 필드와 `modifiers` 목록. [도형](#도형) 참조 |
| Text `payload` | `content`, `paragraphs`, `style_runs`. v3에는 `placement`도 필수 |
| ComponentInstance `payload` | `component`, `placement` |
| 인스턴스 `placement` | `none` 또는 `id`, `order_key`, `position` mapping |
| Text 문단 | `style` (기존 문단 style 표의 `u32` index) |
| Text style run | `start`, `end`, `style` (`u32`) |
| 색상 | `space`, `rgba` |
| 테두리 | `color`, `thickness` |

`document`·`page`·`view`는 각각 하나의 mapping이며 배열이 아닙니다. `nodes`와 Text의 `paragraphs`·`style_runs`는 sequence입니다. 문서 validator가 ID 중복, root Frame, 부모 연결, 트리 깊이와 참조 불변식을 검증합니다. 부모가 없는 노드는 View의 root Frame이어야 합니다. 표시 이름은 빈 문자열을 포함해 문자열 그대로 보존하며 식별에 사용하지 않습니다.

| 필드 | 허용 값 |
|---|---|
| `id`, `root_frame` | 전체 `StableId`를 나타내는 하이픈 UUID 문자열 |
| `parent` | 부모 노드의 전체 `StableId`, root는 `none` |
| `order_key` | byte당 두 자리 hex 문자열. 빈 값·홀수 길이·마지막 byte `00`은 거부 |
| `visible`, `locked` | 소문자 `true` 또는 `false` |
| `kind` | `frame`, `fragment`, `rectangle`, `ellipse`, `line`, `polygon`, `text`, `component-instance` |
| Fragment `payload` | 정확히 `none`; 별도 시각·layout 필드 없음 |
| Frame `size` | `width height` 순서의 비음수 유한 `f32` 두 개 |
| Frame `transform` | `a b c d tx ty` 순서의 유한 `f32` 여섯 개 |
| Frame `corner_radius` | 좌상·우상·우하·좌하 순서의 비음수 유한 `f32` 네 개 |
| Frame `background` | 색상 mapping 또는 `none` |
| Frame `border` | 테두리 mapping 또는 `none`; `thickness`는 비음수 유한 `f32` |
| Frame `layout_mode` | 참조 `StableId` 또는 `none` |
| 색상 `space` | `srgb` 또는 `display-p3` |
| 색상 `rgba` | 비연관 알파 `r g b a` 순서의 유한 `f32` 네 개. 알파는 `[0, 1]`, RGB는 확장 범위 허용 |

숫자 벡터는 YAML sequence가 아닌 공백으로 구분한 scalar입니다. 지정된 숫자 개수가 다르거나 NaN·무한대이면 거부합니다. `none`은 없는 값을 나타내는 명시적 scalar이며 YAML null로 대체하지 않습니다. `display_name: none`처럼 문자열 필드에 쓴 `none`은 표시 이름 그대로입니다.

`clipping`은 다음 세 mapping 중 하나입니다. `disabled`·`frame-bounds`에 `target`을 붙이면 오류이며, `path`에서는 `target`이 필수입니다.

```yaml
kind: disabled
```

```yaml
kind: frame-bounds
```

```yaml
kind: path
target: "019a8180-0099-7000-8000-000000000099"
```

Path clipping과 `layout_mode`는 참조 ID를 보존합니다. 미해결 Path 참조는 기존 문서 모델 규칙에 따라 보존할 수 있지만, Path 노드 payload나 layout 정의 자체를 이 파일에 추가하는 기능은 포함하지 않습니다. 이미 존재하는 잘못된 종류의 노드를 Path로 참조하면 문서 validator가 거부합니다.

## Text 본문과 스타일 범위

Text의 `payload`는 본문과 기존 모델의 문단·style run을 모두 명시합니다. 다음 예시는 `확인` 뒤 LF가 있는 두 문단이며 범위는 UTF-8 byte가 아닌 Unicode scalar offset입니다.

```yaml
content: "확인\n다음"
paragraphs:
  -
    style: "7"
  -
    style: "2"
style_runs:
  -
    start: "0"
    end: "2"
    style: "3"
```

`paragraphs` 길이는 본문의 LF 개수 + 1이어야 합니다. 빈 본문도 문단 하나를 가지며, 빈 `style_runs`는 `[]`입니다. 각 run은 `0 <= start <= end <= 본문 scalar 길이`를 만족해야 합니다. 정수는 `u32` 범위이며 음수·범위 초과는 거부합니다. v1·v2는 기존 모델의 숫자 index와 범위를 보존합니다. v3는 같은 범위 표현을 사용하며 0번 스타일을 문서 글꼴로 해석합니다.

본문의 Unicode 정규화, CR/CRLF 변환, run 정렬·병합·겹침 제거를 하지 않습니다. 빈 run·겹치는 run·run 순서도 기존 payload 그대로 보존합니다. 제어 문자는 YAML 인용·이스케이프를 사용하고 읽을 수 없는 긴 scalar는 저장 전에 거부합니다.

스타일 표가 없는 문서의 Text는 기존 v1을 사용합니다. 등록된 문서 글꼴과 배치 ID가 있는 v3는 구버전 reader에서 거부됩니다.

## 컴포넌트 인스턴스의 배치

앱의 **Place Instance…**가 만든 노드는 독립 정의의 전체 StableId를 참조합니다. 인스턴스마다 공통 노드 필드에 자신의 ID·부모·순서 키가 있고, `payload`에 다음 값을 기록합니다.

```yaml
kind: component-instance
payload:
  component: "019a8180-0030-7000-8000-000000000030"
  placement:
    id: "019a8180-0031-7000-8000-000000000031"
    order_key: "80"
    position: "230.5 -10"
```

`placement.id`와 `order_key`는 인스턴스 소유의 단일 활성 Transform modifier에서 가져옵니다. `position`은 부모 좌표계의 X·Y이며 선형부는 단위 행렬입니다. 명시적으로 (0, 0)에 배치한 노드도 이 modifier를 유지합니다. modifier가 없는 인스턴스는 `placement: none`으로 저장합니다. Property·Slot override, 복수 modifier, 비활성 modifier, 회전·스케일 등이 있으면 저장을 거부합니다.

v1·v2·v3 모두 이 노드 종류를 사용할 수 있습니다. 이전 reader는 새 종류를 만나면 파일을 거부합니다. 단일 파일 `check`는 인스턴스 노드와 참조 ID를 복원합니다. 참조하는 정의가 실제 프로젝트에 있는지는 `stellar validate <프로젝트>`로 확인합니다. 컴포넌트 정의 파일은 기존 `design/components/*.stellar.yaml`에 따로 둡니다.

## 도형

`kind: rectangle`의 `payload`에는 `width`, `height`, `border-top-left-radius`, `border-top-right-radius`, `border-bottom-right-radius`, `border-bottom-left-radius`를 각각 논리 픽셀로 기록합니다. 화면에 맞춰 줄이기 전의 네 모서리 값을 보존합니다.

`kind: ellipse`는 로컬 경계의 왼쪽 위 좌표 `origin`(공백으로 구분한 x, y), `width`, `height`를 갖습니다. `kind: polygon`은 같은 경계와 3~1,000 사이의 변 수 `sides`, 도 단위 회전 `rotation_degrees`, 보정 전 논리 픽셀 반경 `corner_radius`를 갖습니다. `kind: line`의 `start`와 `end`는 그린 방향의 로컬 좌표이며 `cap`은 `butt`, `round`, `square` 중 하나입니다.

네 도형의 `modifiers` 목록은 원본의 `order_key` 순서로 기록합니다. 각 항목은 `id`, `order_key`, `enabled`, `kind`, `value`를 가지며, 항목이 없는 도형은 빈 목록을 씁니다.

| `kind` | `value` |
| --- | --- |
| `transform` | 여섯 affine 성분을 담은 문자열 |
| `fill` | Frame 색과 같은 `space`와 `rgba` |
| `stroke` | `color`(`space`, `rgba`)와 논리 픽셀 단위의 비음수 `width` |

v1·v2·v3는 같은 도형 문법을 사용합니다. 이전 reader는 해석할 수 없는 노드나 modifier 종류를 거부합니다.

## 텍스트 결정론과 파서 제한

기존 제한 YAML loader와 checked canonical writer를 사용합니다. loader는 모든 scalar를 문자열로 보존하고, codec이 ID·불리언·숫자를 명시적으로 해석합니다. 따라서 예시의 숫자와 불리언이 따옴표로 감싸져 있어도 같은 의미로 읽힙니다. writer는 재파싱해 같은 값 트리가 되는 출력만 반환합니다.

- 키 순서는 위 예시처럼 고정합니다. 노드는 `StableId` 오름차순으로 기록하며, 형제의 표시 순서는 `order_key`가 보존합니다.
- float은 `f32`가 정확히 왕복되는 최단 `Display` 표현을 씁니다. 색상·크기·변환 값을 8비트 색상이나 반올림한 값으로 바꾸지 않습니다.
- UTF-8, 공백 두 칸 들여쓰기, LF와 마지막 newline을 사용합니다. 문자열은 기존 writer의 인용·이스케이프 규칙을 따릅니다.
- 입력·출력 상한은 **1 MiB(1,048,576 bytes)**, YAML 컨테이너 중첩 상한은 **64**입니다. 이벤트 하나의 scanner 선읽기는 **32 Ki 문자**로 제한됩니다.
- anchor, alias, custom tag, merge key, 비스칼라 mapping 키, 중복 키, 다중 YAML document를 거부합니다. 주석은 모델의 일부가 아니므로 저장 시 보존하지 않습니다.

텍스트에는 runtime handle이나 binary payload의 hex·base64 blob을 넣지 않습니다. `order_key`의 hex는 순서 키 byte sequence의 표기이며, 노드나 chunk 전체를 숨긴 직렬화 blob이 아닙니다.

## 지원 범위 밖의 데이터

Path·Image payload, component library, token set, 도형의 Transform·Fill·Stroke 및 인스턴스나 v3 Text의 배치를 제외한 modifier stack, 다중 문서·페이지·View는 저장할 수 없습니다. 미지원 데이터를 만나면 원본을 보존하고 오류를 반환합니다.

기존 프로젝트 export는 로드한 문서를 다시 직렬화한 **전체 chunk pointer 집합**이 저장된 generation과 정확히 같아야 합니다. ChunkId·내용 hash·kind·schema version을 비교하고 generation schema와 단일 root 문서도 검사합니다. 따라서 문서 밖 메타데이터, component·token chunk, 사용되지 않는 추가 chunk, 알 수 없는 optional chunk도 조용히 버리지 않습니다. 이미 알려진 kind라는 이유만으로 허용하지 않습니다. 지원되는 단일 View의 예상 집합은 DocumentManifest 하나, ViewTree 하나, subtree 하나입니다.

이 검사는 지원되는 원본의 명시적 export만 허용하는 경계입니다. 프로젝트 전체를 자동 마이그레이션하거나 기존 원본 파일을 삭제·대체하지 않습니다.

## 열기·저장과 파생 캐시

라이브러리 `save_new`는 완성된 임시 파일을 hard link로 게시하므로 마지막 검사 후 다른 파일이 생겨도 기존 항목을 덮어쓰지 않습니다. hard link를 지원하지 않는 파일 시스템은 오류로 거부합니다. `save`는 열었을 때의 원본 bytes hash와 문서 ID를 확인한 뒤 단일 파일을 원자적으로 교체합니다. 원본이 외부에서 바뀌었거나 재해석할 수 없는 필드가 생겼으면 덮어쓰지 않습니다. CLI에는 현재 `export`와 읽기 전용 `check`만 노출합니다. 저장은 기존 파일 권한을 유지합니다. 마지막 hash 검사와 rename 사이에 비협조적 외부 writer가 개입하는 경쟁은 기존 atomic 저장과 같이 남아 있으므로 앱 연결 전 프로젝트 저장 잠금과 외부 변경 통합이 필요합니다. 부모 디렉터리 fsync 또는 새 파일 게시 뒤 임시 링크 제거 실패는 대상이 이미 생긴 뒤 반환될 수 있으므로 오류 후 실제 원본을 다시 읽어 상태를 확인해야 합니다.

캐시는 라이브러리 `open_with_cache(path, project_root)`를 명시적으로 호출할 때 생성합니다.

```text
<project_root>/.stellar/cache/text-source-v1/<sourcehash>/<ChunkId>.starchunk
```

`sourcehash`는 실제 UTF-8 원본 bytes의 SHA-256 digest를 소문자 hex 64자로 쓴 값입니다. 주석·공백 변경도 다른 캐시 키가 됩니다. 텍스트에서 기존 serializer의 DocumentManifest·ViewTree·subtree 세 chunk를 재생성하고 기존 envelope 형식으로 기록합니다. 같은 bytes이면 유지하고, 없거나 손상됐으면 다시 만듭니다. 캐시 전용 디렉터리의 `.gitignore`(`*`)를 먼저 생성·검증하므로 기존 프로젝트에도 새 캐시가 Git 변경으로 나타나지 않습니다. 기존 marker 내용이 다르면 덮어쓰지 않고 cache 오류로 보고합니다. 캐시는 Git 밖의 파생 데이터이며 삭제해도 텍스트 원본에서 재구축할 수 있습니다. 캐시 I/O 실패는 상태로 보고하고, 유효한 원본의 열기 결과는 반환합니다.

현재는 캐시가 있어도 **열 때마다 텍스트 전체를 해석·검증하고 chunk를 재생성해 비교**합니다. 빠른 열기·부분 로딩·성능 예산 충족을 주장하지 않습니다. 캐시 GC도 이번 구현 범위 밖입니다.

## 후속 범위

에디터의 기본 열기·자동 저장 연결, codegen, 파일 감시와 외부 변경 자동 반영, 여러 의미 파일로 분할하는 저장 journal, 전체 프로젝트 마이그레이션, 추가 노드·component·token 직렬화는 아직 구현하지 않았습니다. 현재 보장하는 저장 원자성은 이 단일 텍스트 파일에 한정됩니다.
