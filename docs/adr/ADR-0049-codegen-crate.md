# ADR-0049: 단방향 코드 생성 crate(codegen)와 중립 IR 입력 경계

## Status

Accepted (2026-11-14). 제품 소유자가 기술 문서에 세부가 없는 설계 결정(새 crate, crate graph 간선)을 ADR 기록 조건으로 위임한 범위(2026-11-14)에서 정했습니다.

## Context

- 기술 아키텍처 §17.3은 Web·Apple·Android용 단방향 코드 생성을 로드맵에 둡니다. Stellar 문서가 원본이고, 생성 코드의 역수입·자동 동기화는 하지 않으며, 같은 입력에서 결정론적으로 생성하고 변환 불가·근사 변환을 진단합니다.
- Phase XXII의 WBS 행(XXII.1~XXII.5)은 중립 `CodegenIR`과 플랫폼별 adapter를 전제로 하지만, 기술 아키텍처 §7 저장소 구성과 crate graph에 이를 담을 crate가 없습니다.
- IR 입력은 이미 하위 crate의 지역 모델로 있습니다: target-profile의 `ViewModel`·override resolver·다중 profile validator(IX.10.7·IX.10.8), accessibility의 중립 snapshot(IX.8.6), layout의 Fragment flatten layout tree(X.1.2). 이 crate들은 모두 core-types에만 의존합니다.
- 문서 모델(document)이나 binding graph(bindings)에 직접 의존하면 코드 생성이 저장 형식·편집 command 계층과 묶이고, 입력 해석 책임이 adapter마다 흩어집니다.

## Decision

1. `crates/codegen`(`stellar-codegen`)을 추가합니다. 허용 직접 의존은 `core-types`, `target-profile`, `accessibility`, `layout`입니다. 문서·binding·project·renderer crate에는 의존하지 않습니다.
2. 입력 경계는 하위 crate의 지역 모델입니다: `ViewModel`(구조·공통값·profile override), `VariantLayer`, `LayoutInput`, `AccessibilitySnapshot`과, 이 crate가 정의하는 상태 변수·token 선언, Text 내용, PropertyPath → token binding 표입니다. 문서 계층이 이 값을 채워 넘깁니다(target-profile·accessibility와 같은 방식).
3. `CodegenIr`은 Target Profile 하나로 해석한 결과입니다. 구조·layout·semantics·state·tokens·target capability를 담고, 결정론 semantic snapshot 텍스트를 제공합니다. 생성 코드를 IR이나 문서로 되읽는 API는 두지 않습니다.
4. 플랫폼 adapter(Web·Apple·Android)는 이 crate 안의 module로 두고 IR만 읽습니다.
5. 외부 의존성 없이 시작합니다. 외부 crate가 필요하면 `config/dependencies.yaml`과 별도 ADR로 추가합니다.

## Alternatives

- **export crate에 넣기**: export는 Scene Snapshot을 입력으로 받는 PNG·SVG 내보내기(ADR-0019)이고 core-types·scene에만 의존합니다. 코드 생성은 Scene이 아니라 의미 구조(layout 규칙·접근성·상태)를 입력으로 하므로 의존 방향이 다릅니다. 채택하지 않습니다.
- **document에 직접 의존**: 입력을 한 번에 얻을 수 있지만 저장 형식과 편집 계층 변경이 코드 생성까지 번지고, 하위 crate의 지역 모델(validator·resolver·snapshot)과 다른 해석 경로가 생깁니다. 채택하지 않습니다.
- **플랫폼별 crate 분리(codegen-web 등)**: adapter가 같은 IR·진단·결정론 규칙을 공유하므로 지금은 crate 하나의 module로 충분합니다. 나중에 빌드 시간 문제가 생기면 다시 검토합니다.

## Consequences

- 긍정: 코드 생성 작업을 문서 모델 변경과 독립적으로 진행할 수 있고, 같은 IR로 세 플랫폼 adapter를 결정론적으로 비교할 수 있습니다.
- 부정: 전체 코드 생성에는 문서 계층에서 IR 입력을 채우는 연결 코드가 따로 필요합니다(아직 없음). 입력 모델이 하위 crate에서 바뀌면 IR 변환도 함께 바뀝니다.

## Compatibility

