# 단일 View 텍스트 원본(쓰기 v2)

[ADR-0059](../adr/ADR-0059-text-source-files-with-chunk-cache.md)의 방향과 [ADR-0061](../adr/ADR-0061-text-source-format-and-layout.md)의 형식 결정을 따르는 opt-in 형식입니다. 문서 하나, 페이지 하나, View 하나와 그 아래 Frame·Fragment·Rectangle·Ellipse·Line·Polygon·Text·컴포넌트 인스턴스 참조를 사람이 읽는 YAML 파일 하나에 보관합니다. 기존 프로젝트의 기본 열기·저장 경로는 세대 기반 chunk 저장을 계속 사용합니다.

구현 원본은 [`text_codec.rs`](../../crates/project/src/document_io/text_codec.rs), 파일 입출력은 [`text_source.rs`](../../crates/project/src/document_io/text_source.rs), 기존 프로젝트 export는 [`ops/text_source.rs`](../../crates/ops/src/text_source.rs)입니다.

## 버전

- 쓰기 버전은 `schema_version: "2"` 하나입니다. 모든 저장은 v2로 씁니다.
- `"1"`은 이미 커밋된 원본의 읽기 전용 문법입니다. 문서 style 표·도형·컴포넌트 인스턴스·배치가 없고, Frame을 `size`·`corner_radius`와 색·clipping mapping으로 기록합니다. v1 파일을 열어 저장하면 v2로 씁니다.
- 번호를 재정의하기 전 작업 트리에서만 쓰인 v2(`document.initial_typography_id`)와 v3(`document_font`의 `project_fonts`) 문법은 읽지 않고 거부합니다.

## 파일 이름과 CLI

파일 이름은 `home-00000003.stellar.yaml`처럼 사람이 읽는 View 이름의 slug와 View `StableId`의 짧은 고정 부분을 조합하는 것을 권장합니다. 확장자·slug 생성·충돌 처리를 자동화하지 않으며, CLI에 출력 경로를 직접 지정합니다. 식별 기준은 파일 안의 전체 `StableId`입니다. 이름을 바꿔도 ID는 유지합니다.

```sh
stellar text-source export ./MyProject ../exports/home-00000003.stellar.yaml
stellar text-source check ../exports/home-00000003.stellar.yaml
```

`export <project> <new-file>`은 지원 범위의 기존 저장 프로젝트를 새 파일로 내보냅니다. 출력 파일이 이미 있으면 실패하며 덮어쓰지 않습니다. 기존 프로젝트의 원본과 generation은 변경하지 않습니다. 기본 저장 전환(V.10.7) 전에는 generation이 원본이므로, 실제 경로로 해석한 출력 위치가 프로젝트 루트 안이면 두 번째 원본이 되어 거부합니다(종료 코드 69). `check <file>`은 텍스트를 읽어 문법·지원 범위·문서 불변식을 검증합니다. `check`와 라이브러리 `open`은 원본·캐시를 쓰지 않는 읽기 전용 작업입니다.

## 전체 예시

아래는 root Frame과 자식 Fragment가 있는 완전한 입력입니다. 모든 표시 이름과 공통 노드 필드, Frame의 열한 속성을 포함합니다. 예시 ID는 서로 다른 UUIDv7입니다.

