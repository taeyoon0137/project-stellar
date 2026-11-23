# ADR-0087: 활성 기본값 세트를 문서를 열 때 모델에 넣고, 저장할 때 `stellar.yaml`에 되쓰며, 조회 결과에 값 출처를 둔다

## Status

Accepted (2026-11-23).

- ADR-0082(생략값 기본값 세트)와 ADR-0086 결정 10·11이 후속으로 남긴 연결을 정합니다.
- ADR-0086 결정 10의 "`SetStyleDefaults`를 실행하는 제품 경로는 두지 않는다"는 ops 경로에 한해 이 문서로 풀립니다.
  앱 runtime 경로는 아직 연결하지 않습니다(결정 4).

## Context

- 활성 세트의 원본은 `stellar.yaml`의 `style_defaults`입니다(ADR-0082 결정 4). `DocumentModel`은 그 이름을 거울로
  들고 있고, `SetStyleDefaults` command는 이 거울만 바꿉니다(ADR-0086 결정 10). 거울이 있어야 undo 한 번으로
  되돌아갑니다.
- 지금까지는 문서를 여는 경로가 거울을 언제나 `css`로 두었고, 저장 경로는 거울을 `stellar.yaml`에 쓰지 않았습니다.
  그래서 `stellar apply`에 canonical `SetStyleDefaults`를 넣으면 command는 성공하지만 결과가 조용히 사라졌습니다.
- 조회 결과(`stellar query`, MCP 읽기)에 값 출처가 없었습니다(ADR-0082 결정 5). 출처는 `stellar style-defaults
  resolve`에만 있었습니다.
- 제약이 있습니다.
  - `stellar.yaml`은 Git 원본 파일이고, 문서 내용은 generation(chunk·manifest·current pointer)입니다. 두 곳을 한
    번에 원자적으로 바꾸는 장치는 없습니다. 여러 파일 저장 journal(ADR-0078)은 텍스트 원본 파일끼리의 거래이고
    generation pointer 교체를 담지 않습니다.
  - MCP는 프로젝트 문자열(경로, 세트 이름이 든 진단 문구)을 오류 문구에 넣지 않습니다(XIX.9.6).
  - headless MCP에는 node 하나의 속성을 읽는 tool이 따로 없습니다. tool 목록은 agent-protocol이 정하고, 이 결정은
    목록을 바꾸지 않습니다.

## Decision

1. **열기: snapshot이 활성 세트를 한 번 평탄화하고 모델 거울을 채웁니다.**
   - `ops::snapshot::open_snapshot`은 `stellar.yaml` 선택과 `design/style-defaults/` 사용자 세트를 읽어
     `manifest_style_defaults::active_set`으로 한 번 평탄화합니다. 결과는 `ProjectSnapshot::style_defaults()`입니다.
   - 세트를 풀지 못해도(사용자 세트 파일 형식 오류, 없는 세트, 순환) 열기는 실패하지 않습니다. 문서 읽기·검증·Git
     같은 다른 조회가 세트 파일 하나 때문에 막히면 안 되기 때문입니다. 세트를 쓰는 조회만 그 오류를 돌려줍니다.
   - 쓰지 않는 사용자 세트의 사슬 오류(예: 고르지 않은 세트의 순환)는 그 세트를 고를 때 드러납니다. 파일 형식
     오류는 세트를 고르지 않아도 읽을 때 드러납니다.
   - `ProjectSnapshot::document_model`은 모델 거울에 `style_defaults_name()`(매니페스트 선택, 키가 없으면 `css`)을
     넣습니다. 문서 영속 레코드(`to_records`)에는 거울이 들어가지 않으므로 chunk bytes는 그대로입니다.
