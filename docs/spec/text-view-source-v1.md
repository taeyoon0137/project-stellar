# 단일 View 텍스트 원본 v1

[ADR-0059](../adr/ADR-0059-text-source-files-with-chunk-cache.md)의 첫 opt-in 구현 형식입니다. 문서 하나, 페이지 하나, View 하나와 그 아래 Frame·Fragment·Text를 사람이 읽는 YAML 파일 하나에 보관합니다. 기존 프로젝트의 기본 열기·저장 경로는 세대 기반 chunk 저장을 계속 사용합니다. 이 형식은 전체 프로젝트 포맷 전환이나 공개 포맷 동결을 뜻하지 않습니다.

구현 원본은 [`text_codec.rs`](../../crates/project/src/document_io/text_codec.rs), 파일 입출력은 [`text_source.rs`](../../crates/project/src/document_io/text_source.rs), 기존 프로젝트 export는 [`ops/text_source.rs`](../../crates/ops/src/text_source.rs)입니다.

## 파일 이름과 CLI

파일 이름은 `home-00000003.stellar.yaml`처럼 사람이 읽는 View 이름의 slug와 View `StableId`의 짧은 고정 부분을 조합하는 것을 권장합니다. 확장자·slug 생성·충돌 처리를 자동화하지 않으며, CLI에 출력 경로를 직접 지정합니다. 식별 기준은 파일 안의 전체 `StableId`입니다. 이름을 바꿔도 ID는 유지합니다.

```sh
stellar text-source export ./MyProject ./home-00000003.stellar.yaml
stellar text-source check ./home-00000003.stellar.yaml
stellar text-source apply ./home-00000003.stellar.yaml ./edit.json --dry-run
stellar text-source apply ./home-00000003.stellar.yaml ./edit.json
```

`export <project> <new-file>`은 지원 범위의 기존 저장 프로젝트를 새 파일로 내보냅니다. 출력 파일이 이미 있으면 실패하며 덮어쓰지 않습니다. 기존 프로젝트의 원본과 generation은 변경하지 않습니다. `check <file>`은 텍스트를 읽어 문법·지원 범위·문서 불변식을 검증합니다. `check`와 라이브러리 `open`은 원본·캐시를 쓰지 않는 읽기 전용 작업입니다.

## 명시적 명령 적용

`apply <file> <input|-> [--dry-run]`은 내보낸 단일 View 원본에 기존 command transaction을 적용합니다. 입력 파일 대신 `-`로 표준 입력을 읽을 수 있습니다. `check`/`export` 응답의 `document`와 `source_hash`를 요청에 사용합니다. `stellar-apply`의 generation revision과 혼용하지 않습니다.

```json
{
  "format": "stellar-text-apply",
  "format_version": 1,
  "base_source_hash": "sha256:<check가 반환한 소문자 hex 64자리>",
  "document": "019a8180-0001-7000-8000-000000000001",
  "commands": [
    {"kind": "rename", "node": "019a8180-0011-7000-8000-000000000011", "name": "확인 버튼"},
    {"kind": "set-visible", "node": "019a8180-0011-7000-8000-000000000011", "visible": false}
  ]
}
```

요청의 키는 위 다섯 개만 허용합니다. 입력은 기존 apply와 같은 8 MiB·깊이 16·명령 10,000개 상한을 사용합니다. `rename`, `set-visible`, `set-locked`, `move-node`, `delete-subtree` 필드 형식과 등록된 명령의 `canonical` 소문자 hex 형식은 기존 `stellar apply`와 같습니다. command validator와 전체 transaction 실행을 거치므로 중간 실패는 저장하지 않습니다. 미지원 노드·modifier·속성이 생기는 명령은 결과 텍스트 직렬화 단계에서 **dry-run과 쓰기 모두 거부**합니다.

Frame의 크기와 local 아핀 변환은 두 apply 경로 모두에서 JSON 숫자로 지정할 수 있습니다. `node`는 Frame의 전체 StableId이며 아래 필드는 모두 필수입니다. 다른 키나 숫자를 담은 문자열은 거부합니다.

```json
{"kind":"set-frame-size","node":"019a8180-0010-7000-8000-000000000010","width":320.5,"height":180}
{"kind":"set-frame-transform","node":"019a8180-0010-7000-8000-000000000010","a":1,"b":0,"c":0,"d":1,"tx":12.75,"ty":-30}
```

각 object를 요청의 `commands` 배열에 넣습니다. 숫자는 원문에서 모델의 `f32`로 변환하며 정수·소수·지수 표기를 허용합니다. `f32` 변환 결과가 무한대이면 오류이고, `width`·`height`는 비음수여야 합니다. 변환은 `(x, y)`를 `(a*x + c*y + tx, b*x + d*y + ty)`로 보냅니다. 이 두 입력은 기존 `SetProperty`의 `size`·`transform`에 대응하므로 새 저장 opcode나 바이너리 포맷을 만들지 않습니다. Frame이 아닌 노드에 적용하면 validator가 전체 transaction을 거부합니다. 현재 모델의 묶음 속성을 사용하며, CSS width/height longhand 전환(IX.11.22)의 완료를 뜻하지 않습니다.

