# Workspace crate 지도

이 문서는 루트 `Cargo.toml`의 workspace member 32개를 crate마다 한 문단으로 설명합니다. 커밋
`939d142`의 코드에서 확인한 내용만 적었습니다. 근거는 각 crate의 `Cargo.toml`, `src/lib.rs` module
문서와 module 선언, 그리고 의존 정책 `tools/xtask/src/crate_graph.rs`입니다. 문서와 코드가 다르면
코드가 기준입니다.

## 목차

- [읽는 법](#읽는-법)
- [의존 요약 표](#의존-요약-표)
- [공통 기반](#공통-기반)
- [문서 모델과 편집](#문서-모델과-편집)
- [레이아웃, 텍스트, 장면, 렌더링](#레이아웃-텍스트-장면-렌더링)
- [저장, 프로젝트, Git](#저장-프로젝트-git)
- [앱 runtime과 셸 경계](#앱-runtime과-셸-경계)
- [에이전트 도구 계층](#에이전트-도구-계층)
- [Target Profile과 접근성](#target-profile과-접근성)
- [개발 도구 crate](#개발-도구-crate)
- [관련 문서](#관련-문서)

## 읽는 법

- **이름**: package 이름은 모두 `stellar-` 접두어를 쓰고(예외: `xtask`), 이 문서의 표와 의존 목록은
  접두어를 뺀 짧은 이름으로 적습니다. Rust 코드에서는 `stellar_core_types`처럼 밑줄 이름으로 씁니다.
- **허용 직접 의존**: `crate_graph.rs`의 `allowed_edges()` 목록입니다. `cargo xtask validate-crate-graph`가
  `crates/*`와 `tools/stellar-*` manifest의 모든 의존 표(`[dependencies]`, `[build-dependencies]`,
  `[dev-dependencies]`, `[target.<spec>.*]`)에서 `stellar-` 접두어 의존을 읽어 이 목록과 비교합니다.
  정책에 없는 package는 `unknown-crate`, 목록에 없는 edge는 `forbidden-edge`로 실패합니다.
- **실제 의존**: manifest에 지금 선언된 내부 의존입니다. 허용 목록의 부분집합이며, 허용됐지만 아직
  쓰지 않는 edge가 있습니다.
- **전이 도달 금지**: `tools/xtask/src/crate_graph/reach.rs`의 `FORBIDDEN_REACH`가 ADR-0005 §8 표의
  "전이적으로도 도달 금지" 열을 인코딩합니다. 시작 crate는 dev를 포함한 모든 표에서 출발하고, 그
  뒤로는 normal·build 의존만 따라갑니다. 지금 workspace에 있는 crate 중 규칙이 걸린 것은
  `stellar-cli`, `stellar-local-ipc`, `stellar-mcp`입니다. `stellar-ops`, `stellar-agent-protocol`,
  `stellar-agent-host` 규칙은 그 crate가 추가되는 순간부터 적용됩니다.
- **검사 범위 밖**: `tools/xtask`(디렉터리 이름이 `stellar-`로 시작하지 않음), `benchmarks/fixtures`,
  `tests/integration`은 graph 검사 대상 manifest가 아니고 정책 표에도 없습니다.
- **외부 의존**: 승인 목록은 `config/dependencies.yaml`이고, `cargo xtask validate-dependencies`가
  workspace 모든 member의 외부 직접 의존을 검사합니다(ADR-0020 Decision 5). 승인된 외부 crate는
  manifest에서 `=` exact 버전으로 고정합니다.
- **공통 설정**: `xtask`를 뺀 member는 workspace의 `edition = "2021"`과 `rust-version = "1.96.0"`
  (ADR-0001의 baseline)을 상속합니다. `xtask`는 `edition = "2021"`을 직접 적고 `rust-version`을 두지
  않습니다. 모든 member가 `[lints] workspace = true`로 workspace lint를 적용합니다.

## 의존 요약 표

"dev"는 `[dev-dependencies]`에만 있는 의존이고, `[test-support]`는 그 의존에 켠 feature입니다.

| crate | 위치 | 허용 직접 의존(정책) | 실제 내부 의존 | 외부 직접 의존 |
|---|---|---|---|---|
| core-types | `crates/core-types` | 없음 | 없음 | 없음 |
| diagnostics | `crates/diagnostics` | core-types | core-types | 없음 |
| expressions | `crates/expressions` | core-types | core-types | 없음 |
| document | `crates/document` | core-types, expressions | core-types, expressions | 없음 |
| commands | `crates/commands` | document, core-types | core-types, document. dev: document`[test-support]` | 없음 |
| history | `crates/history` | commands, core-types | commands, core-types. dev: commands`[test-support]` | 없음 |
| bindings | `crates/bindings` | core-types, expressions | core-types, expressions | 없음 |
| layout | `crates/layout` | core-types | core-types | 없음 |
| text | `crates/text` | core-types | core-types | `harfrust =0.15.0`, `unicode-bidi =0.3.18`, `unicode-segmentation =1.13.3`, `unicode-linebreak =0.1.5` |
| scene | `crates/scene` | core-types | core-types | 없음 |
| renderer-core | `crates/renderer-core` | scene | scene | 없음 |
| renderer-metal | `crates/renderer-metal` | renderer-core | renderer-core | 없음 |
| renderer-d3d12 | `crates/renderer-d3d12` | renderer-core | renderer-core | 없음 |
| export | `crates/export` | core-types, scene | core-types, scene | 없음 |
| storage | `crates/storage` | core-types | core-types | 없음 |
| assets | `crates/assets` | storage, core-types | core-types, storage | 없음 |
| dependencies | `crates/dependencies` | core-types | core-types | 없음 |
| git-porcelain | `crates/git-porcelain` | core-types | core-types | 없음 |
| project | `crates/project` | core-types, document, storage, assets, dependencies, git-porcelain, commands, history, text | core-types, document, storage, assets, dependencies, git-porcelain, commands, history, text. dev: document`[test-support]` | `yaml-rust2 =0.11.0` |
| semantic-diff | `crates/semantic-diff` | document, core-types | document, core-types. dev: document`[test-support]` | 없음 |
| semantic-merge | `crates/semantic-merge` | semantic-diff | semantic-diff | 없음 |
| runtime | `crates/runtime` | core-types, document, commands, history, project, storage, assets, renderer-core | core-types, document, project, renderer-core | 없음 |
| ffi | `crates/ffi` | core-types, runtime | core-types, runtime | 없음 |
| terminal | `crates/terminal` | core-types | core-types | 없음 |
| local-ipc | `crates/local-ipc` | core-types | 없음 | 없음 |
| cli | `tools/stellar-cli` | core-types, project, document, storage, diagnostics | project | 없음 |
| mcp | `tools/stellar-mcp` | core-types, diagnostics, local-ipc | local-ipc | 없음 |
| target-profile | `crates/target-profile` | core-types | core-types | 없음 |
| accessibility | `crates/accessibility` | core-types | core-types | 없음 |
| `xtask` | `tools/xtask` | 정책 밖 | storage, core-types | `yaml-rust2 =0.11.0` |
| bench-fixtures | `benchmarks/fixtures` | 정책 밖 | core-types, document, expressions, storage, project, layout, assets | 없음 |
| integration-tests | `tests/integration` | 정책 밖 | dev만: core-types, document`[test-support]`, commands`[test-support]`, history, storage, project, git-porcelain`[test-support]`, semantic-diff, semantic-merge, expressions`[test-support]`, renderer-core`[test-support]`, target-profile | 없음 |

`test-support` feature를 정의한 crate는 document, commands(document의 `test-support`를 함께 켬),
git-porcelain, expressions, renderer-core입니다. 다섯 crate 모두 기본값은 꺼져 있고, manifest 주석은 다른
crate가 `[dev-dependencies]`에서만 켜도록 정합니다. document의 `workspace_production_code_does_not_use_test_support`
테스트가 workspace 모든 manifest의 `[dependencies]`·`[build-dependencies]`·`[features]`가 이 feature를 켜지
않는지 검사합니다.

## 공통 기반

### stellar-core-types

`crates/core-types`는 제품 crate가 공유하는 기반 타입을 둡니다. 주요 module은 안정·런타임
식별자(`ids`), 구조화 오류(`error`), PropertyKey·PropertyPath(`property`), 강한 타입 참조와 미해결
참조(`reference`), 타입드 값과 기하(`value`), `TypedValue` tagged union(`typed_value`), 문자열 값과
정규화 경계(`text`), 색상·색공간(`color`), 그라디언트(`gradient`), 그림자(`shadow`), 자식 순서
키(`order_key`)와 그 merge fixture(`order_key_merge_fixtures`), 단조 시계 추상화(`clock`), revision
newtype(`revision`)입니다. 정책상 허용 직접 의존이 없고 실제로도 의존이 없는 그래프의 맨 아래
crate입니다. 주요 ADR은 식별자·참조·공통 기반 타입의 ADR-0007, 값·단위·기하·그래픽 값의 ADR-0008,
색상 transfer 함수의 ADR-0002, OrderKey 알고리즘의 ADR-0003입니다.

### stellar-diagnostics

`crates/diagnostics`는 구조화 로그와 진단 이벤트 타입을 둡니다. 프로젝트 내용·텍스트·경로 같은 민감
데이터는 `Sensitive`에만 담고, 비민감 `TelemetryEvent`는 컴파일 타임 상수 이름(`&'static str`)과 숫자
코드만 받으므로 민감 값을 넣으면 컴파일에 실패합니다(`compile_fail` doctest로 고정).
`SensitiveDiagnostic`은 비민감 이벤트와 민감 detail을 나눠 담습니다. `memory` module은 CPU·GPU 메모리
예산 provider와 정규화 bucket이며, 운영체제 메모리 provider는 macOS만 지원합니다(ADR-0006). 허용
직접 의존과 실제 의존은 core-types입니다.

## 문서 모델과 편집

### stellar-expressions

`crates/expressions`는 표현식 타입 시스템과 평가기를 둡니다. 타입과 nullable `Optional<T>`(`types`),
type rule(`rules`, `arithmetic`, `logic`), 함수 signature registry(`functions`), 닫힌 연산
집합(`ops`), AST와 구조 검증(`ast`), binary·canonical 직렬화(`codec`), cycle 검출(`cycle`), type
checker(`typecheck`)가 중심입니다. GUI 편집용 block·edge projection(`graph_view`), socket 타입(`sockets`),
편집 command plan(`edit_plan`), 오류의 block·socket 매핑(`gui_diagnostics`)도 있습니다. 평가는 공통
의미(`eval`), bytecode compiler(`bytecode`)와 evaluator(`vm`), AST evaluator(`ast_eval`), 함수
evaluator(`function_eval`), 환경 입력 key registry(`environment`)로 나뉩니다. `test_support` module은
기본값이 꺼진 `test-support` feature나 unit test에서만 빌드되며, renderer-core와 따로 보관하는 Display P3 ↔
sRGB 행렬을 통합 테스트 대조용으로 다시 내보냅니다. 허용 직접 의존과 실제 의존은 core-types입니다. 주요 ADR은
환경 입력 key registry와 `os_version` 인코딩을 정한 ADR-0026입니다.

### stellar-document

`crates/document`는 문서 모델을 소유합니다. 저장 구조는 generational arena(`arena`), 공통 node 필드의
SoA 저장(`node_columns`), 문서 소유자와 읽기 전용 snapshot view(`snapshot`)입니다. 모델은 문서·페이지·View
정의(`definitions`), node 종류(`node_kind`), parent-child 삽입·이동 validator(`tree_ops`), 참조
index(`reference_index`), node 수·깊이 훅(`tree_stats`)으로 구성됩니다. node payload는 `frame`,
`fragment`, `shape`, `path`, `text`, `image`이고, modifier stack(`modifier`), capability
validator(`capabilities`), 컴포넌트 정의·인스턴스·Slot(`component`), Frame control 계약(`control`),
View 변수와 Variant(`view_variant`)가 붙습니다. Frame property metadata registry(`frame_metadata`)와 modifier
property metadata·무효화 class registry(`modifier_metadata`)도 있습니다. 영속 레코드(`records`), 전체 validator(`validate`),
canonical 인코딩과 View subtree 분할(`record_codec`), 런타임 문서 모델(`model`), 열린 문서 집합과
제한(`open_set`, `open_limits`)도 이 crate에 있습니다. `test_support` module은 기본값이 꺼진
`test-support` feature나 이 crate의 unit test에서만 컴파일됩니다. 허용 직접 의존과 실제 의존은
core-types, expressions입니다. expressions 의존은 Property default, 상태 변수 initial, transition
조건 같은 타입이 선언된 표현식 위치에 씁니다(manifest 주석).

### stellar-commands

`crates/commands`는 편집 command 모델입니다. module 문서에 따르면 사용자·시스템·migration·향후 agent의
문서 편집이 모두 이 crate의 같은 command 경로를 지납니다. command 식별자와 출처(`id`), numeric
registry(`kind`), validate·apply·invert 인터페이스를 가진 직렬화 가능한 closed enum(`command`), 적용
context(`edit`), canonical 직렬화(`codec`), 대상 상태(`state`), base revision과 precondition을 담은
요청(`request`), 결과 change set(`change`), 되돌리기 guard(`guard`), 전부 성공하거나 전부 롤백하는
`CommandTransaction`(`transaction`)이 뼈대입니다. 구체 command는 node 구조(`node_ops`),
`SetProperty`(`property`), modifier stack(`modifier_ops`), node 종류 변환(`convert`),
`DuplicateSubtree`(`duplicate`), 창별 선택(`selection`)입니다. `pub use stellar_document as document`로
문서 모델 타입을 다시 내보내므로 history는 document에 직접 의존하지 않고 그 타입을 쓸 수 있습니다.
`test-support` feature는 테스트 생성기(`test_support`)와 fuzz target(`fuzz`, `sequence_fuzz`)을 다른
crate의 테스트에 노출합니다. 허용 직접 의존과 실제 의존은 document, core-types입니다.

### stellar-history

`crates/history`는 창별 undo/redo history입니다. module 문서에 따르면 history는 문서를 직접 바꾸지
않고 commands의 transaction 실행 경로만 쓰며, 현재 실행 세션의 편집 이벤트로서 Git 이력·저장 파일과
분리됩니다. 주요 module은 history 항목(`entry`), 창 하나의 stack(`history`), grouping 정책(`grouping`),
창별 history(`window`)입니다. 로드맵 작업(XXV.1)으로 영속 history journal의 schema(`journal`), 기록
위치와 기록(`journal_store`), 앱 재실행 후 복원과 journal 봉인(`journal_restore`)이 추가돼 있습니다.
허용 직접 의존과 실제 의존은 commands, core-types입니다.

### stellar-bindings

`crates/bindings`는 토큰과 통합 Binding Graph를 둡니다. 토큰 쪽은 정의·이름·타입·TokenSet(`token`),
프로젝트·라이브러리·문서 scope registry(`registry`), dependency graph와 저장 전 cycle
거부(`token_graph`), 값 평가 cache(`token_cache`), 생성·삭제·rename·expression 변경
command(`token_commands`)입니다. binding 쪽은 Value Binding 정의·직렬화·type 검증(`binding`), 토큰·속성·
Component Property·환경 입력을 묶은 dependency graph(`graph`), 영향받는 binding만 위상 순서로
재평가하는 scheduler(`scheduler`), 평가 값 provenance(`provenance`), binding command(`binding_commands`),
환경 입력 reverse index(`environment_index`), Orphaned Binding 검증·fallback·재연결(`orphan`,
`fallback`, `orphan_commands`), 대규모 synthetic graph 생성기(`synthetic`)입니다. 허용 직접 의존과 실제
의존은 core-types, expressions입니다.

## 레이아웃, 텍스트, 장면, 렌더링

### stellar-layout

`crates/layout`은 결정론적 레이아웃 엔진입니다. 입력 tree(`input`), 출력 box(`output`), 좌표·크기
(`geometry`), 입력 속성과 dirty class registry(`property`), layout revision(`revision`), 오류(`error`),
진입점(`engine`)을 중심으로 자유
배치(`free`), Stack 배치(`stack`)와 grow·shrink·basis(`flex`), Grid 배치(`grid`, 내부 `grid_tracks`),
Overlay(`overlay`), Anchor·Constraint(`constraint`), local transform(`transform`)을 구현합니다. 크기
결정(`sizing`), content intrinsic 측정 hook(`intrinsic`), 측정 cache(`measure`), node별 input hash와
output cache(`cache`), rounding 정책과 logical subpixel 단위(`rounding`), worker result
guard(`guard`), 적합성 fixture corpus(`corpus`)도 있습니다. 증분 레이아웃 쪽은 command invalidation을 layout
dirty set으로 바꾸는 `dirty`, 영향 범위만 다시 배치하는 `incremental`, 단일 leaf 변경 benchmark(`bench`)입니다.
문서 원천을 Fragment flatten한 layout tree view(`view`)가 있고, 내부 module로 canonical bytes reader·writer
(`codec`)와 공간 분배(`distribute`)가 있습니다. document에 의존하지 않고 자체 입력 tree를 받습니다. 허용 직접 의존과 실제 의존은 core-types입니다.

### stellar-text

`crates/text`는 결정론적 텍스트 엔진입니다. 폰트 출처·hash·face 참조(`font`), face registry와
dedup(`registry`), 명시적 fallback chain(`fallback`), missing font 진단(`missing_font`), shaping wrapper
(`shaping`), BiDi(`bidi`), script·language·direction itemization(`itemize`), 줄바꿈 기회(`line_break`),
grapheme 단위 font fallback(`font_runs`), 고정소수점 단위(`units`), paragraph pipeline(`paragraph`),
줄 나누기(`line_layout`), 줄 높이·정렬(`text_layout`), caret·선택·hit test(`caret`), cache key(`cache`),
paragraph 단위 증분 레이아웃(`incremental`)으로 구성됩니다. 내부 module로 테스트 빌드 전용 작업량
계수기(`op_count`)와 debug 빌드 정렬 불변식 검사(`tiling`)가 있습니다. `#![forbid(unsafe_code)]`로 unsafe 코드를
금지합니다. 허용 직접 의존과 실제 내부 의존은 core-types이고, 외부 의존은 ADR-0016이 승인한 순수
Rust shaping·Unicode crate(`harfrust`, `unicode-bidi`, `unicode-segmentation`, `unicode-linebreak`)뿐입니다.

### stellar-scene

`crates/scene`은 불변 Scene 모델입니다. module 문서에 따르면 scene은 Document Node와 GPU 객체를 담지
않고, document 계층이 평가한 값을 복사해 담으며, 외부 cache가 해석하는 데이터는 `SceneResourceRef`
식별자로만 가리킵니다. 주요 module은 path geometry(`geometry`), 구조 공유 persistent vector
(`persistent`), node와 resource 참조(`node`), clip·paint stack(`stack`), 불변 snapshot(`snapshot`)과
canonical serialization(`codec`), DocumentRevision에서 SceneRevision으로의 매핑(`revision`), render
thread용 읽기 전용 handle(`handle`), 오래된 snapshot 감지(`freshness`), node 종류별 builder(`builder`),
bounds·공간 인덱스·가시 후보·broad-phase hit test(`bounds`, `spatial`, `visibility`, `hit_test`), node
종류별 narrow-phase hit test(`narrow_phase`), selection handle·modifier control point의 transient hit-test
layer(`transient`)입니다.
renderer-core가 scene에만 직접 의존하므로 `ColorSpace`, `FiniteF32`, `NonFiniteError`, `Size2`, `Vec2`
같은 core-types 기본 타입을 다시 내보냅니다. 허용 직접 의존과 실제 의존은 core-types입니다.

### stellar-renderer-core

`crates/renderer-core`는 플랫폼 공통 렌더러 추상화이고 플랫폼 backend를 역참조하지 않습니다.
backend 독립 Render Graph(`render_graph`), GPU backend interface(`backend`), generational GPU
handle(`gpu_handle`), upload queue와 frame-local allocator(`frame_resources`), present queue 깊이 1 이하
frame scheduler(`frame_scheduler`)가 중심입니다. scene primitive를 pass로 나누는 compiler
(`pass_compiler`), clip 계획(`clip_plan`), offscreen 경계 분석(`offscreen`), batching(`batching`),
compile trace(`graph_trace`), tessellation interface(`tessellation`), stroke tessellation(`stroke`),
tessellation worker pool과 stale 결과 폐기(`tessellation_worker`), glyph atlas interface(`glyph`), content 기반
texture cache key(`texture_cache`), mipmap 생성과 upload task(`mipmap`), worker 전용 이미지 decoder
interface(`image_decode`), asset placeholder(`placeholder`), thread 역할 표시(`thread_role`)도 있습니다.
benchmark용 present queue 깊이·dropped stale frame 지표(`present_metrics`)와 surface 출력 색공간 협상·gamut
경고·HDR 미지원 상태(`output_color`)도 이 crate에 있습니다. 내부 module `digest`는 결정론적 digest입니다.
`test_support` module은 기본값이 꺼진 `test-support` feature나 unit test에서만 빌드되며, expressions와 따로
보관하는 Display P3 ↔ sRGB 행렬을 통합 테스트 대조용으로 다시 내보냅니다. 허용 직접 의존과 실제 의존은 scene
하나입니다. runtime이 이 crate의 thread 역할 표시를 runtime thread에 bind합니다(ADR-0013).

### stellar-renderer-metal

`crates/renderer-metal`은 macOS Metal backend입니다. `#![cfg(target_os = "macos")]`로 게이트되고,
`MetalBackend`가 renderer-core의 `RendererBackend`를 Metal로 구현합니다. 공통 계층은 handle·descriptor·graph만
넘기고 Metal 객체는 이 crate 밖으로 나가지 않습니다. 외부 binding crate 없이 Objective-C runtime(`objc_msgSend`)과
Metal·QuartzCore·CoreGraphics C 함수를 직접 선언합니다(`objc`, `sys`). module은 모두 crate 내부입니다. format
대응(`format`), `MTLDevice`·command queue(`device`), CAMetalLayer 연결·drawable·resize·frame skip(`surface`),
texture 생성(`texture`), buffer와 upload(`buffer`), shader 소스·ABI(`shaders`), shader library·pipeline
cache(`pipeline`), draw command 기록(`commands`), transient texture pool(`transient`), RenderGraph pass
encoding(`encode`), frame CPU timestamp trace(`trace`), GPU timestamp query(`timestamps`), 오류 event·GPU 메모리
예산(`events`), backend 구현(`backend`)이 있습니다. 공개 API는 `MetalBackend`(`with_layer`로 셸의 CAMetalLayer를
받거나 `headless`로 external texture에만 그림), `MemoryUsage`, `MetalEvent`, `ShaderError`, `LayerOptions`,
`SurfaceStats`, `MetalFrameTrace`, timing 상수, backend 식별자 `BACKEND = "metal"`입니다. 테스트(XI.8.1\~XI.8.7)는
실제 Metal device에서 돌고, device가 없으면 실패하며 `STELLAR_ALLOW_MISSING_METAL=1`일 때만 건너뜁니다. 허용 직접
의존과 실제 의존은 renderer-core이고 외부 의존은 없습니다. 대상 플랫폼은 ADR-0006에 따라 macOS입니다.

### stellar-renderer-d3d12

`crates/renderer-d3d12`는 Windows Direct3D 12 backend 골격입니다. `#![cfg(target_os = "windows")]`로
게이트되고, 내용은 `BACKEND = "d3d12"` 상수 하나입니다. ADR-0006은 Windows 지원을 보류하면서 이
골격을 삭제하지 않고 빌드·테스트가 통과하는 상태로 유지하기로 했습니다. 허용 직접 의존과 실제 의존은 renderer-core입니다.

### stellar-export

`crates/export`는 PNG·SVG 내보내기(Phase XX)용 로드맵 crate입니다. 문서 모델과 GPU 객체를 직접 다루지
않고 불변 Scene Snapshot을 입력으로 받습니다. 현재 module은 내보내기 요청과 field schema(`request`),
scene snapshot·resource revision 입력 고정(`capture`), SVG 벡터 변환(`svg`)입니다. 허용 직접 의존과
실제 의존은 core-types, scene입니다. 주요 ADR은 로드맵 crate를 추가한 ADR-0019입니다.

## 저장, 프로젝트, Git

### stellar-storage

`crates/storage`는 생성(generation) 기반 저장소입니다. canonical binary 인코딩(`canonical`), content
hash(`hash`), chunk 저장소(`chunk`), atomic 파일 쓰기(`atomic_file`), generation 저장(`generation`),
저장 트랜잭션(`save`), garbage collection(`gc`), 스키마 버전과 마이그레이션(`migration`)을 둡니다.
내부 `fs_guard`는 저장소 파일을 정규 파일·크기 상한으로만 읽기, symlink 저장소 디렉터리 거부, 파일
identity 비교를 맡습니다. 허용 직접 의존과 실제 의존은 core-types입니다. 주요 ADR은 content hash
알고리즘의 ADR-0004, 저장 포맷 기준선의 ADR-0010, 저장 트랜잭션·GC 보존·마이그레이션의 ADR-0011,
chunk 분할 임계치의 ADR-0012, decoder·파일 접근 경화의 ADR-0015입니다.

### stellar-assets

`crates/assets`는 콘텐츠 주소 에셋 저장입니다. 식별자(`id`), media type(`media_type`), record와
canonical 인코딩(`record`), 기기별 비밀 key HMAC으로 만든 외부 원본 출처 지문(`origin`), streaming
content hash(`hash_stream`), content 파일 경로(`path`), 저장소와 외부 파일 import·dedup(`store`),
catalog(`catalog`), temp·fsync·atomic rename 쓰기(`atomic_write`), 원본 재import(`update_source`),
프로젝트 내부 원본 경로 경계(`project_source`), 에셋 content GC(`gc`), import 정책(`policy`)으로
구성됩니다. 내부 `fs_guard`는 symlink를 따라가지 않는 열기와 파일 identity 비교를 맡습니다. 허용 직접
의존과 실제 의존은 storage, core-types이고, content hash는 ADR-0004를 따릅니다.

### stellar-dependencies

`crates/dependencies`는 Git 기반 외부 Component·Token library 의존성 모델(Phase XV)입니다.
`stellar.yaml`·`stellar.lock.yaml` 텍스트, 시스템 Git 실행, 프로젝트 probe·저장소 검증은 project
crate가 이 crate의 trait과 형식 중립 트리(`node`)를 구현·연결합니다. 선언 모델(`declaration`), version
선택자와 lock pin(`selection`), 선언 충돌 검증(`validation`), 민감값 분류와 redaction
(`sensitivity`), resolver 그래프와 전이 resolver(`graph`, `resolver`), 시스템 Git 실행 경계와 ref
해석(`git`, `git_refs`), library cache(`content_hash`, `cache_root`, `cache_index`, `mirror`,
`cache_entry`, `cache_lock`, `entry_validation`, `cache_privacy`), lock 변환과 무결성(`lock`,
`lock_integrity`), local path override(`local_override`), 다중 버전 경고(`duplicate_report`)가 주요
module입니다. 사용 프로젝트 쪽 library 모델(Phase XV.5·XV.7)은 읽기 전용 library snapshot과 resource
key(`library_snapshot`), 외부 component·token 참조(`external_ref`), library resource 읽기 전용 정책
(`read_only`), 외부 참조 scope 해석(`ref_resolution`), lock 기준 library 로드와 부분 열기(`library_load`),
update 후보 준비(`library_update`), library resource Semantic Diff(`resource_diff`)입니다. 허용 직접 의존과
실제 의존은 core-types입니다. document·commands crate는 이 crate에 의존할 수 없으므로, `read_only` 판정을
command 경로에 연결하는 위치는 project crate(또는 project를 거치는 runtime)이며 아직 연결은 없습니다. 주요
ADR은 의존성 entry와 lock entry 형식을 단일화한 ADR-0021, library cache entry 범위와 content hash의 ADR-0022,
library snapshot resource key의 ADR-0027입니다.

### stellar-git-porcelain

`crates/git-porcelain`은 System Git porcelain입니다. document를 직접 수정하지 않고, Git 상태는
`SnapshotSource` 원천별 snapshot(파일 목록, blob, materialize된 임시 프로젝트 디렉터리)으로만
제공합니다. Git 실행 기반은 executable 탐색(`discovery`), 버전 probe(`version`), 보안 실행기(`runner`),
취소와 프로세스 tree 종료(`cancel`), 결과·출력 디코딩·trace redaction(`command`, `decode`, `trace`)
입니다. 설치 쪽은 상태 머신(`install_state`), macOS·Windows 설치 어댑터(`install_macos`,
`install_windows`), 재탐지(`reprobe`)입니다. 저장소 연산은 `repo`, `ops`, `status`, `repo_status`,
`classify`, 파일 단위 영역별 status(`file_status`), raw status와 semantic status 실패 분리(`project_status`),
stage candidate 임시 tree writer(`candidate_tree`), `commit`, `identity`, `object`, `remote`, `refname`, 신뢰
gate(`trust`), 위치
판별(`location`), capability probe(`capability`), conflict 수집과 virtual snapshot(`conflict`,
`conflict_snapshot`), snapshot provider(`snapshot`), Git LFS 감지(`lfs`)이고, 로드맵 기능으로 고급 Git
core API(`advanced`)와 Merge Composer(`composer`)가 있습니다. `fixture` module은 `test-support` feature
뒤의 Git 통합 fixture harness이고, `corpus`(unix 테스트 빌드 전용)는 특수 경로·mode·LFS pointer·손상
프로젝트 Git corpus입니다. 허용 직접 의존과 실제 의존은 core-types입니다. Windows 설치
어댑터는 ADR-0006에 따라 유지되는 Windows 지향 코드입니다.

### stellar-project

`crates/project`는 프로젝트 수명주기를 담당하고 다른 core crate를 가장 많이 묶습니다. 제한
YAML(`yaml`), `stellar.yaml` 매니페스트(`manifest`), 의존성 형식 중립 트리 변환(`manifest_node`),
exact commit의 library 매니페스트 읽기(`manifest_source`), `stellar.lock.yaml` 모델과 기록
transaction(`lock`, `lockfile_write`)이 파일 형식을 맡습니다. 폴더 이름·root 검증·식별(`folder_name`,
`root`, `identity`), 생성과 scaffold(`create`, `fs_transaction`, `scaffold`), 열기 probe(`probe`),
신뢰 상태·저장소·capability matrix(`trust`, `trust_store`, `capability`), clone 요청(`clone`), 복구
스냅샷(`recovery`, `recovery_payload`, `recovery_probe`, `recovery_shutdown`), 경로 resolver
(`path_resolver`), 문서와 chunk model 입출력(`document_io`), 파일 감시와 외부 변경
(`watcher`, `external_change`), 창 문서 세션과 clean 창의 외부 generation 자동 reload(`window_session`), 지연
자동 저장(`autosave`), 저장된 프로젝트 loader(`saved_project`),
비공개 임시 디렉터리(`private_temp`), conflict virtual project 열기(`conflict_open`), 폰트(`fonts`)가
수명주기를 맡습니다. 허용 직접 의존은 core-types, document, storage, assets, dependencies,
git-porcelain, commands, history, text이고, 아홉 개를 모두 씁니다(commands·history는 autosave dirty
판정과 창 session, XIII.1.2·XIII.3.1). 외부 의존은 `yaml-rust2 =0.11.0`입니다. `stellar` 명령의
`inspect`가 이 crate의 `probe`와 `root`를 씁니다. 주요 ADR은 ADR-0021입니다.

### stellar-semantic-diff

`crates/semantic-diff`는 의미 기반 diff(Phase XIV.3)입니다. Git HEAD·Index·Working Tree에서 읽은 영속
레코드를 `SemanticSnapshot`(StableId index)으로 펼치고, 두 snapshot의 차이를 `StableId + PropertyPath`
단위로 계산합니다. module은 속성 값과 key(`value`, `keys`), snapshot 모델과 구성(`snapshot`,
`build`), 변경 모델(`change`), diff(`diff`), canonical 인코딩(`codec`), diff 적용(`apply`)입니다. Git 화면용
module로 HEAD·Index·WorkingTree semantic status 분류(`status`), 선택 변경 Staging 계획(`stage`), 변경
dependency graph(`dependency`), snapshot 무결성 validator(`validate`), snapshot을 영속 문서 레코드로 되돌리는
역투영(`records`)이 있습니다.
semantic-merge가 이 crate에만 의존하므로 `pub use stellar_core_types as core_types`로 공용 값 타입을
다시 내보냅니다. 허용 직접 의존과 실제 의존은 document, core-types입니다.

### stellar-semantic-merge

`crates/semantic-merge`는 3-way Semantic Merge(Phase XIV.10)입니다. base·ours·theirs
`SemanticSnapshot`을 `StableId + PropertyPath` 단위로 비교해 겹치지 않는 변경은 자동 병합하고, 겹치는
변경은 세 상태를 모두 담은 conflict로 남깁니다. module은 입력·결과·conflict 모델(`model`)과 merge
엔진(`merge`)입니다. 허용 직접 의존과 실제 의존은 semantic-diff 하나입니다. 페이지·View·Slot 정의 목록은 3-way 목록
병합으로 합치고 candidate에 남은 소속 객체와 맞춥니다(SM-1 수정, XIV.10.6 작업 보고서).

## 앱 runtime과 셸 경계

### stellar-runtime

`crates/runtime`은 앱 프로세스 runtime입니다. 아키텍처 §9.2의 앱 내부 thread 모델(Native UI,
Document, Render, Worker Pool)을 코어 쪽에서 구성합니다. module은 DocumentThread event
loop(`document_thread`), UI에서 DocumentThread로 가는 bounded command queue와 입력 우선순위
(`command_queue`), RenderThread와 최신 scene snapshot의 lock-free handoff(`render_thread`), 셸 경계용
앱 runtime(프로젝트·문서 session, 완료 event, 상태 snapshot, `app`)입니다. 외부 의존 없이 표준
라이브러리 thread·동기화만 씁니다. 네이티브 셸은 `stellar-ffi`를 거쳐 이 crate에 도달하고, 이 crate는
ffi를 역참조하지 않습니다. 허용 직접 의존은 core-types, document, commands, history, project, storage,
assets, renderer-core이고, 실제로는 core-types, document, project, renderer-core를 씁니다. 주요 ADR은
crate 경계와 thread 모델의 ADR-0013, 셸↔코어 FFI 표면의 ADR-0014입니다. ADR-0005 §8은 이후
agent-host, agent-protocol, ops와 에이전트 편집 구현 crate를 허용 의존에 더하는 작업을 XXIX.3.1로
둡니다.

### stellar-ffi

`crates/ffi`는 네이티브 셸과 코어 사이의 C ABI 경계입니다. runtime handle을 opaque pointer가 아니라
정수형 registry handle(0은 무효)로 노출하고, 모든 작업 함수는 `StellarStatus`를 반환하며, 오류 세부는
전역이 아니라 handle별 `stellar_last_error`로 읽습니다. 모든 경계는 `ffi_guard`로 감싸 panic이
경계를 넘지 않고 `InternalPanic`으로 바뀝니다. 프로젝트·문서 요청(`stellar_project_*`,
`stellar_document_*`)은 `stellar-runtime`의 `AppRuntime`으로 넘기고, 완료는 pull 방식 event와 상태
snapshot으로만 셸에 돌아옵니다. 입력 쪽 module은 공통 입력 이벤트(`input`), macOS `NSEvent`
adapter(`input_macos`), coalescing 입력 대기열(`input_queue`, 용량 `INPUT_QUEUE_CAPACITY` = 4096)입니다.
`crate-type = ["lib", "staticlib"]`로 빌드하고, C 헤더 `crates/ffi/include/stellar_ffi.h`는
`cargo xtask generate-ffi-header`가 생성합니다. 허용 직접 의존과 실제 의존은 core-types, runtime이며
문서·프로젝트 작업에는 runtime을 통해서만 도달합니다. 주요 ADR은 C ABI 기준선의 ADR-0009, ADR-0013,
ADR-0014입니다.

### stellar-terminal

`crates/terminal`은 내장 터미널 세션 코어(Phase XVIII)입니다. 운영체제 셸을 호스팅하는 세션의 코어만
두고 UI는 앱 셸이 맡으며, Git이나 AI 로직을 재구현하지 않습니다. module은 세션 식별자(`id`), argv
배열 명령·환경·cwd profile(`profile`), 환경 변수 허용 목록(`env`), 작업 디렉터리 선택·검증(`cwd`),
child process 상태(`state`), 창별 다중 세션 registry(`registry`), 신뢰된 프로젝트에서만 시작하게 하는
capability token(`capability`), 실행 요청과 프로세스 경계(`launch`), 행·열 크기(`size`), 셸
설정(`shell`), header metadata(`header`), macOS 전용 POSIX PTY 실행기(`pty`)입니다. 세션·출력·프로세스
상태는 메모리에만 두고 저장·복구 snapshot에 넣지 않으며, 저장소·프로젝트·문서·직렬화 crate에 의존하지
않는다는 점을 테스트로 고정합니다. 허용 직접 의존과 실제 의존은 core-types이고, 주요 ADR은 ADR-0019
입니다.

## 에이전트 도구 계층

### stellar-local-ipc

`crates/local-ipc`는 `stellar-mcp` 브리지와 실행 중 앱 runtime이 함께 쓰는 로컬 제어 채널입니다.
network port를 쓰지 않고 사용자 전용 디렉터리의 Unix domain socket만 씁니다. module은 OS 사용자
식별과 사용자별 디렉터리(`os`), 실행 중 앱 endpoint registry(`registry`), session nonce(`nonce`), 같은
OS 사용자 확인과 nonce handshake(`handshake`), 연결 상대 자격 증명(`peer`), 코드 서명 신원
검사(`code_identity`), 인증 기준(`auth`), 앱 측 endpoint와 IPC I/O 스레드(`listener`), 앱 측 연결
수명(`session`)이며, 모두 `cfg(target_os = "macos")`로 게이트되어 다른 OS에서는 빈 crate로 빌드됩니다.
registry와 handshake 구현은 처음에 `tools/stellar-mcp`에 있었고 앱 runtime이 같은 코드를 쓰도록 이
crate로 옮겼습니다. 허용 직접 의존은 core-types이지만 실제 manifest에는 내부·외부 의존이 없습니다.
전이 도달 금지 대상은 document, commands, ops입니다. 주요 ADR은 ADR-0005 §5·§8과 ADR-0006입니다.

### stellar-cli (`stellar` 명령)

`tools/stellar-cli`는 ADR-0005의 L1 headless 명령줄 도구입니다. package·library 이름은 `stellar-cli`,
실행 파일 이름은 `stellar`이고, 앱 UI·GUI·네트워크 없이 동작하며 AI·모델·agent 코드나 의존을 두지
않습니다. module은 명령 registry·도움말·인자 해석(`cli`), 명령 구현(`commands`), 종료 코드(`exit`),
외부 crate 없는 엄격한 JSON reader·writer(`json`), 결과 envelope와 stdout·stderr 역할(`output`)입니다.
현재 명령은 `help`, `version`, `inspect`이고 사용법은 [stellar 명령 사용 안내](stellar-cli.md)에
정리했습니다. `json` module은 `tools/stellar-mcp/src/json.rs`와 module 문서만 다른 의도적 사본이고
`json_copy_sync` 테스트가 본문 일치를 검사합니다. 허용 직접 의존은 core-types, project, document,
storage, diagnostics이고 실제로는 project 하나만 씁니다. 전이 도달 금지 대상은 코드 기준으로
`stellar-local-ipc`, `stellar-agent-protocol`, `stellar-mcp`, `stellar-agent-host`입니다(ADR-0005 §8
표는 이 열에 mcp-core를 적고 있고, 코드 목록에는 `stellar-mcp-core` 대신 `stellar-mcp`가 있습니다).
주요 ADR은 ADR-0005, 외부 JSON crate 승인의 ADR-0020, `inspect` 출력의 필드 접근을 바꾼 ADR-0021입니다.

### stellar-mcp

`tools/stellar-mcp`는 ADR-0005의 L2 MCP stdio 브리지입니다. AI client(외부 agent CLI)와 stdin·stdout
JSON-RPC 2.0으로 통신하고, network port를 열지 않으며, stdout은 protocol 메시지 전용이고 로그는 모두
stderr(`log`)에 씁니다. 인자 없이 실행하면 MCP 서버를 돌리고, `--attach [--instance <id>]`는 세션 전에
실행 중 앱을 골라 인증·handshake하며(macOS 전용), `--connection-info`는 client 등록 정보 JSON 한 줄만
쓰고 끝납니다. module은 JSON(`json`), 등록 정보(`connection_info`), XXIV.1.2 표와 같은 종료
코드(`exit`), 앱 연결(`attach`, macOS), 프레이밍(`framing`), JSON-RPC 모델(`jsonrpc`), 부모 프로세스
종료 감지(`parent`, macOS), protocol version 협상(`protocol`), 서버 루프(`server`)이고,
`pub use stellar_local_ipc as ipc`로 기존 경로를 유지합니다. 서버는 현재 `initialize`, `ping`,
`notifications/initialized`만 처리합니다. 초기화 전의 다른 request는 `SERVER_NOT_INITIALIZED`, 초기화 뒤의
다른 method는 `METHOD_NOT_FOUND`로 응답합니다(resources·tools는 XIX.2). headless 도구 `stellar-cli`는 이 브리지에 의존할 수 없습니다. 허용 직접 의존은 core-types,
diagnostics, local-ipc이고 실제로는 local-ipc만 씁니다. 전이 도달 금지 대상은 agent-host, ffi입니다.
주요 ADR은 ADR-0005, ADR-0006, ADR-0020입니다. ADR-0005 §3은 로직을 `crates/mcp-core`의
`stellar-mcp-core` library로 옮기고 이 package는 진입점만 갖게 하는 계획을 둡니다.

## Target Profile과 접근성

### stellar-target-profile

`crates/target-profile`은 Target Profile과 Capability Registry(Phase IX.10)입니다. core-types에만 의존하므로
`stellar.yaml` 텍스트는 project crate가 형식 중립 트리(`node`)로 바꿔 주고받는 구조이고, 문서·command·binding
계층의 입력은 이 crate의 지역 타입과 trait(예: upgrade의 `ProjectSnapshot`, 적용의 `UpgradeHost`)으로 받아 상위
crate가 구현합니다. module은 형식 중립 매니페스트 트리와 스키마 오류(`node`), profile 식별자·종류·schema
version(`profile`), 종류별 필드(`spec`), 프로젝트 profile 목록과 기본 profile(`manifest`), registry 버전과
프로젝트 pin(`registry`), capability 상태와 element·property·contract별 registry 표(`capability`), 앱 내장
registry release(`builtin`), 생성 가능 여부와 비활성화 사유 조회(`query`), Binding 환경 입력(`environment`),
profile 희소 override resolver(`overrides`), 다중 profile 검사(`validate`), upgrade plan(`upgrade`), 승인된
upgrade 적용과 undo(`apply`)입니다.

- `spec`·`manifest`가 정한 `target_profiles` 키와 `default_target_profile`은 미연결 초안입니다. project crate
  파서(`docs/spec/stellar-format.md` §14.5)와 호환되지 않고 crate graph에 project → target-profile edge도
  없습니다. 형식 통합과 연결 방식은 ADR 결정이 남아 있습니다(IX.10.3·IX.10.4 `[~]`).
- `environment`는 crate graph상 expressions를 쓸 수 없어 ADR-0026 환경 key registry의 profile key 1\~3과
  `platform` enum 번호를 지역 상수로 복제합니다. 통합 테스트 `constant_copies`가 두 사본을 대조합니다.

허용 직접 의존과 실제 의존은 core-types입니다.

### stellar-accessibility

`crates/accessibility`는 시각 layer와 독립된 접근성 의미 모델(기술 아키텍처 §18, Phase IX.8)입니다.
document·bindings 값을 직접 읽지 않고 StableId·PropertyPath·TypedValue로 출처를 가리킵니다. module은
canonical binary 도우미(`codec`), 접근성 의미 node(`node`), 시각 tree와 별도 parent/child를 갖는 접근성
트리(`tree`), 생성기 입력인 중립 시각 source tree(`source`), 기본 접근성 트리 생성기(`generate`), Control
contract의 role·state·action 매핑(`control`), label 누락·잘못된 중첩·focus·동작 불일치·읽기 순서 중복
진단(`diagnostics`), Target adapter용 중립 접근성 snapshot(`snapshot`)이고, `test_support`는 unit test
전용입니다. 허용 직접 의존과 실제 의존은 core-types입니다.

## 개발 도구 crate

### xtask

`tools/xtask`는 저장소 작업 runner이고 `.cargo/config.toml`의 alias(`cargo xtask` =
`cargo run --package xtask --`)로 실행합니다. 등록 명령은 `help`, `generate-ledger`,
`generate-ffi-header`, `validate-task-status`, `validate-task-report`, `validate-pr-body`,
`validate-commit-message`, `validate-versions`, `validate-dependencies`, `validate-adr`, `verify-task`,
`sync-plan`, `check-core`, `validate-crate-graph`, `validate-storage-fixtures`, `bench-chunk-size`,
`test-migrations` 17개이고, 종료 코드는 성공 0, 입력 오류 64, 정책 위반 65, 내부 오류 70입니다. 의존
방향 정책(`crate_graph`), ADR 보호 경로 정책(`adr_policy`), 의존성 승인 검사(`dependencies`), 커밋
제목 검사(`commit_message`), PR 본문 검사(`pr_body`), 버전 검사(`versions`), FFI 헤더 생성(`ffi_header`),
chunk 크기 측정(`chunk_bench`), migration fixture runner(`migration_fixtures`), storage fixture 검사
(`storage_fixtures`), workspace fmt·clippy·test·doctest 통합 검증(`check_core`), 명령 dispatch(`commands`),
계획서·작업 추적·보고서 검증(`plan`, `task_status`, `task_report`, `sync`), 제한 YAML(`yaml`)이 module로
나뉩니다. 내부 도구라 `main.rs`에서
`missing_docs`를 allow 하고, edition은 workspace 값을 상속하지 않고 직접 `"2021"`로 적습니다. 의존은
storage, core-types(chunk 크기 측정 harness용)와 `yaml-rust2 =0.11.0`이며, crate graph 정책 검사 대상은
아닙니다.

### stellar-bench-fixtures

`benchmarks/fixtures`는 성능 기준 fixture 생성기(XVI.3)입니다. 제품 crate가 아니며 제품 코드가 이
crate에 의존하지 않습니다. fixture 파일을 저장소에 넣지 않고 generator version, fixture 종류, seed,
규모 매개변수로 필요할 때 다시 만듭니다. 문서는 실제 문서 모델로 조립해 전체 validator를 통과시키고,
`stellar_project::document_io`로 chunk model을 만든 뒤 content hash를 계산합니다. module은 대규모
synthetic fixture(`synthetic`), 한계 fixture(`limits`), workload fixture(`workload`), sparse synthetic
asset provider(`asset_provider`), chunk split 계획 요약(`split`), content hash(`digest`)이고, 공용 부품으로
결정론적 PRNG(`rng`), StableId 발급기(`ids`), 생성 오류(`error`), payload factory와 문서 마무리(`build`),
생성 CLI(`cli`)가 있습니다. 실행
파일은 `cargo run -p stellar-bench-fixtures --release -- <fixture>`로 요약 JSON을 출력합니다. 의존은
core-types, document, expressions, storage, project, layout, assets이고 crate graph 정책 검사 대상은
아닙니다. 주요 ADR은 ADR-0019입니다.

### stellar-integration-tests

`tests/integration`은 여러 crate를 잇는 종단 시나리오 테스트 전용 crate입니다(ADR-0019와 같은 개발용
crate 원칙). 제품 crate는 이 crate에 의존하지 않고, 테스트는 crate의 `tests/` 아래에 둡니다.

- 프로젝트 시나리오: `project_lifecycle`, `recovery_round_trip`, `chunk_layout`, `component_library`,
  `gesture_grouping`, `multi_window`, `random_sequences`. 공용 helper는 `tests/common/`(`mod.rs`,
  `builders.rs`)입니다.
- Git 시나리오: `git_snapshot_diff`, `git_semantic_merge`, `git_trust_gate`. 공용 helper는
  `tests/git_common/mod.rs`입니다(git-porcelain fixture harness 사용).
- 상수 사본 대조: `constant_copies`. crate graph상 서로 의존할 수 없는 crate가 따로 보관하는 같은 계약 값이
  같은지 검사합니다. 대상은 환경 key registry의 target-profile 사본과 expressions 원본, Display P3 ↔ sRGB
  행렬의 expressions·renderer-core 사본입니다.
- 알려진 결함을 재현하는 테스트 하나는 `#[ignore]`입니다. `chunk_layout`의
  `product_serializer_splits_views_over_approved_threshold`(V.6.4 보고서 "남은 위험")입니다. SM-1 재현 테스트
  `git_semantic_merge`의 `independent_page_and_view_insertions_merge_cleanly`는 수정 후 기본 실행 대상입니다.

`[dependencies]`는 비어 있고, `[dev-dependencies]`에 core-types, document(`test-support`),
commands(`test-support`), history, storage, project, git-porcelain(`test-support`), semantic-diff,
semantic-merge, expressions(`test-support`), renderer-core(`test-support`), target-profile이 있습니다. crate
graph 정책 검사 대상은 아닙니다.

## 관련 문서

- [Crate 의존 그래프](../architecture/crate-dependency-graph.md): 허용 직접 의존의 사람용 기준표
- [ADR 목록](../adr/)
- [기술 아키텍처 정의서](../PROJECT_STELLAR_TECHNICAL_ARCHITECTURE.md)
- [구현 계획서](../PROJECT_STELLAR_IMPLEMENTATION_PLAN.md)
- [stellar 명령 사용 안내](stellar-cli.md)