2. **저장: 바뀐 거울을 두 단계로 `stellar.yaml` 한 줄에 씁니다.**
   - project API는 `prepare_selection(root, expected, name)`과 `PreparedSelection::commit(root)`입니다.
     - 준비 단계는 파일을 쓰지 않습니다. `stellar.yaml`을 읽어 지금 선택이 `expected`(연 시점의 이름)인지 보고, 다르면
       `style-defaults-stale`입니다. `name` 세트가 이 프로젝트의 사용자 세트와 함께 평탄화되는지도 봅니다.
     - commit은 준비할 때 읽은 bytes hash가 그대로일 때만 씁니다(`source_file::write_checked`). 선택이 같으면 파일을
       건드리지 않습니다.
     - `css`를 고르면 키를 지웁니다. 그래서 세트 변경과 되돌림은 Git diff에서 한 줄입니다.
     - 기존 `set_active`는 `prepare_selection(root, None, name)` 뒤 `commit`과 같습니다. 같은 일을 하는 코드가 두 벌이
       아닙니다.
   - `ops::apply`는 transaction 뒤 모델 거울이 연 시점 이름과 다르면 semantic diff 계산 뒤에 준비합니다.
     - 준비가 실패하면 마지막 `SetStyleDefaults` 입력 index의 `ApplyError::Rejected`입니다. dry-run도 같은 검사를
       하고 쓰지 않습니다.
     - 쓰기 모드는 generation 저장(pointer 교체)이 끝난 뒤 commit합니다. commit이 실패하면 `ApplyError::Save`이고,
       문구에 "generation은 저장했지만 stellar.yaml 기본값 세트를 쓰지 못했습니다"를 적습니다.
   - 순서를 이렇게 정한 이유는 다음과 같습니다.
     - generation 저장의 실패(다른 writer, 디스크)가 가장 흔한 실패입니다. 그 실패가 `stellar.yaml`을 바꾸지 않게
       하려면 generation이 먼저여야 합니다. 반대 순서는 실패할 때 `stellar.yaml`을 되돌리는 보상 쓰기가 필요하고,
       그 보상도 실패할 수 있습니다.
     - commit 실패는 준비 단계의 검사를 모두 지난 뒤의 I/O 오류나, 같은 순간 잠금을 쓰지 않는 외부 편집뿐입니다.
       ops 쓰기는 교차 프로세스 저장 잠금(XXIV.4.5) 안에서 돌기 때문입니다. 이때 남는 상태는 "문서 변경은 저장,
       세트 변경은 미반영"이고, 다시 `SetStyleDefaults`를 적용하면 맞춰집니다. 오류 문구가 이를 알립니다.
   - semantic diff 결과에는 세트 변경이 나오지 않습니다. 세트는 문서 레코드가 아니라 `stellar.yaml`의 값이기
     때문입니다. 세트만 바꾸는 apply는 `changes`가 0개이고 `stellar.yaml` 한 줄이 바뀝니다.
3. **조회 출처(ADR-0082 결정 5).**
   - `ops::query::node_layout(snapshot, document, node)`가 node의 layout longhand 전부(`LayoutLonghand::ALL`)를 연
     snapshot의 활성 세트로 풉니다. 해석은 문서 crate `style_defaults::resolve` 하나이고, 값 표기는
     `layout_record::format_value` 하나입니다.
   - 출처는 고정 크기 enum `LayoutSource { Explicit, Default }`입니다. 세트 이름은 결과의 `style_defaults` 필드에
     한 번만 둡니다. 표기는 `explicit` 또는 `default(<세트 이름>)`(`NodeLayout::source_label`)입니다.
   - 세트 값과 같은 명시 값은 `explicit`입니다. `UnsetProperty` 뒤에는 `default(<세트>)`입니다.
   - 풀 수 없는 경우는 `LayoutUnavailable`입니다. 상자가 없는 node(Fragment)는 `NoBox`, 세트를 풀지 못하면
     `StyleDefaults { code }`입니다. 이 결과는 조회 전체를 실패시키지 않습니다. 문구는 프로젝트 문자열 없는 고정
     문구이고, 자세한 진단은 `stellar style-defaults show`가 냅니다.
   - 출력 위치는 둘입니다. JSON 모양은 같습니다(`{style_defaults, class, values: [{longhand, value, source}]}`,
     NoBox는 `null`, 세트 오류는 `{error: {code, message}}`).
     - CLI: `stellar query <경로> node <문서> <node>`의 `node.layout`(`ops::query::node_detail`의 `layout`).
     - MCP: `document.copy_css` 결과의 `layout`. node 하나의 값을 읽는 기존 headless tool이 이것뿐이라 여기에 둡니다.
       세션 snapshot에서 `node_layout`을 부릅니다. tool 목록과 입력 schema는 바뀌지 않습니다.
   - 두 출력이 같다는 것은 같은 fixture로 한 golden(`tools/stellar-mcp/tests/fixtures/layout-provenance.json`)을 두
     test(CLI·MCP)가 비교해 고정합니다.
4. **runtime(앱) 연결은 이 결정의 API를 그대로 씁니다.** 앱 열기·저장 경로(`crates/runtime`)는 text-source-default
   작업이 연결합니다. 이 결정은 runtime을 고치지 않고 다음 사용법을 정합니다.
   - 열 때: `manifest_style_defaults::active_set(root, &manifest)`로 세트를 평탄화하고, 성공하면
     `model.set_style_defaults(set.name())`로 거울을 채웁니다. 실패하면 거울은 매니페스트 이름
     (`active_name(&manifest)`)으로 두고 layout 해석만 진단으로 막습니다(결정 1과 같은 규칙).
   - 저장할 때: 모델 거울이 연 시점 이름과 다르면 generation 저장 전에
     `prepare_selection(root, Some(opened), model.style_defaults())`로 준비하고, 실패하면 저장을 시작하지 않습니다.
     generation 저장이 끝나면 `commit(root)`합니다. 순서와 실패 처리는 결정 2와 같습니다.
   - 외부에서 `stellar.yaml` 선택이 바뀌면(ADR-0059 외부 변경 반영) 연 시점 이름을 새 이름으로 바꾸고 거울도 새
     이름으로 둡니다. 그 반영 경로는 runtime 작업 몫입니다.

