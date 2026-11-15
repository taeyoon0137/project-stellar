# ADR-0061: 텍스트 원본의 형식·배치·읽기 정책

## Status

Accepted (2026-11-15). 제품 책임자가 텍스트 원본 형식과 배치의 판단을 위임했고, 2026-11-15 검토 결정(D2)으로
확정했습니다. [ADR-0059](ADR-0059-text-source-files-with-chunk-cache.md)가 "정하지 않은 사항"으로 남긴 텍스트 문법,
파일 확장자, 디렉터리 배치의 첫 범위를 정하고, ADR-0059 결정 6(위층 불변)을 아래 결정 9로 보완합니다.

## Context

- ADR-0059는 의미 단위 텍스트 파일을 Git 원본으로, chunk를 `.stellar/cache/`의 로컬 캐시로 쓰기로 정했습니다.
  문법·확장자·배치·전환 WBS는 후속으로 남겼습니다.
- 첫 구현(V.10.1~V.10.4와 그 뒤의 미커밋 작업)은 이 사항들을 정하면서 결정 문구를 Accepted 상태인 ADR-0059 본문에
  직접 덧붙였습니다. 결정의 출처가 흐려졌으므로 ADR-0059는 원래 문안으로 되돌리고 구현 결정을 이 ADR에 모읍니다.
- 2026-11-15 검토에서 확인한 형식 결함은 다음과 같습니다.
  - Frame을 `size`, `corner_radius`, 색 mapping 같은 묶음 값으로 저장했습니다. 한 줄이 여러 longhand를 담거나 한
    longhand가 여러 줄로 나뉘어, Git의 줄 단위 병합이 ADR-0024 결정 4·14의 병합 단위(longhand 하나)와 어긋났습니다.
  - View 원본 v2는 파일에 없는 문서 style 표를 `schema_version` 번호에서 복원했습니다(숨은 상수 표).
  - 같은 스키마를 자식 유무에 따라 다른 버전 번호로 쓰거나, 버전 번호가 문법을 구분하지 못했습니다.
  - export가 프로젝트 안에 두 번째 원본을 만들 수 있었고, 프로젝트 단위 조회의 symlink 정책이 기술 아키텍처 §19.3과
    달랐습니다.

## Decision

1. **문법과 확장자.** 원본은 기존 제한 YAML 하위 집합(제한 loader와 재파싱 검증 canonical writer)으로 쓰고 확장자는
   `.stellar.yaml`입니다. loader는 모든 scalar를 문자열로 보존하고 codec이 ID·불리언·숫자를 명시적으로 해석합니다.
   알 수 없는 키·빠진 키·중복 키·모르는 버전은 거부합니다.
2. **영속 속성 하나는 키 하나, 한 줄입니다.** Git의 줄 병합 단위가 ADR-0024 결정 4의 longhand 병합 단위와 같아지도록
   다음을 지킵니다.
   - 여러 longhand를 한 줄에 묶지 않습니다. Frame은 `width`·`height`와 네 모서리
     `border-top-left-radius`·`border-top-right-radius`·`border-bottom-right-radius`·`border-bottom-left-radius`를
     각각 한 줄로 씁니다(ADR-0024 결정 14의 PropertyKey와 같은 이름). 배치 평행이동은 `x`·`y`, 도형 좌표는 `x`·`y`·
     `x1`·`y1`·`x2`·`y2`입니다.
   - 값 하나를 여러 줄로 나누지 않습니다. 색은 `srgb 1 0 0 1`처럼 색 공간과 비연관 RGBA를 한 줄에 쓰고, Frame
     `border`(아직 longhand로 나뉘지 않은 묶음 key 0x0105)는 두께와 색을, `clipping`은 방식과 Path 대상을 한 줄에
     씁니다. `transform`은 CSS `transform` 행렬 하나이므로 여섯 성분을 한 줄에 씁니다.
   - 키 순서는 형식마다 고정합니다. 숫자는 정확히 왕복하는 최단 f32 표현입니다.
