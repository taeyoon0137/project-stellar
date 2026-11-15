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

### 5. 기본 글꼴 chain과 글꼴 읽기

- 문서가 정하지 않은 값은 CSS initial의 화면 해석으로 그립니다. 문서에는 아무것도 쓰지 않습니다.
  - font-family가 빈 목록(CSS initial)이거나, 지정한 family의 고정 face를 하나도 쓸 수 없으면 기본 chain으로 그립니다.
  - `font-size: medium`은 16px(design profile이 없을 때의 UA 기본값)입니다.
  - 글자색이 없으면 검정(sRGB, CSS initial `color`의 밝은 화면 해석)입니다.
- 기본 chain(macOS, PostScript 이름 순서): `Helvetica` → `AppleGothic` → `HiraginoSans-W3`.
  - Latin, 한글, 일본어 가나·한자를 덮습니다. glyph가 없는 글자는 chain의 다음 face로 넘깁니다(X.9.5).
  - CoreText로 이름이 정확히 같은 파일만 찾고(다른 글꼴로 대체 금지, X.8.3), 파일은 한 번 읽어 hash·face index·shaping
    face를 같은 bytes에서 구합니다. 64 MiB를 넘는 파일은 건너뜁니다.
  - 찾지 못한 이름은 건너뜁니다. 하나도 없으면 family 없는 Text는 그리지 않고 scene의 skipped로 셉니다.
  - 이 chain은 화면 대체용입니다. 기기마다 파일 hash가 다를 수 있으므로 export·codegen의 재현성 근거로 쓰지 않습니다.
- 글꼴 파일은 document thread에서 읽지 않습니다.
  - runtime이 시작할 때 loader thread 하나를 띄워 기본 chain을 프로세스에서 한 번 읽습니다.
  - 문서 style 표에 등록된 project face는 처음 필요할 때 loader에 요청합니다. 읽는 동안 그 Text는 기본 chain으로
    대체해 그립니다. 다 읽으면 열린 창의 대체 layout과 scene을 버리고 상태를 다시 게시합니다.
  - loader의 결과는 (프로젝트 루트, face, 그 face의 글꼴·asset record)로 구분합니다. record가 바뀌면 다시 검증합니다.
- loader가 face를 다 읽으면 document thread에서 열린 창의 scene을 버리고 다시 평가해 게시한 뒤, scene을 버린 창마다
  요청 없는 `WINDOW_VIEW_CHANGED`(request 0, ADR-0028 "요청 없는 event")를 보냅니다. 셸은 문서 revision이 같아도 이
  event에서 표시 행을 다시 읽으므로, 기본 chain을 다 읽기 전에 열린 창(시작 시 창 복원, 빠른 프로젝트 열기)도 다음
  편집을 기다리지 않고 Text를 그립니다. 같은 창의 요청 없는 event가 아직 남아 있으면 더하지 않습니다.

### 6. 캔버스 Text 편집 session

- 조합 상태의 원본은 runtime 창 session의 조합 하나(`WindowEntry.text`)입니다.
  - 셸의 `NSTextInputClient` adapter는 입력기에 즉시 답하려고 본문·선택·조합 범위의 사본을 둡니다. 조합 본문을
    문서에 반영하거나 직접 그리지 않습니다.
  - 조합 중 본문은 문서와 history에 넣지 않습니다. 같은 창의 canvas scene이 미리보기로 그립니다. scene의 글꼴 cache
    하나만 쓰고, 입력 worker용 두 번째 글꼴 cache와 typing paint 경로는 두지 않습니다.
  - 미리보기는 조합을 시작한 revision에서만 그립니다. scene은 (revision, View, 조합 token·갱신 순번)이 같으면 공유합니다.
- 요청(모두 문서 Unicode scalar 범위, 완료는 event queue의 `WindowViewChanged` 또는 `RequestFailed`)
  - `stellar_text_begin(view, target, revision, new_text, x, y, start, end, out_node, out_request)`: IME 조합을
    시작합니다. out_request가 이후 요청의 token입니다.
    - `new_text = 0`: target은 선택한 기존 Text입니다.
    - `new_text = 1`: target은 자유 배치 부모 Frame이고 (x, y)는 부모 local 원점, 범위는 0..0입니다. 셸은 Text
      도구로 위치를 고를 때 이 조합을 열어 빈 본문의 caret을 보여 줍니다.
    - out_node는 조합 대상입니다. 새 Text는 확정 때 이 ID(UUIDv7, Transform modifier ID도 함께 미리 정함)로 만듭니다.
  - `stellar_text_mark(token, text)`: 조합 본문을 바꿉니다. 조합 안의 선택은 셸이 caret 표시에만 쓰므로 받지 않습니다.
  - `stellar_text_commit(token, text)`: 조합 범위를 확정 본문으로 바꾸고 조합을 끝냅니다.
    - 기존 Text는 Text 범위 편집 하나입니다. 본문이 같으면 history를 늘리지 않고 같은 완료 event를 보냅니다.
      셸은 revision이 그대로인지로 '변경 없음'을 구분합니다.
    - 새 Text는 본문이 있을 때만 Text와 원점 Transform을 만들고 선택합니다. 빈 본문이면 아무것도 만들지 않습니다.
  - `stellar_text_cancel(token)`: 그 조합만 버립니다. 늦은 취소는 더 새 조합에 영향을 주지 않습니다.
  - `stellar_text_replace(view, node, revision, start, end, text)`: 조합 없는 입력(타이핑·삭제·붙여넣기) 한 번입니다.
  - `stellar_text_content(view, node, revision, buf, capacity, out_len)`: 셸 worker 전용 본문 조회입니다. 조합 상태를
    만들거나 바꾸지 않습니다.