- dry-run은 원본·generation·cache·잠금 파일을 쓰지 않고 적용될 semantic `changes`를 반환합니다. `source_hash`는 디스크의 기존 hash이며 `written`은 false입니다.
- 쓰기는 기존 OS 사용자 데이터 저장 잠금을 문서 ID와 canonical 원본 파일 경로로 잡은 뒤 원본을 다시 읽고 hash·문서 ID를 확인합니다. 동일 파일의 협조적 CLI writer는 직렬화합니다. 정상 저장 결과에는 새 `source_hash`와 `written: true`가 있고 `check`로 재열 수 있습니다.
- 동일 모델이 되는 명령은 원본 주석·공백·hash를 그대로 보존하고 `written: false`를 반환합니다. 실제 의미 변경 시에는 v1의 결정론적 writer가 파일 전체를 다시 쓰므로 주석·서식은 보존하지 않습니다.
- 외부 수정에는 주석·공백 변경도 포함하며 `text-source-changed`(75)로 거부합니다. 다시 `check`하고 변경을 검토한 후 새 hash로 요청해야 합니다. 알 수 없는 원본 필드·레코드는 `text-source-unsupported`(69)로 거부하고 원본을 덮어쓰지 않습니다. 명령 실패(1), 잘못된 요청·문서 ID(65), 저장 잠금 경합(75), 잠금용 사용자 데이터 위치 누락(78)을 구분합니다.

기존 프로젝트에서 export한 파일을 이 명령으로 고쳐도 기존 generation은 변경하지 않습니다. 기본 앱에서 그 변경을 자동으로 읽는 연결은 아직 없으므로, 이 CLI는 명시적으로 지정한 텍스트 파일만의 편집 경로입니다. 저장 후 일반 `git diff -- <file>`로 사람이 읽는 속성 변경을 검토할 수 있습니다.

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
| Text `payload` | `content`, `paragraphs`, `style_runs` |
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
| `kind` | `frame`, `fragment`, `text` |
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

`paragraphs` 길이는 본문의 LF 개수 + 1이어야 합니다. 빈 본문도 문단 하나를 가지며, 빈 `style_runs`는 `[]`입니다. 각 run은 `0 <= start <= end <= 본문 scalar 길이`를 만족해야 합니다. 정수는 `u32` 범위이며 음수·범위 초과는 거부합니다. 문단 style과 run style은 기존 모델의 숫자 index를 보존하며, 별도 font/style 정의를 이 파일 안에 만들어 넣지 않습니다.

본문의 Unicode 정규화, CR/CRLF 변환, run 정렬·병합·겹침 제거를 하지 않습니다. 빈 run·겹치는 run·run 순서도 기존 payload 그대로 보존합니다. 제어 문자는 YAML 인용·이스케이프를 사용하고 읽을 수 없는 긴 scalar는 저장 전에 거부합니다. 본문 자체에 전용 CLI 편집 명령은 아직 없으며 외부 텍스트 편집 후 `check`로 검증할 수 있습니다. 기존 `rename`·표시 여부 등 CLI 명령은 Text payload 전체를 보존하면서 적용됩니다.

V.10.4의 부분 확장이므로 schema version은 1을 유지합니다. 이전 구현은 `kind: text`를 미지원으로 거부해 덮어쓰지 않으며, 기존 Frame/Fragment 파일의 bytes·문법은 바뀌지 않습니다.

## 텍스트 결정론과 파서 제한

기존 제한 YAML loader와 checked canonical writer를 사용합니다. loader는 모든 scalar를 문자열로 보존하고, codec이 ID·불리언·숫자를 명시적으로 해석합니다. 따라서 예시의 숫자와 불리언이 따옴표로 감싸져 있어도 같은 의미로 읽힙니다. writer는 재파싱해 같은 값 트리가 되는 출력만 반환합니다.

- 키 순서는 위 예시처럼 고정합니다. 노드는 `StableId` 오름차순으로 기록하며, 형제의 표시 순서는 `order_key`가 보존합니다.
- float은 `f32`가 정확히 왕복되는 최단 `Display` 표현을 씁니다. 색상·크기·변환 값을 8비트 색상이나 반올림한 값으로 바꾸지 않습니다.
- UTF-8, 공백 두 칸 들여쓰기, LF와 마지막 newline을 사용합니다. 문자열은 기존 writer의 인용·이스케이프 규칙을 따릅니다.
- 입력·출력 상한은 **1 MiB(1,048,576 bytes)**, YAML 컨테이너 중첩 상한은 **64**입니다. 이벤트 하나의 scanner 선읽기는 **32 Ki 문자**로 제한됩니다.
- anchor, alias, custom tag, merge key, 비스칼라 mapping 키, 중복 키, 다중 YAML document를 거부합니다. 주석은 모델의 일부가 아니므로 저장 시 보존하지 않습니다.