3. **숨은 상수 표를 두지 않습니다.** 파일에 없는 값을 버전 번호나 앱 기본값에서 복원하지 않습니다. View 원본의 문서
   style 표는 `document_font` 블록에 기본 typography resource 하나의 CSS longhand 일곱 개, 기본 글자색, 문단 style 0,
   문자 style 0을 모두 명시합니다. 이 범위로 표현할 수 없는 표(다른 index, 추가 resource, face가 지정된 family,
   OpenType feature·variation, 등록 글꼴)는 저장 전에 거부합니다.
4. **쓰기 버전은 형식마다 하나입니다.** View 원본은 `schema_version: "2"`, 컴포넌트 원본은 `"3"`만 씁니다.
   - 이미 커밋된 View v1과 컴포넌트 v1·v2는 읽기 전용입니다. 컴포넌트 v1·v2 파일을 편집하면 v3로 다시 씁니다. 무변경
     요청은 기존 bytes를 보존합니다.
   - 검토 전 미커밋 작업 트리에서만 쓰인 View v2(`initial_typography_id`)·v3(`document_font`와 `project_fonts`)와
     컴포넌트 v3(`corner_radius` 한 줄)의 문법은 같은 번호를 재정의하므로 읽지 않고 거부합니다.
   - 도형·컴포넌트 인스턴스·배치는 View v2에서만 받습니다.
5. **배치.**
   - 컴포넌트 정의는 프로젝트의 `design/components/`에 정의 하나 = 파일 하나로 둡니다. 파일명은
     `<이름>-<component StableId SHA-256 앞 12자리 hex>.stellar.yaml`이고 식별은 파일 안의 전체 StableId입니다.
     이름 부분만 다른 파일은 ID로 조립하고 진단을 돌려줍니다. 접미사가 다르면 거부합니다.
   - View는 ADR-0059 결정 1대로 Frame 의미 경계 분할을 목표로 하지만, 지금은 단일 View 파일 하나를 임시 형식으로
     유지합니다(V.10.4 후속). 기본 저장 전환(V.10.7) 전까지 View 원본은 명시 export로만 만들며, 프로젝트 루트 안으로의
     export는 generation과 함께 두 번째 원본이 되므로 거부합니다.
   - 로컬 캐시는 Git 밖 `.stellar/cache/text-source-v1/<원본 hash>/`입니다.
6. **문서 style 표와 에셋 레코드의 원본은 텍스트 asset catalog 한 곳입니다.** 문서·View·컴포넌트 원본은 catalog의
   항목을 참조만 합니다. catalog 형식은 제품 문서 migration 경로(IX.11.21)를 먼저 정한 뒤 별도로 설계합니다. 그
   전까지 View 원본은 결정 3의 문서 기본 style만 직접 기록하고, 글꼴·이미지 asset record를 담은 문서는 거부합니다.
7. **이중 원본을 만들지 않습니다(읽기·쓰기).** generation에 component-library chunk가 있는 프로젝트에서는 YAML 원본을
   읽거나 쓰지 않습니다. 컴포넌트 원본을 쓰는 명령의 대상 폴더가 Stellar 프로젝트 안에 있으면(조상 폴더에
   `stellar.yaml`이 있으면) 그 프로젝트의 저장 잠금을 잡고 같은 chunk 검사를 다시 한 뒤에만 씁니다. 프로젝트
   manifest를 읽을 수 없으면 쓰지 않습니다.
8. **symlink 읽기 정책은 신뢰 상태 하나로 정합니다.** 프로젝트 단위 컴포넌트 조회는 신뢰 상태에서 얻은
   `ExternalAccess`로 경로를 해석합니다(기술 아키텍처 §19.3). 신뢰한 프로젝트는 내부·외부 target을 모두 읽고,
   신뢰하지 않은 프로젝트는 외부 target의 내용을 읽지 않습니다. CLI·headless MCP는 OS 사용자 데이터의 신뢰 저장소를,
   앱은 창 세션의 신뢰 상태를 씁니다. 사용자가 경로를 직접 지정한 `component-source check <폴더>`는 신뢰한 프로젝트와
   같은 결과입니다.