```yaml
schema_version: "2"
document:
  id: "019a8180-0001-7000-8000-000000000001"
  display_name: "예제 문서"
  document_font: none
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
      background: "srgb 0.95 0.96 1 1"
      border: "1.5 display-p3 0.2 0.3 0.5 1"
      border-bottom-left-radius: "12"
      border-bottom-right-radius: "12"
      border-top-left-radius: "12"
      border-top-right-radius: "12"
      clipping: frame-bounds
      height: "900"
      layout-mode: none
      transform: "1 0 0 1 0 0"
      width: "1440"
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

## 한 속성 한 줄

영속 속성 하나는 키 하나이고 값은 한 줄입니다([ADR-0061](../adr/ADR-0061-text-source-format-and-layout.md) 결정 2). Frame 키 이름은 PropertyKey의 CSS longhand 이름이고, 키는 이름 오름차순으로 씁니다. 그래서 공통 조상에서 서로 다른 속성을 바꾼 두 브랜치는 바뀐 두 줄 사이에 바뀌지 않은 줄이 있으면 일반 `git merge`로 합쳐집니다. 이웃한 줄의 변경은 Git 충돌로 남고 `stellar semantic-merge`가 속성 단위로 해결합니다.

| Frame 키 | 값 |
|---|---|
| `background` | `none` 또는 색 |
| `border` | `none` 또는 두께(비음수 f32) 뒤에 색. 예: `1.5 display-p3 0.2 0.3 0.5 1` |
| `border-bottom-left-radius`, `border-bottom-right-radius`, `border-top-left-radius`, `border-top-right-radius` | 비음수 유한 f32 |
| `clipping` | `disabled`, `frame-bounds`, 또는 `path <StableId>` |
| `height`, `width` | 비음수 유한 f32 |
| `layout-mode` | 참조 `StableId` 또는 `none` |
| `transform` | `a b c d tx ty` 순서의 유한 f32 여섯 개(CSS `transform` 행렬 하나) |

색은 `srgb 1 0 0 1`처럼 색 공간(`srgb` 또는 `display-p3`)과 비연관 알파 `r g b a`를 공백으로 구분해 한 줄에 씁니다. 알파는 `[0, 1]`, RGB는 확장 범위를 허용합니다.

Path clipping과 `layout-mode`는 참조 ID를 보존합니다. 미해결 Path 참조는 기존 문서 모델 규칙에 따라 보존할 수 있지만, Path 노드 payload나 layout 정의 자체를 이 파일에 추가하는 기능은 포함하지 않습니다. 이미 존재하는 잘못된 종류의 노드를 Path로 참조하면 문서 validator가 거부합니다.

## 문서 style 표(`document_font`)

`document.document_font`는 문서 style 표를 숨은 기본값 없이 모두 명시합니다. 표가 없는 기존 문서는 `document_font: none`입니다. 새 프로젝트의 문서는 다음과 같습니다.

```yaml
document_font:
  color: none
  paragraph:
    line-height: auto
    text-align: start
    typography: none
    wrapping: wrap
  run:
    color: none
    font-feature-settings: normal
    font-variation-settings: normal
    letter-spacing: none
    typography: none
  typography:
    id: "019a8180-0006-7000-8000-000000000006"
    font-family: none
    font-size: medium
    font-stretch: "100%"
    font-style: normal
    font-weight: "400"
    letter-spacing: normal
    line-height: normal
```

- `color`는 문서 기본 글자색(`none` 또는 색)입니다. `paragraph`는 문단 style 0, `run`은 문자 style 0입니다. `typography`는 문서 기본 typography resource 하나와 CSS longhand 일곱 개입니다.
- `font-family`는 `none`(빈 목록) 또는 큰따옴표로 감싼 이름을 `, `로 구분한 한 줄입니다(예: `"Pretendard", "system-ui"`). 이름 안의 `"`와 `\`는 `\`로 escape합니다.
- `font-size`는 `medium` 또는 1/64 px 단위를 정확히 표현하는 십진수와 `px`입니다. `font-weight`는 1~1000, `font-stretch`는 비음수 백분율, `font-style`은 `normal`, `italic`, `oblique <각도>deg`입니다. `line-height`는 `normal`, 단위 없는 비음수 배수, 또는 1/64 px 단위의 `px`입니다. `letter-spacing`은 `normal`, `px`, `em`입니다.
- 문단의 `line-height`는 `auto`, 배수, 또는 `px` 고정값이고 `text-align`은 `start`, `center`, `end`, `justify`, `wrapping`은 `wrap` 또는 `nowrap`입니다. 문자 style의 `letter-spacing`이 `none`이면 typography를 상속합니다. `font-feature-settings`와 `font-variation-settings`는 현재 `normal`만 지원합니다.
- 이 블록으로 표현할 수 없는 표는 저장 전에 거부합니다. 다른 index의 문단·문자 style, 추가 typography resource, 고정 face를 지정한 family, OpenType feature·variation, 등록 글꼴이 그런 경우입니다. 문서 style 표와 글꼴·이미지 asset record의 원본은 텍스트 asset catalog 한 곳에 두고 문서는 참조만 하기로 했습니다(ADR-0061 결정 6). catalog 형식은 IX.11.21 migration 경로를 정한 뒤 설계하며, 그 전까지 등록 글꼴이 있는 문서는 이 형식으로 내보낼 수 없습니다.

## 스키마와 값 규칙

모든 mapping은 아래 키만 허용하며, 나열한 키는 모두 필수입니다. 값이 기본값이거나 비어 있어도 키를 생략하지 않습니다. 알 수 없는 키, 중복 키, 빠진 키, 잘못된 타입은 오류입니다.