- Undo 묶음: 확정·대체는 `GroupingKey::TextInput`으로 기록합니다. 삽입은 caret이 범위 시작에서 본문 끝으로, 삭제는
  범위 끝에서 시작으로 움직인다고 보고, caret이 이어지면 같은 항목에 합칩니다. 새 Text의 생성도 같은 group의 첫
  항목이라 Undo 한 번이 생성과 이어진 입력을 함께 되돌립니다.
- 편집 중 ⌘Z·⇧⌘Z는 adapter가 받아 조합을 먼저 확정하고 문서 history로 보냅니다. 끝나면 본문을 다시 읽습니다.
- 포커스를 잃으면 조합을 먼저 확정합니다.
- 편집 가능 여부는 문서 값(표시·잠금·활성 View 소속·선택)으로만 정합니다. 프로젝트 신뢰는 보지 않습니다(D1).
- 새 Text를 만들 때 문서의 0번 문단 style이 해석되지 않으면(style 표가 없는 문서) 조합 시작을 거부합니다.
- 삭제: `stellar_window_insert_text`(셸이 첫 입력을 따로 삽입하던 경로), `stellar_text_input_paint`와
  `StellarTextPaintSink`, `stellar_text_preview`, runtime `TextPreviewCache`·`canvas_text_preview_input`,
  FFI 창별 조합 slot. 조합 시작 직후 확정을 부르던 두 번 왕복과, 본문을 읽으려고 조합을 열던 경로도 없어집니다.

## Alternatives

- Document Font UI를 유지하고 face index 입력만 숨기기: 사용자가 정의하지 않은 UI이고, 라이선스 확인과 글꼴 asset
  위치(D2)가 정해지지 않았으므로 미선택.
- `SetDocumentTextStyles`를 다음 편집 UI를 위해 남기기: 호출자가 없는 command kind는 registry와 journal 형식만
  늘립니다. 미선택.

## Consequences

- 같은 Text의 연속 입력은 Undo 한 번입니다. 새 Text를 입력 없이 나오면 문서와 history가 바뀌지 않습니다.
- 조합 갱신마다 document thread가 해당 창의 scene을 다시 만듭니다. 다른 Text의 layout은 저장 cache를 재사용합니다.

- 앱에서 문서 글꼴을 고르는 경로가 없어집니다. 문서 style 표에 이미 등록된 project 글꼴은 계속 표시합니다.
- 문단·문자 style의 서로 다른 필드 편집이 Git 병합에서 충돌하지 않습니다.

## Compatibility

- FFI: 같은 저장소의 셸 외에 소비자가 없습니다. 셸은 재생성한 헤더로 함께 바뀝니다.
  - 제거: `stellar_window_set_document_font`, `stellar_window_text_paint`, `stellar_window_insert_text`,
    `stellar_text_input_paint`, `stellar_text_preview`.
  - 시그니처 변경: `stellar_text_begin`(새 Text 대상·out_node·요청 ID), `stellar_text_mark`(조합 안 선택 제거, 요청 ID),
    `stellar_text_cancel`(요청 ID).
  - 추가: `stellar_text_replace`, `stellar_text_content`.
- command registry: 0x0051을 제거합니다. Codex 작업 중에만 쓰였고 배포된 journal이 없습니다.
- 파일 포맷: STTY v1 bytes는 바꾸지 않습니다. axis가 있는 고정 face는 이제 거부합니다. Codex 작업 중에만
  만들 수 있었고 앱·CLI에는 그런 값을 만드는 경로가 없습니다.
- binding 안전값: Typography 안전값이 빈 payload에서 CSS initial record로 바뀝니다.

## Performance

- 문서 thread에서 글꼴 파일을 읽지 않습니다. 기본 chain(약 25 MB)은 프로세스에서 한 번 읽어 메모리에 둡니다.

## Security

- 기본 chain은 CoreText가 알려 준 설치 글꼴 경로만 읽고 쓰지 않습니다. 파일 크기 상한을 둡니다.

- 사용자가 고른 임의 경로의 파일을 프로젝트로 복사하는 경로가 없어집니다.

## Migration

- 데이터 마이그레이션은 없습니다. 셸은 재생성한 `stellar_ffi.h`를 사용합니다.