파일 포맷, 공개 FFI, 저장 구조에 영향이 없습니다. semantic snapshot 텍스트는 fixture 비교용이며 파일 형식이 아닙니다.

## Performance

핵심 입력·렌더링 경로에 들어가지 않습니다. IR 변환은 node 수에 대해 O(N log N)이고 재귀를 쓰지 않습니다.

## Security

생성 코드를 실행하거나 되읽지 않습니다. 입력 텍스트(이름·문자열)는 adapter가 대상 언어 규칙으로 escape합니다.

## Migration

없습니다.

## 정적 접근성 HTML 연결 (2026-11-14)

- 저장된 프로젝트의 명시한 문서·View를 CLI로 내보내는 사용자 요청에 따라 `ops → codegen`,
  `ops → accessibility`, `ops → target-profile` 간선을 허용합니다. CLI의 직접 의존은 ops로 유지하며,
  document와 codegen 사이의 역방향 간선이나 새 외부 의존은 추가하지 않습니다.
- 현재 `DocumentModel`은 control 의미·상태·token·Variant를 복원하지 못하고, 저장된 target profile도
  codegen의 registry pin 형식과 연결되지 않았습니다. 없는 layout·capability를 만들어 `CodegenSources`를
  채우지 않습니다. 기존 `CodegenIr`의 tree 부분을 `IrTree`로 분리하고 `build_accessibility_ir`는
  구조·저장 텍스트·접근성 snapshot만 받습니다. 전체 adapter 입력 `CodegenIr`에는 여전히 target과
  기존 `build_ir` 검증이 필요합니다. 기존 플랫폼 golden은 그대로 유지합니다.
- `stellar export-accessibility <경로> <문서 id> <View id>`는 `ProjectSnapshot::document_model`과
  `View.root_frame`만 사용합니다. Frame·Fragment·Text의 정적 의미를 기존 접근성 생성기와
  `web::aria::map_accessibility`로 매핑하고 같은 정적 검사를 실행합니다. 이름을 role·label로 해석하지
  않습니다. Shape·Path·Image는 의미·에셋 투영이 없어 오류로 거부합니다(숨김 node도 동일).
- 성공 결과에는 HTML 조각·읽은 revision·범위 및 미지원 진단을 포함합니다. 다른 View를 내보내지 않으며
  제외 사실을 알립니다. geometry·paint·스타일·modifier·profile·동적 의미는 명시적으로 진단합니다.
  파일 출력 옵션은 없고 JSON의 `result.html`을 사용합니다. Git·프로젝트를 변경하지 않습니다.
- 이는 정적 접근성 투영의 제품 경로이며 XXIV.1.4의 일반 render/export나 Phase XXII 전체 코드 생성
  완료가 아닙니다. 저장 형식·FFI·초기 앱 메뉴·text-source apply에는 영향이 없습니다.

## 고정 Free Frame HTML+CSS 연결 (2026-11-14)

- 저장된 지정 단일 View를 일반 CLI로 브라우저에 출력하는 사용자 요청 범위에서
  `stellar export-web <경로> <문서 id> <View id>`를 추가합니다. 지원 범위와 실패 계약은
  [web-frame-export](../spec/web-frame-export.md)에 둡니다. 기존 접근성 명령은 유지합니다.
- 문서 해석은 ops가 맡고 구조는 기존 접근성 `IrTree`·Web mapper를 재사용합니다. 아직 없는 layout·
  capability를 만들어 전체 `CodegenSources`를 채우지 않습니다. `web::frame::FrameStyle`은 저장된
  core 값(크기·affine·색·테두리·반경)을 요소 하나의 CSS로 쓰는 좁은 출력 입력입니다. 새 범용 IR이나
  property bundle·variant·longhand 스키마를 만들지 않고, codegen의 document 의존도 추가하지 않습니다.
- Frame의 legacy 좌표·border 보정과 root 캔버스 이동 생략은 ADR-0024·속성 모델 5절의 변환을 출력에만
  적용합니다. 저장 migration·ADR-0059 기본 전환을 실행하지 않습니다. 미지원 데이터는 node ID를 포함한
  오류와 HTML 없음으로 반환하며 전체 코드 내보내기 WBS 상태는 변경하지 않습니다.
