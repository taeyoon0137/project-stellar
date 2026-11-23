# ADR-0077: 텍스트 원본 전체 의미·쓰기 v4·다중 파일 배치·제품 migration registry

## Status

Accepted (2026-11-23). 엔진 lane `text-source-codec`(V.10.3·V.10.4·IX.11.21·TS-R2·TS-ASSET·TS-MAP, V.10.7 준비)의
결정입니다. [ADR-0059](ADR-0059-text-source-files-with-chunk-cache.md)와
[ADR-0061](ADR-0061-text-source-format-and-layout.md)의 본문은 고치지 않고, 이 ADR이 다음을 대체·보완합니다.

- ADR-0061 결정 2의 Frame 파일 키 이름과 줄 순서 → 아래 결정 2.
- ADR-0061 결정 4의 쓰기 버전(View v2, 컴포넌트 v3) → 아래 결정 2(둘 다 v4).
- ADR-0061 결정 5의 컴포넌트 파일 이름 규칙과 단일 View 임시 배치 → 아래 결정 1.
- ADR-0061 결정 6의 "catalog 형식은 IX.11.21 뒤 설계" → 아래 결정 1·3(catalog 형식 확정).

## Context

- ADR-0059·0061의 텍스트 원본은 단일 View(Frame·Fragment·도형·Text·인스턴스 배치)와 Free Frame 컴포넌트만
  담았습니다. Path·Image·대부분의 modifier·Text style 표 전체·글꼴·이미지 record는 쓸 수 없어, 기존 generation을
  텍스트로 옮기면 의미가 빠졌습니다(V.10.4 미완).
- ADR-0061의 Frame 파일 키는 registry 이름과 달랐습니다(`background`, `border-color`·`border-width`, 이름순 줄).
  ADR-0074의 "CSS 속성·값 그대로" 원칙과 css-registry lane(IX.11.1·IX.11.2·IX.11.22)이 정하는 registry 어휘를
  파일이 따르지 않았습니다.
- 제품 문서 migration registry가 없어 `SavedProjectLoader::open`은 schema를 보지 않고 current generation을 열었고,
  옛 schema bytes를 읽을 고정 decoder도 없었습니다(IX.11.21).
- 앱·AI 에이전트가 같은 파일을 편집하려면 node가 어느 파일 몇 번째 줄인지 core가 알려 줘야 합니다(UX.7.5 코어).

## Decision

1. **배치(다중 파일 원본).** project 루트의 `design/` 아래에 다음을 둡니다.
   - `document.stellar.yaml`: 문서 id·이름과 페이지 목록, 페이지마다 View 파일 참조(`design/` 기준 상대 경로) 순서.
   - `pages/<page-slug>/<view-slug>.stellar.yaml`: View 하나(id·이름·root Frame)와 그 node.
   - 큰 Frame은 하위 파일 `pages/<page-slug>/<view-slug>/<frame-slug>.stellar.yaml`로 나누고 부모 파일의 `nodes`에
     `$ref: <참조 파일 기준 상대 경로>` 항목을 둡니다. 분할 임계값은 **직계 자식 2,000개** 또는 **파일 1 MiB**입니다.
     직계 자식이 2,000개를 넘는 Frame은 항상 나누고, 파일이 1 MiB를 넘으면 그 파일 안에서 하위 tree가 가장 큰
     Frame부터 나눕니다. 결과는 모델만의 함수입니다.
   - `components/<slug>.stellar.yaml`: 컴포넌트 정의 하나 = 파일 하나.
   - `tokens/<slug>.stellar.yaml`: token set 하나 = 파일 하나.
   - `assets/catalog.stellar.yaml`: 이미지 asset record, 등록 글꼴(ProjectFontRecord와 글꼴 파일 AssetRecord), 문서별
     Text style 표의 유일한 원본. 문서·View·컴포넌트는 asset StableId·글꼴 file hash·style index로 참조만 합니다.
   - 파일 이름은 표시 이름의 slug이고 정체성은 파일 안의 StableId입니다. slug는 ASCII 소문자·숫자·`-`(최대 64자)만
     쓰며(파일 시스템·Git의 Unicode 정규화 차이 방지), 비면 종류 이름(`page`, `view`, `frame`, `component`,
     `tokens`)입니다. 같은 폴더에서 slug가 겹치면 겹친 항목 모두에 StableId SHA-256 앞 12자리를 붙입니다. 이름을
     바꾸면 그 파일 하나만 옮겨 Git rename이 됩니다. reader는 파일 이름을 식별에 쓰지 않습니다.
   - 한 프로젝트의 텍스트 원본은 root 문서 하나입니다. root 문서가 둘 이상인 generation은 쓰기를 거부합니다.
