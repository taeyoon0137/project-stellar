# ADR-0067: 캔버스 Text 입력·표시 경계와 기본 글꼴

## Status

Accepted (2026-11-15, Codex 작업 검토 묶음 B5)

## Context

- Codex가 이어서 만든 캔버스 Text 경로(U106·U109·U110·U111·U117·U122·U123·U126·U129·U130)는
  ADR 없이 `crates/ffi`와 `crates/core-types` 공개 표면을 넓혔습니다.
- 그중 일부는 사용자가 정의하지 않은 UI이거나 소비자가 없는 ABI입니다.
  - File ▸ Document Font…와 더 보기 메뉴 항목은 face index를 사용자 입력으로 노출하고, 라이선스를 확인하지 않은
    글꼴 파일을 프로젝트에 복사했습니다.
  - 문서 thread에서 사용자가 고른 파일을 동기로 읽었습니다.
- 검토 계획 0절(B5)은 Text 도구가 실제로 동작하는 데 필요한 최소만 남기고, 사용자 미정의 Document Font UI와
  메뉴는 삭제하도록 정했습니다.
- 이 ADR은 B5 묶음이 보호 경로(`crates/ffi`, `crates/core-types`)를 바꾸는 결정과 제거 기록을 한곳에 모읍니다.

## Decision

### 1. Document Font 경로 제거

- 다음을 삭제합니다.
  - 셸: File ▸ Document Font… 항목, 더 보기 메뉴의 같은 항목, 파일 선택 sheet(`ShellDocumentFont.swift`)
  - 셸 검사: `ShellDocumentFontCheck`, `--document-font-check`, 이 UI로 글꼴을 등록하던 `ShellCanvasTextInputCheck`와
    `--canvas-ime-check`
  - FFI: `stellar_window_set_document_font`
  - runtime: `app/document_font.rs`(`AppRuntime::set_document_font`)와 `importing_font` 플래그
  - project: `fonts::document::import_default_font`, `DocumentFontSelection`, `ImportedDocumentFont`
  - commands: `CommandKind::SetDocumentTextStyles`(0x0051)
    - 위 runtime 경로 외에 호출자가 없었습니다.
    - 문서 style 표 전체를 한 번에 바꾸는 command는 longhand 단위 편집(ADR-0024)과 맞지 않습니다.
    - 실제 Text style 편집 경로가 생기면 longhand command로 다시 설계합니다.
- 유지합니다.
  - `fonts::document::load_document_face`와 `verify_document_font_assets`: 이미 style 표에 등록된 project 글꼴을
    다시 엽니다.
  - `DocumentTextStyles.fonts` 형식: 원본 위치는 D2 결정(텍스트 asset catalog 참조)에 따라 텍스트 원본 ADR이 정합니다.
- ShellIntegrationCheck는 U130 이전처럼 `performFrameAction` 메뉴 항목 하나(Show in Finder)만 확인합니다.

### 2. 저장된 Text 그리기 ABI

- 셸은 게시된 canvas scene의 Text 행(`StellarCanvasFrame.flags` bit 4)을 `StellarCanvasTextQuery`(View, node,
  revision, 행 번호)로 조회합니다. 네 값 중 하나라도 현재 게시 상태와 다르면 `STELLAR_NO_ERROR`와 빈 결과를 돌려줍니다.
- `stellar_window_text_parts`: 중복 없는 glyph outline 정의(op 1–4, 정의 끝 op 6), 줄 hit 사각형(op 5),
  배치(op 7, `index`가 정의 번호, `points[0..2]`가 pen 위치)를 한 번에 씁니다. `paint_token`은 같은 layout
  결과를 다시 쓸 때만 같은 값입니다. 셸은 token이 같으면 이전 path를 재사용합니다.
- `stellar_window_text_hit`: Text local 좌표를 문서 전체 Unicode scalar offset과 affinity로 바꿉니다.
- `stellar_window_text_rects`: 같은 layout의 caret(`start == end`) 또는 선택 사각형을 local 좌표로 씁니다.
- `paint_token`은 셸 cache 식별자이므로 `DocumentText`의 같음 비교에서 뺍니다. 내용이 같으면 token이 달라도 같은
  값입니다.
- 소비자가 없던 `stellar_window_text_paint`(glyph마다 outline을 반복해 쓰는 형식)는 삭제합니다.

### 3. Typography 기본값과 고정 face(core-types)

