# ADR-0128: 문서가 그대로인 apply는 generation을 쓰지 않고, 프로젝트 revision에 세트 선택을 넣으며, SVG·CSS 수 표기를 한 함수로 합친다

## Status

Accepted (2026-11-24).

- ADR-0115 결정 4(CLI `style-defaults set`)와 Consequences의 한계, ADR-0091 결정 6(SVG 값 표기 위치)의 후속입니다. 두
  문서의 본문은 고치지 않고 이 문서가 보완합니다.
- 계획서 행 IX.11.29(`set`이 ops 쓰기 경로를 거침)와 IX.11.20(SVG 매핑표·formatter 정리)의 남은 범위를 다룹니다.
- ADR-0082 결정 4("Git diff에서 한 줄"), ADR-0087 결정 2(두 단계 쓰기), ADR-0005 §1(파일이 원본)을 따릅니다.

## Context

- `ops::apply`는 transaction 결과 문서 레코드가 적용 전과 같아도 원본·cache 저장을 했습니다. canonical
  `SetStyleDefaults` 하나만 담은 transaction도 `design/current`와 새 `design/generations/*.starmanifest`를 써서, 세트만
  바꾸는 편집의 Git diff가 세 파일이 됐습니다. 그래서 ADR-0115 결정 4는 CLI `set`을 `ops::apply` 밖의 두 단계 쓰기로
  따로 두었습니다. 같은 command의 쓰기 경로가 둘이었습니다.
- 프로젝트 revision(`ops::revision::ProjectRevision`)은 generation 번호와 manifest hash(원본에서 읽으면 원본 revision
  hash)뿐이었습니다. 세트만 바꾸는 쓰기는 generation을 쓰지 않으므로 revision이 그대로입니다.
  - 그래서 writer A가 revision R을 읽은 뒤 writer B가 세트를 바꿔도 A의 `--base R`은 통과했습니다. A의 쓰기는 B의 세트
    변경 위에서 실행됐습니다(문서 편집은 세트 해석 결과에 영향을 받음).
  - CLI `set`은 준비 단계의 "연 시점 이름" 확인과 commit의 hash 확인으로 `stellar.yaml` 덮어쓰기만 막았습니다. 다른
    명령의 base 검사에는 보이지 않았습니다.
- IX.11.20은 SVG 값 표기를 CSS 값 formatter 한 곳으로 모으라고 합니다. ADR-0091 결정 6은 export → document 의존을 열지
  않고, 합치려면 수 표기를 두 crate가 함께 쓰는 core-types로 옮겨야 한다고 적었습니다. 지금은
  `css::value::format_number`와 `svg::format::write_number`가 같은 규칙을 따로 구현하고 종단 test로만 같음을 지킵니다.
- 매핑표 행의 원천 registry 항목(`SvgFeature::registry_items`, ADR-0121 결정 7)은 export module test가 codegen matrix
  golden과만 대조합니다. registry 자체와의 대조, 그리고 항목의 CSS 매핑이 행의 `css_longhands`를 덮는지는 확인하지
  않았습니다.

## Decision

1. **문서가 그대로인 transaction은 원본·cache를 쓰지 않습니다.**
   - `ops::apply` 쓰기 모드에서 결과 문서 레코드(canonical 정렬)와 Component Context가 적용 전과 같으면 원본·cache
     저장(`snapshot::save`)을 건너뜁니다. 이미 같은 값을 넣는 command, 세트만 바꾸는 `SetStyleDefaults`가 여기에
     해당합니다.
   - 세트를 바꿨으면 `stellar.yaml` 한 줄만 씁니다(ADR-0087 결정 2의 준비 → commit). 결과의 `written_chunks`는 0,
     `source`는 없음입니다.
   - 잠금 안에서 연 snapshot으로 base revision을 비교한 뒤라 그 사이 다른 저장이 끼지 않습니다. 세트 한 줄은 준비 때
     읽은 bytes hash가 그대로일 때만 씁니다.
   - 문서도 바뀌는 transaction은 전처럼 원본·cache를 쓴 뒤 세트 한 줄을 씁니다.