2. **쓰기 버전 v4.** View 원본·컴포넌트 원본·다중 파일 원본은 모두 `schema_version: "4"`만 씁니다.
   - 영속 속성 하나 = 키 하나·한 줄입니다. 키는 document 속성 registry(`frame_metadata`·`payload_metadata`·
     `modifier_metadata`)의 `name`이고 줄 순서는 registry 순서입니다. codec은 registry를 조회만 하므로 registry가
     이름을 CSS longhand로 고치거나 묶음 key를 나누면 codec 코드를 고치지 않고 파일 키가 따라갑니다.
   - 모델이 아직 묶음 값인 Frame `border`(색·두께 하나)는 CSS longhand 열두 줄(`border-{top,right,bottom,left}-{width,
     style,color}`)로 씁니다. 네 변은 같은 값이어야 하고, 테두리가 없으면 CSS 초기값(`medium`·`none`·
     `currentcolor`)입니다. 변마다 다른 값은 모델이 표현하지 않으므로 거부합니다. `clipping`과 `layout-mode`는
     stellar-only 키입니다.
   - 값은 CSS 문법입니다: 길이 `<number>px`, 각도 `<number>deg`, 색 `color(<space> r g b[ / a])`, 행렬
     `matrix(a, b, c, d, tx, ty)`, 단색 배경 없음 `transparent`. float은 정확히 왕복하는 최단 10진 표기만 받고 같은
     값의 다른 표기는 거부합니다.
   - registry에 key가 없는 모델 필드(도형 경계 원점, Path 명령, Image asset·crop·색 공간, Text 본문·문단·run,
     인스턴스 참조·override)는 codec 고유 키를 씁니다. Path 명령·문단·style run은 한 항목 한 줄입니다.
   - 모든 node 종류(Frame·Fragment·도형 넷·Path·Text·Image·ComponentInstance)와 모든 modifier 종류를 씁니다.
     modifier 항목의 키도 registry 이름(`enabled`, `order`, `fill.color` 등)입니다.
   - 모르는 키·값·버전은 거부합니다. View v1·v2와 컴포넌트 v1~v3은 읽기 전용이며 실제 편집 때만 v4로 다시 씁니다.
     같은 번호를 재정의하지 않습니다. 단, registry 이름이 공개 포맷 동결(V.10.7 전환) 전에 바뀌면 v4 파일 키도
     바뀝니다. 이 기간의 v4 파일은 명시 export로만 만들어지므로 다시 export합니다.
   - Git 병합 단위는 줄입니다. 바로 이웃한 두 줄(registry 순서에서 `width`와 `height`, 이웃한 모서리)을 서로 다른
     브랜치가 바꾸면 Git 텍스트 충돌로 남고 자동 해결은 merge driver(XXIV.2.1) 이후입니다. 사이에 다른 줄이 있는
     속성(예: `background`와 `width`)은 일반 `git merge`로 합쳐집니다.
   - CLI·MCP의 컴포넌트 조회 JSON 키는 MCP catalog schema가 고정한 기존 한 줄 표현을 유지합니다. 조회 payload를
     registry 이름으로 바꾸는 일은 css-registry lane이 MCP schema와 함께 합니다.
3. **손실 저장 없음.** 텍스트로 정확히 복원할 수 없는 의미가 하나라도 있으면 쓰기를 거부하고 generation을
   그대로 둡니다. 현재 거부 대상: Text style의 고정 face family, 해석할 수 없거나 알 수 없는 bytes를 보존한
   typography, 외부 출처 지문(HMAC)을 가진 asset record, 보존된 unknown optional field, 컴포넌트 Property·Slot·
   상태 머신(표현식 문법이 정해지기 전), 리터럴이 아닌 token 기본 표현식과 리터럴로 쓰지 않는 값 타입(enum·
   gradient·shadow·typography·resource-ref), 하나를 넘는 root 문서, 알 수 없는 chunk.
4. **제품 migration registry(IX.11.21).** `crates/project/src/migration/`에 둡니다.
   - 제품 schema version은 generation manifest의 `schema_version`이고 현재 1입니다. 제품 step 표
     (`PRODUCT_STEPS`)만 제품 registry에 들어가며 1부터 빈틈 없이 이어져야 합니다. storage의 시험용 noop step과
     xtask fixture step은 이 표를 지나지 않아 번호가 섞이지 않습니다.
   - IX.11.6·IX.11.9·IX.11.10·IX.11.11의 schema 변경은 공개 포맷(Phase XXVIII) 전 version 증가 한 번(1 → 2)으로
     묶습니다. step은 옛 bytes를 schema version별 고정 decoder 사본으로 읽습니다. schema 1 Frame은 현재 타입과 독립된
     `FrameV1` 사본이고, 다른 payload는 현재 decoder와 bytes가 같아 golden 테스트로 고정합니다.
   - `SavedProjectLoader::open`은 V.8.1 판정을 적용합니다: 같은 schema는 편집 가능, 낮은 schema는 경로가 있으면
     메모리에서만 migration한 읽기 전용, 경로가 없거나 높은 schema는 거부. 열기는 디스크를 쓰지 않고 결과를 자동
     commit하지 않습니다. Git snapshot(HEAD·Index·commit·merge base)도 이 경로로 열려 메모리에서 최신 schema로
     맞춰집니다. 디스크에 새 generation을 쓰는 일은 명시 `stellar migrate`(V.8.4 경로)만 합니다.