- `TypographyValue::default()`는 빈 bytes가 아니라 CSS initial record입니다. 이전 기본값은 해석할 수 없어,
  binding 안전값과 새 literal이 표시할 수 없는 값을 만들었습니다.
  - binding 안전값 golden(`fallback_golden_corpus_v1.golden`)의 Typography 행이 이 값으로 바뀝니다.
- Typography family의 고정 face에는 variation axis를 넣지 않습니다(ADR-0024에 규칙을 기록합니다).
- `FontFaceRef`·`FontSource`·`FontFileHash`·`OpenTypeTag`의 형식 테스트는 정의가 있는 core-types로 옮깁니다.
  `stellar-text`에는 다른 테스트가 쓰는 값 helper만 남깁니다.

### 4. 문서 Text style 표의 병합과 현재 저장 상태

- 병합(결정): semantic snapshot은 문단·문자 style entry를 필드 단위로 투영합니다.
  - 경로는 `[TEXT_PARAGRAPH_STYLE | TEXT_RUN_STYLE, 필드]` + 원래 u32 index입니다. entry 존재 표식은 필드 없는
    경로의 `Bool(true)`입니다.
  - 필드 key(snapshot 전용 대역, 저장하지 않음): typography `0xE10A`, alignment `0xE10B`, wrapping `0xE10C`,
    line-height `0xE10D`, fill `0xE10E`, letter-spacing `0xE10F`, features `0xE110`, variations `0xE111`.
  - 같은 entry의 다른 필드를 두 branch가 바꾸면 충돌 없이 합칩니다. 같은 필드를 다르게 바꾸면 충돌입니다.
  - 이전에는 entry 전체를 bytes 하나로 투영해, 다른 필드를 고쳐도 충돌했습니다.
- 삭제(결정): `ProjectDocumentIndex`의 `STDI` v2 encode·decode 분기는 쓰는 곳이 없어 삭제합니다. index는 문서
  이름·페이지·View만 담고, Text style 표는 각 문서 정의가 저장합니다.
- 현재 저장 상태(Proposed, 확정 아님): 아래는 Codex가 Accepted ADR-0024에 결정처럼 적었던 현행 구현입니다.
  최종 위치와 형식은 D2에 따라 텍스트 원본 ADR이 정하며, 그때 이 항목을 대체합니다.
  - style 표가 있는 문서는 DocumentManifest pointer schema 2를 쓰고, 문서 정의 뒤에 `STTS` v1 표를 둡니다.
  - 표에는 기본 typography·글자색, StableId별 typography resource, hash별 project 글꼴과 asset record, 원래
    u32 index별 문단·문자 style이 있습니다.
  - 기존 v1 generation은 빈 style 표로 읽습니다.

## Alternatives

- Document Font UI를 유지하고 face index 입력만 숨기기: 사용자가 정의하지 않은 UI이고, 라이선스 확인과 글꼴 asset
  위치(D2)가 정해지지 않았으므로 미선택.
- `SetDocumentTextStyles`를 다음 편집 UI를 위해 남기기: 호출자가 없는 command kind는 registry와 journal 형식만
  늘립니다. 미선택.

## Consequences

- 앱에서 문서 글꼴을 고르는 경로가 없어집니다. 문서 style 표에 이미 등록된 project 글꼴은 계속 표시합니다.
- 문단·문자 style의 서로 다른 필드 편집이 Git 병합에서 충돌하지 않습니다.

## Compatibility

- FFI: `stellar_window_set_document_font`를 제거합니다. 같은 저장소의 셸 외에 소비자가 없습니다.
- command registry: 0x0051을 제거합니다. Codex 작업 중에만 쓰였고 배포된 journal이 없습니다.
- 파일 포맷: STTY v1 bytes는 바꾸지 않습니다. axis가 있는 고정 face는 이제 거부합니다. Codex 작업 중에만
  만들 수 있었고 앱·CLI에는 그런 값을 만드는 경로가 없습니다.
- binding 안전값: Typography 안전값이 빈 payload에서 CSS initial record로 바뀝니다.

## Performance

- 문서 thread에서 사용자 글꼴 파일을 동기로 읽는 경로가 없어집니다.

## Security

- 사용자가 고른 임의 경로의 파일을 프로젝트로 복사하는 경로가 없어집니다.

## Migration

- 데이터 마이그레이션은 없습니다. 셸은 재생성한 `stellar_ffi.h`를 사용합니다.