## Alternatives

| 대안 | 판단 |
|---|---|
| `stellar.yaml`을 generation보다 먼저 쓰고 실패하면 되돌림 | 채택하지 않음. 가장 흔한 실패(generation CAS·디스크)마다 보상 쓰기가 필요하고, 보상이 실패하면 세트만 바뀐 상태가 남습니다 |
| `stellar.yaml`과 generation을 저장 journal(ADR-0078) 한 거래로 묶음 | 지금은 채택하지 않음. journal은 텍스트 파일 거래이고 generation pointer 교체를 담지 않습니다. 텍스트 원본이 기본 원본이 되면(V.10.7) 같은 journal 거래로 옮길 수 있습니다 |
| 세트를 못 풀면 `open_snapshot` 자체를 실패 | 채택하지 않음. 세트 파일 하나의 오류가 검증·Git·트리 조회를 모두 막습니다 |
| 출처 enum에 세트 이름 문자열을 값마다 둠 | 채택하지 않음. 40개 값마다 같은 이름이 반복됩니다. ADR-0082 Consequences대로 고정 크기 enum과 이름 한 번입니다 |
| MCP에 node 속성 읽기 tool을 새로 둠 | 지금은 채택하지 않음. tool 목록·권한·schema는 agent-protocol 결정이고 이 작업의 범위 밖입니다. 기존 `document.copy_css`가 node 하나의 값을 읽는 tool입니다 |

## Consequences

- `stellar apply`의 canonical `SetStyleDefaults`가 실제로 `stellar.yaml`에 반영됩니다. 되돌리는 같은 command 하나가
  원래 bytes로 돌려놓습니다.
- `stellar query … node`와 MCP `document.copy_css`가 layout 값과 출처를 냅니다. CLI golden(`query-node.json`)에
  `layout` 필드가 더해졌습니다.
- 한계가 있습니다.
  - MCP 세션 cache의 최신 판정(`TrustedProject::snapshot_is_current`)은 `stellar.yaml`과 generation만 봅니다. 사용자
    세트 파일만 바뀌면 같은 세션의 `layout` 결과가 옛 세트로 남을 수 있습니다. 판정에 세트 파일 digest를 더하는
    일은 `ops::trusted` 소유 작업입니다.
  - `stellar style-defaults resolve`(CLI)는 아직 snapshot의 세트(`ProjectSnapshot::style_defaults`)가 아니라 같은 공개
    함수(`load_user_sets`·`flatten_named`)를 직접 부릅니다. 결과는 같지만 `ops::query::node_layout`으로 옮기는 일이
    남았습니다(CLI 소유).
  - 속성 탭 snapshot의 출처 표시는 UI 재설계 때 같은 `node_layout`을 씁니다.
  - 앱 runtime의 열기·저장 연결은 결정 4의 사용법대로 runtime 작업이 합니다.

## Compatibility

- 파일 포맷 변경은 없습니다. `stellar.yaml`의 `style_defaults` 키와 사용자 세트 형식은 ADR-0082·ADR-0086 그대로입니다.
- 공개 API가 늘었습니다.
  - `stellar_project::manifest_style_defaults`: `prepare_selection`, `PreparedSelection`,
    `StyleDefaultsFileError::Stale`.
  - `stellar_ops::snapshot::ProjectSnapshot`: `style_defaults_name`, `style_defaults`.
  - `stellar_ops::query`: `node_layout`, `NodeLayout`, `LayoutProperty`, `LayoutSource`, `LayoutUnavailable`,
    `NodeDetail::layout`.
- CLI·MCP JSON에 필드가 더해졌습니다(`node.layout`, `copy_css`의 `layout`). 기존 필드는 그대로입니다.
- FFI 변경은 없습니다.

## Performance

- 열 때 사용자 세트 디렉터리를 한 번 읽고 평탄화합니다. 세트 파일은 256개, 파일마다 1 MiB, 사슬 16단계가 상한입니다.
- `node_layout`은 longhand 40개의 고정 배열 조회입니다. 문서 하나를 읽는 비용(chunk 로드)이 지배합니다.
- 세트를 바꾸는 apply는 기존 저장 경로에 `stellar.yaml` 한 번 읽기·쓰기가 더해집니다.

## Security

- `stellar.yaml` 쓰기는 symlink가 아닌 정규 파일만 받고, 읽은 bytes hash가 그대로일 때만 씁니다.
- 사용자 세트 파일은 기존 제한 YAML loader의 크기·깊이 상한과 symlink 거부를 그대로 씁니다.
- MCP 결과의 세트 오류는 안정 code와 고정 문구만 담습니다. 경로·세트 이름이 든 진단 문구를 넣지 않습니다.

## Migration

- 데이터 migration은 없습니다. 키가 없는 기존 프로젝트는 `css` 거울로 열리고, 저장 결과 bytes는 예전과 같습니다.