5. **기본 열기·저장 전환(V.10.7) 조건.** 이 lane은 전환하지 않습니다. 전환은 (가) 기존 generation fixture 전체의
   semantic snapshot 동일성 corpus 통과, (나) 다중 파일 저장 journal(V.10.5) 연결, (다) 외부 변경 반영(V.10.6)
   연결 뒤에 합니다. 전환 뒤 generation은 읽기·이전 전용이고 chunk는 원본 hash 키로 된 `.stellar/cache/` 파생물입니다.
   그 전까지 다중 파일 원본은 명시 export로 프로젝트 밖에만 만들어 두 원본을 동시에 쓰지 않습니다.
6. **node → 줄 범위(TS-MAP).** codec이 쓴 텍스트의 `nodes` 항목에서 span table(node StableId → `design/` 기준
   파일·시작 줄·끝 줄)을 만듭니다. 읽을 때는 디스크 bytes를 같은 규칙으로 스캔하므로 줄 번호는 그 파일 기준입니다.

## Alternatives

| 대안 | 판단 |
|---|---|
| ADR-0061의 이름순 Frame 줄 순서 유지 | width·height처럼 자주 따로 바뀌는 줄을 떨어뜨리지만, 파일 키가 registry와 따로 놀아 codegen·MCP와 이름이 갈립니다. lane 결정대로 registry 순서를 씁니다. 미선택 |
| 묶음 `border`를 두 줄(`border-color`·`border-width`)로 유지 | CSS shorthand 이름이라 "CSS 그대로" 원칙과 다릅니다. 미선택 |
| 색을 `#rrggbb[aa]`로 쓰기 | 8비트로 반올림되어 왕복하지 않습니다. 미선택 |
| 파일 이름에 항상 StableId 접미사 | 이름 변경마다 diff가 커지고 읽기 어렵습니다. 충돌 때만 붙입니다. 미선택 |
| Unicode slug | 같은 이름이 NFC·NFD 두 경로가 될 수 있습니다. 미선택 |
| migration을 storage crate에 두기 | storage는 document에 의존하지 않아 node record를 해석하지 못합니다. 미선택 |
| 열 때 옛 schema를 디스크에 바로 migration | 사용자가 원하지 않은 변경·자동 commit 위험. 메모리 migration만 합니다. 미선택 |

## Consequences

- 장점: 기존 generation의 전체 의미(모든 node·modifier·컴포넌트 tree·Text style 표·글꼴·이미지 record)를 텍스트로
  손실 없이 옮기고 다시 읽어 같은 semantic snapshot을 얻습니다. 파일 키가 registry 어휘를 따라 codegen·MCP와
  이름이 맞춰집니다. AI·CLI가 node의 파일·줄을 바로 찾습니다.
- 단점: registry 순서에서 이웃한 줄(width·height, 이웃한 모서리)은 Git 텍스트 충돌입니다(ADR-0061보다 후퇴).
  Frame 하나가 테두리 열두 줄을 써 파일이 길어집니다. 컴포넌트 Property·Slot·상태 머신과 표현식 token은 아직
  텍스트로 쓰지 않습니다.

## Compatibility

- 파일 포맷: View·컴포넌트 원본 쓰기 버전이 v4로 바뀝니다. 커밋된 View v1·v2, 컴포넌트 v1~v3은 계속 읽습니다.
  공개 포맷 동결 전이라 외부 호환 약속은 없습니다.
- 컴포넌트 파일 이름: 새 파일은 slug 이름입니다. 기존 `<이름>-<hash12>` 파일은 이름 그대로 읽습니다.
- 공개 API: `SavedProjectError`에 `Schema`·`Migration`이 추가되었고, 새 schema generation은 열기 단계에서 거부됩니다.
  CLI·MCP 조회 JSON은 바뀌지 않습니다.
- FFI·보호 경로(`crates/core-types`, `crates/storage`, `crates/ffi`, schemas, 성능 예산)는 바꾸지 않습니다.

## Performance

- 다중 파일 export·check는 전체를 해석·검증합니다. 빠른 열기와 cache GC는 V.10.7 전환에서 원본 hash 키 cache로
  연결하고 그때 열기·저장 예산을 실측합니다. 1 MiB 파일 상한으로 파일 하나의 해석 비용을 묶습니다.

## Security

- 모든 파일은 기존 제한 YAML loader(파일당 1 MiB, 중첩 64)로 읽습니다. `$ref`는 참조 파일 아래의 상대 경로만
  받고 `..`·절대 경로·symlink를 거부하며, 같은 파일을 두 번 참조하거나 참조하지 않은 파일이 있으면 거부합니다.
  다중 파일 원본 전체 읽기는 64 MiB, 파일 4,096개로 제한합니다.

## Migration

- 기존 프로젝트는 `stellar text-source export-project`로 generation을 다중 파일 원본으로 옮깁니다. 변환은 다시 읽은
  결과의 semantic snapshot이 원본과 같을 때만 성공하며 원본 generation은 바꾸지 않습니다.
- 단일 View v2·컴포넌트 v3 파일은 다음 실제 편집에서 v4로 바뀝니다.