2. **프로젝트 revision에 세트 선택을 넣습니다.**
   - `ProjectRevision::with_style_defaults(selection)`이 원본·cache revision에 `stellar.yaml` 활성 세트 이름(키가 없으면
     `css`)을 더합니다.
     - 선택이 기본 세트 `css`이면 revision을 바꾸지 않습니다. 세트를 고르지 않은 프로젝트의 표기는 전과 같고, 세트를
       바꿨다가 `css`로 되돌리면 처음 revision과 같습니다. 내용이 같은 상태라 같은 revision이어도 됩니다.
     - 그 밖에는 generation 번호를 두고 hash를 영역 구분자(`stellar.project-revision.style-defaults.v1`과 줄바꿈),
       이전 hash 표기, 줄바꿈, 세트 이름을 이은 bytes의 content hash로 바꿉니다. 저장된 generation이 없으면 이전 hash
       자리는 `-`이고 결과는 `g0:<hash>`입니다.
   - `ProjectSnapshot::revision`은 이 값입니다. 더하기 전 값은 `ProjectSnapshot::storage_revision`입니다.
     `snapshot::save::save_documents`의 결과 revision도 세트 선택을 더한 값입니다. 같은 쓰기에서 세트도 바꾸는
     `ops::apply`는 `save_documents_selecting`에 새 선택을 넘깁니다.
   - 새 generation을 쓰는 다른 ops 경로도 결과 revision에 세트 선택을 더합니다(`ops::migrate` 결과는 다시 연
     revision과 같음). HEAD generation과 같은지 보는 비교(`compare`의 HEAD 재사용 판정)는 세트 선택을 더하지 않은
     `storage_revision`의 hash를 manifest hash와 대조합니다. 그래서 세트를 고른 프로젝트도 빠른 경로를 씁니다.
   - revision 표기 해석은 `g0:<hash>`를 받습니다. 1 이상 generation에 hash가 없는 표기는 전처럼 거부합니다.
   - 그래서 다른 writer의 세트 변경도 모든 ops 쓰기(`stellar apply`, `stellar css paste`, MCP, Merge Composer target,
     `style-defaults set`)의 base revision 비교가 잡습니다.
   - `ProjectRevision::Saved`의 `manifest_hash` 필드 이름은 그대로 둡니다. 원본에서 읽은 상태가 이미 manifest hash가
     아닌 원본 revision hash를 담았습니다. 이 결정으로 뜻이 "revision hash"로 넓어집니다. 필드 이름을 바꾸면 L2·runtime
     호출자가 모두 바뀌어 이 작업 범위를 넘습니다.
3. **CLI `style-defaults set`은 `ops::apply` 하나로 씁니다.**
   - canonical `SetStyleDefaults` 하나를 담은 요청을 `ops::apply`에 넘깁니다. 저장 잠금, 다시 열기, base revision 검사,
     세트 준비, 쓰기 순서는 `stellar apply`와 같습니다. `--dry-run`은 잠금 없이 준비까지 합니다.
   - 대상 문서는 root 문서 순서의 첫 문서입니다. `SetStyleDefaults`는 문서 모델의 세트 거울을 바꾸는 command라 문서가
     하나 필요하고, 선택은 프로젝트 하나라 어느 문서에 적용해도 결과가 같습니다. 저장된 문서가 없으면 `no-document`
     (exit 65)입니다.
   - 없는 세트·순환 같은 세트 파일 오류는 적용 전에 `flatten_named`로 확인해 그 code(`unknown-style-defaults` 등)로
     알립니다. 판정 함수는 적용 경로의 준비 단계와 같은 함수입니다.
   - `--base`가 없으면 명령을 시작할 때 연 revision을 씁니다. 그 뒤 다른 writer가 바꾸면 덮어쓰지 않고
     `base-revision-mismatch`입니다.
   - 결과 `revision`은 쓰기 뒤 revision이라 세트를 바꾸면 base와 다릅니다(결정 2).
4. **수 표기를 `stellar_core_types::number` 하나로 둡니다.**
   - `number::write_number`·`format_number`가 `f32`를 왕복하는 가장 짧은 10진 표기(지수 없음, `-0`은 `0`)를 씁니다.
   - CSS 값 formatter `css::value::format_number`와 SVG profile `svg::format::write_number`가 이 함수를 부릅니다. SVG
     profile은 유한하지 않은 값을 `NonFinite`로 거부하는 검사만 더합니다.
   - 단위(`px`·`%`), 구분자(CSS 행렬 `, `, SVG 행렬 공백), 색 표기는 각 profile이 이 함수 위에 정합니다. 이 부분은
     표기 규칙 자체가 다르므로 합치지 않습니다(ADR-0091 결정 6의 SVG profile 목록).
   - export → document 의존은 열지 않습니다(ADR-0019). core-types는 두 crate가 이미 의존합니다.
5. **매핑표 원천 항목을 registry와 대조합니다.**
   - 종단 test(`tests/integration/tests/svg_registry_mapping.rs`)가 행마다 `registry_items`의 (범위, 이름)이 registry
     편집 항목 하나인지와, 그 항목들의 CSS 매핑 longhand 합집합이 그 행의 `css_longhands`를 모두 덮는지 봅니다.

## Alternatives

