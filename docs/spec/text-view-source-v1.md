# 단일 View 텍스트 원본 v1

[ADR-0059](../adr/ADR-0059-text-source-files-with-chunk-cache.md)의 첫 opt-in 구현 형식입니다. 문서 하나, 페이지 하나, View 하나와 그 아래 Frame·Fragment를 사람이 읽는 YAML 파일 하나에 보관합니다. 기존 프로젝트의 기본 열기·저장 경로는 세대 기반 chunk 저장을 계속 사용합니다. 이 형식은 전체 프로젝트 포맷 전환이나 공개 포맷 동결을 뜻하지 않습니다.

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

## 스키마와 값 규칙

모든 mapping은 아래 키만 허용하며, 나열한 키는 모두 필수입니다. 값이 기본값이거나 비어 있어도 키를 생략하지 않습니다. 알 수 없는 키, 중복 키, 빠진 키, 잘못된 타입은 오류입니다. `schema_version`은 정확히 `1`만 지원합니다.

| 위치 | 필수 키와 의미 |
|---|---|
| 최상위 | `schema_version`, `document`, `page`, `view`, `nodes` |
| `document` | `id`, `display_name` |
| `page` | `id`, `display_name` |
| `view` | `id`, `display_name`, `root_frame` |
| `nodes`의 각 항목 | `id`, `parent`, `order_key`, `visible`, `locked`, `display_name`, `kind`, `payload` |
| Frame `payload` | `size`, `transform`, `clipping`, `background`, `border`, `corner_radius`, `layout_mode` |
| 색상 | `space`, `rgba` |
| 테두리 | `color`, `thickness` |

`document`·`page`·`view`는 각각 하나의 mapping이며 배열이 아닙니다. `nodes`만 sequence입니다. 문서 validator가 ID 중복, root Frame, 부모 연결, 트리 깊이와 참조 불변식을 검증합니다. 부모가 없는 노드는 View의 root Frame이어야 합니다. 표시 이름은 빈 문자열을 포함해 문자열 그대로 보존하며 식별에 사용하지 않습니다.

| 필드 | 허용 값 |
|---|---|
| `id`, `root_frame` | 전체 `StableId`를 나타내는 하이픈 UUID 문자열 |
| `parent` | 부모 노드의 전체 `StableId`, root는 `none` |
| `order_key` | byte당 두 자리 hex 문자열. 빈 값·홀수 길이·마지막 byte `00`은 거부 |
| `visible`, `locked` | 소문자 `true` 또는 `false` |
| `kind` | `frame` 또는 `fragment` |
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

## 텍스트 결정론과 파서 제한

기존 제한 YAML loader와 checked canonical writer를 사용합니다. loader는 모든 scalar를 문자열로 보존하고, codec이 ID·불리언·숫자를 명시적으로 해석합니다. 따라서 예시의 숫자와 불리언이 따옴표로 감싸져 있어도 같은 의미로 읽힙니다. writer는 재파싱해 같은 값 트리가 되는 출력만 반환합니다.

- 키 순서는 위 예시처럼 고정합니다. 노드는 `StableId` 오름차순으로 기록하며, 형제의 표시 순서는 `order_key`가 보존합니다.
- float은 `f32`가 정확히 왕복되는 최단 `Display` 표현을 씁니다. 색상·크기·변환 값을 8비트 색상이나 반올림한 값으로 바꾸지 않습니다.
- UTF-8, 공백 두 칸 들여쓰기, LF와 마지막 newline을 사용합니다. 문자열은 기존 writer의 인용·이스케이프 규칙을 따릅니다.
- 입력·출력 상한은 **1 MiB(1,048,576 bytes)**, YAML 컨테이너 중첩 상한은 **64**입니다. 이벤트 하나의 scanner 선읽기는 **32 Ki 문자**로 제한됩니다.
- anchor, alias, custom tag, merge key, 비스칼라 mapping 키, 중복 키, 다중 YAML document를 거부합니다. 주석은 모델의 일부가 아니므로 저장 시 보존하지 않습니다.

텍스트에는 runtime handle이나 binary payload의 hex·base64 blob을 넣지 않습니다. `order_key`의 hex는 순서 키 byte sequence의 표기이며, 노드나 chunk 전체를 숨긴 직렬화 blob이 아닙니다.

## 지원 범위 밖의 데이터

Shape·Path·Text·Image payload, component instance·library, token set, modifier stack과 다중 문서·페이지·View는 이번 범위에 포함하지 않습니다. 미지원 데이터를 만났을 때 생략한 결과를 저장하지 않고 실패합니다. disabled modifier도 stack 데이터이므로 거부합니다.

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