텍스트에는 runtime handle이나 binary payload의 hex·base64 blob을 넣지 않습니다. `order_key`의 hex는 순서 키 byte sequence의 표기이며, 노드나 chunk 전체를 숨긴 직렬화 blob이 아닙니다.

## 지원 범위 밖의 데이터

Shape·Path·Image payload, component instance·library, token set, modifier stack과 다중 문서·페이지·View는 이번 범위에 포함하지 않습니다. 미지원 데이터를 만났을 때 생략한 결과를 저장하지 않고 실패합니다. disabled modifier도 stack 데이터이므로 거부합니다.

기존 프로젝트 export는 로드한 문서를 다시 직렬화한 **전체 chunk pointer 집합**이 저장된 generation과 정확히 같아야 합니다. ChunkId·내용 hash·kind·schema version을 비교하고 generation schema와 단일 root 문서도 검사합니다. 따라서 문서 밖 메타데이터, component·token chunk, 사용되지 않는 추가 chunk, 알 수 없는 optional chunk도 조용히 버리지 않습니다. 이미 알려진 kind라는 이유만으로 허용하지 않습니다. 지원되는 단일 View의 예상 집합은 DocumentManifest 하나, ViewTree 하나, subtree 하나입니다.

이 검사는 지원되는 원본의 명시적 export만 허용하는 경계입니다. 프로젝트 전체를 자동 마이그레이션하거나 기존 원본 파일을 삭제·대체하지 않습니다.

## 열기·저장과 파생 캐시

라이브러리 `save_new`는 완성된 임시 파일을 hard link로 게시하므로 마지막 검사 후 다른 파일이 생겨도 기존 항목을 덮어쓰지 않습니다. hard link를 지원하지 않는 파일 시스템은 오류로 거부합니다. `save`는 열었을 때의 원본 bytes hash와 문서 ID를 확인한 뒤 단일 파일을 원자적으로 교체합니다. 원본이 외부에서 바뀌었거나 재해석할 수 없는 필드가 생겼으면 덮어쓰지 않습니다. CLI에는 `export`, 읽기 전용 `check`, 명시적 `apply`를 노출합니다. 저장은 기존 파일 권한을 유지합니다. 마지막 hash 검사와 rename 사이에 비협조적 외부 writer가 개입하는 경쟁은 기존 atomic 저장과 같이 남아 있으므로 CLI는 협조적 writer 저장 잠금을 사용하며 앱 연결 전 프로젝트 전체 잠금과 외부 변경 통합이 필요합니다. 부모 디렉터리 fsync 또는 새 파일 게시 뒤 임시 링크 제거 실패는 대상이 이미 생긴 뒤 반환될 수 있으므로 오류 후 실제 원본을 다시 읽어 상태를 확인해야 합니다.

캐시는 라이브러리 `open_with_cache(path, project_root)`를 명시적으로 호출할 때 생성합니다.

```text
<project_root>/.stellar/cache/text-source-v1/<sourcehash>/<ChunkId>.starchunk
```

`sourcehash`는 실제 UTF-8 원본 bytes의 SHA-256 digest를 소문자 hex 64자로 쓴 값입니다. 주석·공백 변경도 다른 캐시 키가 됩니다. 텍스트에서 기존 serializer의 DocumentManifest·ViewTree·subtree 세 chunk를 재생성하고 기존 envelope 형식으로 기록합니다. 같은 bytes이면 유지하고, 없거나 손상됐으면 다시 만듭니다. 캐시 전용 디렉터리의 `.gitignore`(`*`)를 먼저 생성·검증하므로 기존 프로젝트에도 새 캐시가 Git 변경으로 나타나지 않습니다. 기존 marker 내용이 다르면 덮어쓰지 않고 cache 오류로 보고합니다. 캐시는 Git 밖의 파생 데이터이며 삭제해도 텍스트 원본에서 재구축할 수 있습니다. 캐시 I/O 실패는 상태로 보고하고, 유효한 원본의 열기 결과는 반환합니다.

현재는 캐시가 있어도 **열 때마다 텍스트 전체를 해석·검증하고 chunk를 재생성해 비교**합니다. 빠른 열기·부분 로딩·성능 예산 충족을 주장하지 않습니다. 캐시 GC도 이번 구현 범위 밖입니다.

## 후속 범위

에디터의 기본 열기·자동 저장 연결, codegen, 파일 감시와 외부 변경 자동 반영, 여러 의미 파일로 분할하는 저장 journal, 전체 프로젝트 마이그레이션, 추가 노드·component·token 직렬화는 아직 구현하지 않았습니다. 현재 보장하는 저장 원자성은 이 단일 텍스트 파일에 한정됩니다.