| 대안 | 판단 |
|---|---|
| revision 표기에 세트 이름을 그대로 붙임(`g3:<hash>+react-native`) | 채택하지 않음. `ProjectRevision`은 `Copy` 값이고 L2·runtime·agent-edit가 variant를 직접 만듭니다. 새 필드는 그 호출자를 모두 바꿉니다. hash에 접으면 표기 형식과 타입이 그대로입니다 |
| 기본 세트도 hash에 접음 | 채택하지 않음. 세트를 고르지 않은 모든 프로젝트의 revision 표기가 바뀝니다 |
| 사용자 세트 파일(`design/style-defaults/*`) 내용까지 revision에 넣음 | 지금은 채택하지 않음. 목표는 `stellar.yaml` 선택 변경을 잡는 것입니다. 세트 파일 편집은 생략값 해석에 영향을 주지만 쓰기 경로가 따로 없고, 넣으려면 열 때마다 세트 파일을 모두 hash해야 합니다. 후속에서 정합니다 |
| CLI `set`을 계속 두 단계 쓰기로 둠 | 채택하지 않음. 같은 command의 쓰기 경로가 둘이 되고, base 검사 규칙도 따로 지켜야 합니다 |
| 문서가 없는 프로젝트에서 CLI가 두 단계 쓰기로 물러섬 | 채택하지 않음. 이중 경로입니다. 저장된 문서가 없는 프로젝트는 앱이 만들지 않는 상태입니다 |
| 수 표기를 document에 두고 export가 document에 의존 | 채택하지 않음. ADR-0019·ADR-0091 결정 6의 경계(export는 Scene Snapshot만 읽음)를 깹니다 |
| CSS·SVG 행렬·색 표기까지 한 함수로 | 채택하지 않음. 구분자와 색 문법이 profile마다 다른 규칙입니다. 같은 부분(수)만 합칩니다 |

## Consequences

- `stellar apply`·MCP·앱 저장 경로 밖의 ops 쓰기로 세트만 바꾸면 Git diff가 `stellar.yaml` 한 줄입니다(ADR-0115
  Consequences의 한계 해소).
- 이미 같은 값을 넣는 transaction은 아무 파일도 쓰지 않습니다. revision도 그대로입니다.
- 다른 writer의 세트 변경이 base revision을 바꾸므로 낡은 revision으로 한 쓰기는 거부됩니다.
- `style-defaults set`과 `stellar apply`의 `SetStyleDefaults`가 같은 경로이고 결과 bytes가 같습니다.
- SVG 수 표기와 CSS 수 표기가 같은 함수라 한쪽만 바뀔 수 없습니다. 레이아웃 선언 표기(`crates/layout`
  `declaration_text`)도 같은 함수로 옮길 수 있습니다(후속).
- 한계가 있습니다.
  - runtime 창의 `agent_edit::project_revision`은 창이 연 generation만 봅니다. 세트가 `css`가 아닌 프로젝트에서는 MCP
    headless revision과 표기가 다릅니다. runtime이 세트 거울을 `stellar.yaml`과 잇는 작업(활성 세트 거울 연결)에서
    `with_style_defaults`를 불러야 합니다.
  - 세트를 바꿨다가 되돌리는 ABA는 revision이 같습니다. 내용이 같은 상태라 의도한 동작입니다.
  - `docs/spec/style-defaults.md` 3장의 `set` 설명은 ADR-0115 기준 문장입니다(결과는 같음). 문서 소유 작업이 이
    결정을 덧붙입니다.

## Compatibility

- 파일 포맷 변경은 없습니다. `stellar.yaml`, chunk, 원본, command codec이 그대로입니다.
- revision 표기: 세트가 `css`인 프로젝트는 그대로입니다. 다른 세트를 고른 프로젝트는 hash가 달라지고, 저장 전
  프로젝트는 `g0:<hash>`가 생깁니다. 해석기는 `g0:<hash>`를 받습니다. 이전 표기로 저장해 둔 base revision은 세트를
  고른 프로젝트에서 불일치가 됩니다. 다시 조회해 얻습니다.
- 공개 API가 늘었습니다: `ProjectRevision::with_style_defaults`, `ProjectSnapshot::storage_revision`,
  `snapshot::save::save_documents_selecting`, `stellar_core_types::number::{write_number, format_number}`.
- `ApplyOutcome`의 필드는 그대로이고, 문서가 그대로인 쓰기의 `written_chunks`·`source` 값이 0·없음이 됩니다.
- CLI `style-defaults set`: 결과 필드는 그대로입니다. `revision`이 쓰기 뒤 값이 되고, 저장된 문서가 없으면
  `no-document`입니다.
- FFI 변경은 없습니다.

## Performance

- 결과 비교는 이미 만든 적용 전후 레코드·Context의 비교 한 번입니다(문서 크기 선형). 같으면 저장 전체(직렬화·해시·
  쓰기)를 건너뛰어 더 빠릅니다.
- revision 계산은 세트가 `css`가 아닐 때 짧은 hash 한 번입니다.
- 수 표기는 같은 `Display` 규칙이고, SVG 경로는 값마다 만들던 임시 `String`을 없앴습니다.

## Security

- 쓰기 순서와 guard는 그대로입니다(저장 잠금, base revision, `stellar.yaml` 정규 파일·읽은 hash 확인).
- revision hash는 무결성 표지가 아니라 변경 감지용입니다. 같은 입력은 같은 hash입니다.

## Migration

- 데이터 migration은 없습니다.
- 세트를 고른 프로젝트의 자동화가 이전 revision 표기를 저장해 두었다면 다시 조회합니다.