| 위치 | 필수 키와 의미 |
|---|---|
| 최상위 | `schema_version`, `document`, `page`, `view`, `nodes` |
| `document` | `id`, `display_name`, `document_font` |
| `page` | `id`, `display_name` |
| `view` | `id`, `display_name`, `root_frame` |
| `nodes`의 각 항목 | `id`, `parent`, `order_key`, `visible`, `locked`, `display_name`, `kind`, `payload` |
| Frame `payload` | [한 속성 한 줄](#한-속성-한-줄)의 열한 키 |
| 도형 `payload` | 종류별 기하 키와 `modifiers` 목록. [도형](#도형) 참조 |
| Text `payload` | `content`, `paragraphs`, `style_runs`, `placement` |
| ComponentInstance `payload` | `component`, `placement` |
| `placement` | `none` 또는 `id`, `order_key`, `translate-x`, `translate-y` mapping |
| Text 문단 | `style` (문단 style 표의 `u32` index) |
| Text style run | `start`, `end`, `style` (`u32`) |

`document`·`page`·`view`는 각각 하나의 mapping이며 배열이 아닙니다. `nodes`와 Text의 `paragraphs`·`style_runs`는 sequence입니다. 문서 validator가 ID 중복, root Frame, 부모 연결, 트리 깊이와 참조 불변식을 검증합니다. 부모가 없는 노드는 View의 root Frame이어야 합니다. 표시 이름은 빈 문자열을 포함해 문자열 그대로 보존하며 식별에 사용하지 않습니다.

| 필드 | 허용 값 |
|---|---|
| `id`, `root_frame` | 전체 `StableId`를 나타내는 하이픈 UUID 문자열 |
| `parent` | 부모 노드의 전체 `StableId`, root는 `none` |
| `order_key` | byte당 두 자리 hex 문자열. 빈 값·홀수 길이·마지막 byte `00`은 거부 |
| `visible`, `locked` | 소문자 `true` 또는 `false` |
| `kind` | `frame`, `fragment`, `rectangle`, `ellipse`, `line`, `polygon`, `text`, `component-instance` |
| Fragment `payload` | 정확히 `none`; 별도 시각·layout 필드 없음 |

숫자 여러 개를 담는 값(`transform`, `border`, 색)은 YAML sequence가 아닌 공백으로 구분한 scalar입니다. 지정된 숫자 개수가 다르거나 NaN·무한대이면 거부합니다. `none`은 없는 값을 나타내는 명시적 scalar이며 YAML null로 대체하지 않습니다. `display_name: none`처럼 문자열 필드에 쓴 `none`은 표시 이름 그대로입니다.

## Text 본문과 스타일 범위

Text의 `payload`는 본문과 기존 모델의 문단·style run을 모두 명시합니다. 다음 예시는 `확인` 뒤 LF가 있는 두 문단이며 범위는 UTF-8 byte가 아닌 Unicode scalar offset입니다.

```yaml
content: "확인\n다음"
paragraphs:
  -
    style: "0"
  -
    style: "0"
style_runs:
  -
    start: "0"
    end: "2"
    style: "0"
placement: none
```

`paragraphs` 길이는 본문의 LF 개수 + 1이어야 합니다. 빈 본문도 문단 하나를 가지며, 빈 `style_runs`는 `[]`입니다. 각 run은 `0 <= start <= end <= 본문 scalar 길이`를 만족해야 합니다. 정수는 `u32` 범위이며 음수·범위 초과는 거부합니다. 문단·run의 index는 모델의 숫자 그대로 보존하며 style 표에 없는 index도 미해결 참조로 남깁니다.

본문의 Unicode 정규화, CR/CRLF 변환, run 정렬·병합·겹침 제거를 하지 않습니다. 빈 run·겹치는 run·run 순서도 기존 payload 그대로 보존합니다. 제어 문자는 YAML 인용·이스케이프를 사용하고 읽을 수 없는 긴 scalar는 저장 전에 거부합니다.

## 배치(`placement`)

Text와 컴포넌트 인스턴스는 노드 소유의 단일 활성 Transform modifier를 배치로 기록합니다.

```yaml
kind: component-instance
payload:
  component: "019a8180-0030-7000-8000-000000000030"
  placement:
    id: "019a8180-0031-7000-8000-000000000031"
    order_key: "80"
    translate-x: "230.5"
    translate-y: "-10"
```

`placement.id`와 `order_key`는 modifier에서 가져옵니다. `translate-x`·`translate-y`는 부모 좌표계의 이동이며 선형부는 단위 행렬입니다. 명시적으로 (0, 0)에 배치한 노드도 이 modifier를 유지합니다. modifier가 없는 노드는 `placement: none`으로 저장합니다. Property·Slot override, 복수 modifier, 비활성 modifier, 회전·스케일 등이 있으면 저장을 거부합니다.

컴포넌트 인스턴스는 독립 정의의 전체 StableId를 참조합니다. 단일 파일 `check`는 인스턴스 노드와 참조 ID를 복원합니다. 참조하는 정의가 실제 프로젝트에 있는지는 `stellar validate <프로젝트>`로 확인합니다. 컴포넌트 정의 파일은 `design/components/*.stellar.yaml`에 따로 둡니다.

## 도형

기하 키는 이름 오름차순으로 쓰고 마지막에 `modifiers` 목록을 둡니다. 모든 길이·좌표는 논리 픽셀 f32입니다.

| `kind` | 기하 키 |
|---|---|
| `rectangle` | `border-bottom-left-radius`, `border-bottom-right-radius`, `border-top-left-radius`, `border-top-right-radius`, `height`, `width`. 화면에 맞춰 줄이기 전의 네 모서리 값을 보존합니다 |
| `ellipse` | `height`, `origin-x`, `origin-y`, `width`. 로컬 경계의 왼쪽 위 좌표와 크기입니다 |
| `line` | `cap`(`butt`, `round`, `square`), `end-x`, `end-y`, `start-x`, `start-y`. 그린 방향의 로컬 좌표입니다 |
| `polygon` | `corner_radius`(보정 전 반경), `height`, `origin-x`, `origin-y`, `rotation_degrees`, `sides`(3~1,000), `width` |

`modifiers` 목록은 원본의 `order_key` 순서로 기록합니다. 각 항목은 `id`, `order_key`, `enabled`, `kind`와 종류별 파라미터를 가지며, 항목이 없는 도형은 빈 목록을 씁니다.

| `kind` | 파라미터 |
| --- | --- |
| `transform` | `matrix`: `a b c d tx ty` |
| `fill` | `color`: 색 |
| `stroke` | `color`: 색, `width`: 비음수 논리 픽셀 |

## 텍스트 결정론과 파서 제한

기존 제한 YAML loader와 checked canonical writer를 사용합니다. loader는 모든 scalar를 문자열로 보존하고, codec이 ID·불리언·숫자를 명시적으로 해석합니다. 따라서 예시의 숫자와 불리언이 따옴표로 감싸져 있어도 같은 의미로 읽힙니다. writer는 재파싱해 같은 값 트리가 되는 출력만 반환합니다.

- 키 순서는 위 규칙대로 고정합니다. 노드는 `StableId` 오름차순으로 기록하며, 형제의 표시 순서는 `order_key`가 보존합니다.
- float은 `f32`가 정확히 왕복되는 최단 `Display` 표현을 씁니다. 색상·크기·변환 값을 8비트 색상이나 반올림한 값으로 바꾸지 않습니다.
- UTF-8, 공백 두 칸 들여쓰기, LF와 마지막 newline을 사용합니다. 문자열은 기존 writer의 인용·이스케이프 규칙을 따릅니다.
- 입력·출력 상한은 **1 MiB(1,048,576 bytes)**, YAML 컨테이너 중첩 상한은 **64**입니다. 이벤트 하나의 scanner 선읽기는 **32 Ki 문자**로 제한됩니다.
- anchor, alias, custom tag, merge key, 비스칼라 mapping 키, 중복 키, 다중 YAML document를 거부합니다. 주석은 모델의 일부가 아니므로 저장 시 보존하지 않습니다.

텍스트에는 runtime handle이나 binary payload의 hex·base64 blob을 넣지 않습니다. `order_key`의 hex는 순서 키 byte sequence의 표기이며, 노드나 chunk 전체를 숨긴 직렬화 blob이 아닙니다.

## 지원 범위 밖의 데이터

Path·Image payload, component library, token set, 도형의 Transform·Fill·Stroke 및 Text·인스턴스의 배치를 제외한 modifier stack, 다중 문서·페이지·View, 위 블록으로 표현할 수 없는 문서 style 표는 저장할 수 없습니다. 미지원 데이터를 만나면 원본을 보존하고 오류를 반환합니다.

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

에디터의 기본 열기·자동 저장 연결, codegen, 파일 감시와 외부 변경 자동 반영, Frame 경계로 여러 의미 파일을 나누는 저장 journal, 전체 프로젝트 마이그레이션, 텍스트 asset catalog, 추가 노드·component·token 직렬화는 아직 구현하지 않았습니다. 현재 보장하는 저장 원자성은 이 단일 텍스트 파일에 한정됩니다.