9. **Semantic Diff·Merge 투영 보완(ADR-0059 결정 6).** 프로젝트 컴포넌트 원본의 정의는 문서에 속하지 않는 프로젝트
   범위 객체로 투영합니다. Origin `ProjectComponent`(교환 tag 4)와 Provenance `ProjectComponentDefinition`(code 5)을
   추가했습니다. 기존 tag·code의 의미는 바꾸지 않으므로 교환 인코딩 version은 1을 유지합니다.
   [의미 모델 §3.2·§4.5](../spec/stellar-semantic-model.md)

## Alternatives

| 대안 | 판단 |
|---|---|
| 전용 들여쓰기 문법 | 새 파서·fuzz 대상이 생깁니다. 이미 검증한 제한 YAML 파서·writer를 재사용합니다. 미선택 |
| 묶음 값 유지(`size: "w h"`, 색 mapping) | Git 줄 병합 단위가 longhand와 달라 한 축만 바꾼 두 브랜치가 충돌하거나, 한 색의 일부만 섞여 병합됩니다. 미선택 |
| 버전별 기본 표(v2 상수) | 앱 기본값이 바뀌면 같은 파일의 의미가 달라지거나 형식 상수가 숨은 원본이 됩니다. 미선택 |
| 글꼴 asset record를 View 파일에 직접 기록 | 같은 record가 문서마다 복제되어 원본이 여러 곳이 됩니다. catalog 한 곳에 둡니다. 미선택 |
| 프로젝트 조회에서 외부 symlink를 항상 거부 | §19.3과 다르고 신뢰한 프로젝트에서 명시 폴더 검사와 결과가 달라집니다. 미선택 |

## Consequences

- 장점
  - 서로 다른 longhand를 고친 두 브랜치는 그 줄 사이에 바뀌지 않은 줄이 있으면 일반 `git merge`로 합쳐집니다. 바로
    이웃한 줄의 변경은 Git이 충돌로 남기고 `stellar semantic-merge`가 속성 단위로 해결합니다.
  - 같은 bytes의 원본은 언제나 같은 문서이고, 새 프로젝트를 export→check→export하면 같은 bytes가 나옵니다.
- 단점
  - 등록 글꼴이 있는 문서는 asset catalog 전까지 View 원본으로 내보낼 수 없습니다.
  - 미커밋 작업 트리에서 만든 View v2·v3, 컴포넌트 v3 파일은 열리지 않습니다. 공개 전이라 사용자 데이터 이전 부담은
    개발용 fixture에 한정됩니다.

## Compatibility

- 파일 포맷: 위 결정 4의 버전만 읽고 씁니다. 공개 포맷 동결 전이므로 외부 호환 약속은 없습니다.
- 공개 API: CLI·MCP 컴포넌트 조회의 Frame payload 키와 색 값이 결정 2의 한 줄 표현을 따릅니다.
- 보호 경로 `crates/storage`의 `source_file`(제한 읽기, 조건부 원자 저장, 새 파일 게시, 캐시 chunk 갱신)은 이 ADR과
  ADR-0059를 근거로 씁니다. `write_new_resolved`는 프로젝트 내부로 해석한 컴포넌트 폴더에 새 파일만 게시하며, 앱의
  선택 생성 경로(D3에 따라 제거 예정)가 유일한 호출자입니다.

## Performance

- 프로젝트 단위 조회는 매 요청 원본 bytes를 읽고 파일 이름·bytes 집합 hash가 같으면 이전 해석을 재사용합니다.
  성능 예산 판정은 기본 저장 전환(V.10.7)에서 실측합니다.

## Security

- 텍스트 파서는 신뢰할 수 없는 저장소 입력을 받습니다. 파일당 1 MiB, 컴포넌트 4,096개, 프로젝트 조회 64 MiB, YAML
  중첩 64의 상한을 둡니다.
- 신뢰하지 않은 프로젝트는 외부 symlink target의 내용을 읽지 않습니다(결정 8).

## Migration

- 커밋된 View v1과 컴포넌트 v1·v2는 그대로 읽습니다. 컴포넌트는 다음 실제 편집에서 v3로 바뀝니다.
- 미커밋 작업 트리에서만 쓰인 문법은 변환하지 않고 거부합니다. 필요하면 generation에서 다시 export합니다.
- asset catalog와 문서 style 표 이전은 IX.11.21 migration 경로를 정한 뒤 별도 ADR로 정합니다.
