# `stellar` 명령 사용 안내

이 문서는 headless 명령 `stellar`(package `stellar-cli`, `tools/stellar-cli`)의 현재 동작과 계획을
설명합니다. 기존 `inspect` 실행 예시는 커밋 `993d7d0` 기준이며, 현재 명령 목록과
정적 접근성 HTML·고정 Frame HTML+CSS 경로는 코드·CLI 통합 테스트 기준입니다. 계획은
[ADR-0005](../adr/ADR-0005-agent-tool-layers.md)와 구현 계획서 Phase XXIV에서 옮긴 내용입니다.
두 부분을 섞지 않았습니다. 문서와 코드가 다르면 코드가 기준입니다.

## 목차

- [요약](#요약)
- [빌드와 실행](#빌드와-실행)
- [명령](#명령)
- [지정 View의 정적 접근성 HTML](#지정-view의-정적-접근성-html)
- [지정 View의 고정 Frame HTML+CSS](#지정-view의-고정-frame-htmlcss)
- [출력 계약](#출력-계약)
- [종료 코드](#종료-코드)
- [inspect 오류 분류](#inspect-오류-분류)
- [실행 예시](#실행-예시)
- [스크립트에서 쓰는 법](#스크립트에서-쓰는-법)
- [계획: ADR-0005 L1](#계획-adr-0005-l1)
- [관련 코드와 문서](#관련-코드와-문서)

## 요약

- `stellar`는 ADR-0005의 L1 도구입니다. AI와 무관한 headless 파일 도구이고, 실행 중인 앱 없이
  core crate로 프로젝트 파일을 직접 다룹니다.
- Cargo package와 library 이름은 `stellar-cli`이고, 실행 파일 이름만 `stellar`입니다
  (`tools/stellar-cli/Cargo.toml`의 `[[bin]] name = "stellar"`).
- 현재 명령은 아래 표와 `stellar --help`에서 확인합니다. `export-accessibility`는 지정 View의
  정적 접근성 HTML 조각을, `export-web`은 고정 Free Frame·Fragment의 HTML+CSS를 읽기 전용으로 만듭니다.
  `apply`, `migrate`, `text-source`에는 쓰기 동작이 있습니다.
- 앱 UI, GUI, 네트워크 없이 동작합니다. 내부 직접 의존은 `stellar-ops`이고 외부 crate 직접 의존은 없습니다.
- 명령이 실행되면 stdout에는 결과 JSON 한 줄(schema `stellar-cli/result` v1)만, stderr에는 사람이
  읽는 진단만 씁니다. 텍스트 출력은 도움말뿐이고, 사용법 오류는 stderr에만 씁니다.

## 빌드와 실행

```sh
# 실행 파일 target/debug/stellar를 만듭니다.
cargo build -p stellar-cli

# 빌드와 실행을 한 번에 합니다.
cargo run -q -p stellar-cli -- --help
```

- 저장소 밖 디렉터리에서 실행하려면 `cargo run -q --manifest-path <저장소>/Cargo.toml -p stellar-cli -- <명령>`처럼
  workspace manifest를 지정하거나, 빌드된 `target/debug/stellar`를 직접 실행합니다.
- 사용자 셸 PATH에 `stellar`를 설치하는 기능은 아직 없습니다. 계획은 [배포와 설치](#배포와-설치)를
  봅니다.
- 인자는 UTF-8이어야 합니다. UTF-8이 아닌 인자는 panic 없이 사용법 오류(64)로 끝납니다.
- 테스트(`tests/headless_help.rs`)는 환경 변수를 모두 지우고(HOME, PATH, DISPLAY, TMPDIR 없음) 빈
  작업 디렉터리와 닫힌 stdin으로 도움말이 성공하는지 확인합니다.

## 명령

형식은 `stellar <명령> [인자]`입니다. 첫 인자가 명령이고, 전역 옵션은 없습니다.
명령별 옵션은 `stellar --help`에 표시합니다.

| 명령 | 같은 뜻의 인자 | 인자 | 출력 |
|---|---|---|---|
| `help` | `--help`, `-h` | 없음 | stdout에 텍스트 도움말, 종료 코드 0 |
| `version` | `--version`, `-V` | 없음 | stdout에 결과 JSON |
| `inspect` | 없음 | `<경로> [--structure]` | 프로젝트 검사, 구조·revision |
| `query` | 없음 | `<경로> <대상>` | project·tree·node·tokens·components 조회 |
| `validate` | 없음 | `<경로>` | 저장 문서 검증 |
| `semantic-diff` | 없음 | `<base> <target>` | 의미 차이 |
| `semantic-merge` | 없음 | `<base> <ours> <theirs>` | 메모리 내 merge·conflict |
| `migrate` | 없음 | `<경로> [--check]` | 스키마 migration 또는 판정 |
| `diff-summary` | 없음 | `<base> <target> [--output <파일>] [--include-content]` | 요약 artifact |
| `ci` | 없음 | `<경로>` | lockfile·문서·migration 검사 |
| `apply` | 없음 | `<경로> <입력 파일 또는 -> [--dry-run]` | transaction 검증·저장 또는 diff |
| `text-source` | 없음 | `export <프로젝트> <새 파일>`, `check <파일>`, `apply <파일> <입력 파일 또는 -> [--dry-run]` | 단일 View 텍스트 원본 생성·검사·편집. [문법과 지원 범위](../spec/text-view-source-v1.md) |
| `export-accessibility` | 없음 | `<경로> <문서 id> <View id>` | 정적 접근성 HTML 조각·revision·진단 |
| `export-web` | 없음 | `<경로> <문서 id> <View id>` | 고정 Free Frame HTML+CSS·revision·진단 |

명령이 없거나(`stellar`만 실행) 알 수 없는 명령이면 사용법 오류(64)입니다.

### `help`

- 도움말 텍스트를 stdout에 쓰고 0으로 끝납니다. 내용은 `tools/stellar-cli/tests/fixtures/help.txt`
  snapshot과 같아야 합니다.
- 도움말에는 명령 목록, stdout·stderr 역할, 종료 코드 표가 들어 있습니다.
- 인자를 붙이면(`stellar help extra`) 사용법 오류(64)입니다.
- stdout 쓰기에 실패하면 74(`ioerr`)로 끝납니다.

### `version`

인자를 받지 않습니다. 성공하면 `result`는 다음 object입니다.

| key | 값 |
|---|---|
| `name` | package 이름. 현재 `"stellar-cli"`(`CARGO_PKG_NAME`) |
| `version` | package 버전. 현재 `"0.0.0"`(`CARGO_PKG_VERSION`) |
| `output_schema_version` | 결과 envelope schema 버전. 현재 `1` |

### `inspect`

`stellar inspect <경로>`는 경로의 프로젝트 root와 `stellar.yaml`을 읽기 전용으로 검사합니다.
`stellar_project::probe::probe_project`(IV.5.1 probe)를 호출하며, 문서 청크(`design/` 등)는
읽지 않습니다.

**인자 규칙**

- 경로가 정확히 하나 있어야 합니다. 없으면 사용법 오류(64)입니다.
- 인자가 하나이고 `-`로 시작하면 알 수 없는 옵션으로 보고 64로 끝납니다. 단 `-` 한 글자는
  표준 입력이 아니라 `-`라는 이름의 경로로 다룹니다.
- 인자가 둘 이상이면 "경로 하나만 받습니다"라는 사용법 오류(64)입니다. 그래서
  `stellar inspect --json demo-project`는 옵션 오류가 아니라 인자 개수 오류로 보고됩니다.

**검사 순서**

1. 경로의 metadata를 읽습니다. 실패하면 io 오류 종류로 종료 코드를 정합니다. 경로가 없으면
   66 `no-input`, 권한이 없으면 77 `no-permission`, 그 밖에는 `io-error`입니다.
2. 경로가 디렉터리가 아니면 1 `not-a-stellar-project`이고, 진단 코드는 `not-a-directory`입니다.
3. root를 검증합니다(`stellar_project::root::validate_project_root`).
   - 같은 경로에 일반 파일 `stellar.yaml`이 없으면 진단 `missing-manifest`입니다.
   - `stellar.yaml`이 있으면 `.git`을 봅니다. 디렉터리면 `git_link`가 `"directory"`, 파일(gitfile)이면
     `"gitfile"`입니다. 둘 다 아니면 진단 `missing-git`입니다.
   - `.git`이 있는지만 확인합니다. `.git`의 내용은 검사하지 않고 Git도 실행하지 않습니다.
   - 실패하면 종료 코드는 1, `error.code`는 `not-a-stellar-project`입니다.
4. `stellar.yaml`을 최대 1 MiB(`MAX_INPUT_BYTES` = 1024 × 1024 bytes)까지만 읽습니다.
   - 파일 크기가 상한을 넘으면 내용을 읽기 전에 거부합니다. 65 `manifest-syntax`, 진단
     `input-too-large`입니다.
   - 열기·읽기에 실패하면 `manifest-read`이고, 종료 코드는 io 오류 종류로 정합니다(권한 없음이면 77).
   - 내용이 UTF-8이 아니면 `manifest-read`이고 종료 코드는 65입니다(io 오류 종류 `InvalidData`).
5. 제한 YAML loader로 parse합니다. 실패하면 65 `manifest-syntax`입니다. 진단 코드는 loader 오류
   종류이고, 메시지는 `<행>행 <열>열: <설명>` 형식입니다. 오류 종류는 `anchor`, `alias`,
   `custom-tag`, `merge-key`, `non-scalar-key`, `duplicate-key`, `multiple-documents`,
   `empty-document`, `parse`, `input-too-large`, `non-printable`, `nesting-too-deep`,
   `lookahead-too-large`입니다.
6. 매니페스트 스키마를 검증합니다. 실패하면 65 `manifest-schema`, 진단 코드 `schema`입니다. 필수
   최상위 필드는 `schema_version`, `project`, `target_profiles`, `dependencies`이고, 알 수 없는 키는
   거부합니다.

**성공 결과**

| key | 값 |
|---|---|
| `manifest_schema_version` | `stellar.yaml`의 `schema_version` |
| `project` | `{"id", "name"}` |
| `git_link` | `"directory"` 또는 `"gitfile"` |
| `target_profiles` | `{"id", "platform", "minimum_os", "baseline"}` 배열. 값이 없는 선택 필드는 `null` |
| `dependencies` | `{"name", "source", "version"}` 배열 |

stdout JSON에는 입력 경로를 넣지 않습니다. 의존성의 `repository` URL과 `library_id`도 출력하지
않습니다. 입력 경로는 stderr 진단에만 나타납니다.

## 지정 View의 정적 접근성 HTML

```sh
# 저장된 문서·View ID는 이 조회 결과의 documents[].pages[].views[]에서 얻습니다.
stellar query /path/to/project project

stellar export-accessibility /path/to/project DOCUMENT_UUID VIEW_UUID > /tmp/view-result.json
# 성공한 JSON 결과에서 HTML만 꺼냅니다. 출력 파일은 원본 프로젝트 밖에 둡니다.
jq -je 'select(.ok == true) | .result.html' /tmp/view-result.json > /tmp/view.html
```

`export-accessibility`는 **정적 접근성 HTML 조각**을 제공합니다. `render/export` 전체, 디자인의 시각적
재현, 실행 가능한 웹 앱 내보내기가 아닙니다. 입력은 저장된 generation이고 앱의 미저장 편집은 읽지 않습니다.
명시한 문서의 `View.root_frame` 하위만 OrderKey·StableId 순으로 순회합니다. 다른 View는 결과에 포함하지
않고 `view-not-selected` 진단에 ID를 남깁니다. View 이름이나 node 이름으로 의미를 추정하지 않습니다.

| 입력 | 현재 출력·처리 |
|---|---|
| Frame | generic `div`; 이름으로 button·dialog·label을 만들지 않음 |
| Fragment | 자체 element 없이 자식을 제자리에서 펼침 |
| Text | 원문과 줄바꿈을 보존하는 `span`을 출력합니다. CR은 `&#13;`으로 기록하며, 선택한 View에 U+0000이 있으면 해당 노드에 `unsupported-text-character` 오류를 반환합니다. |
| visible=false | 자신·하위 접근성 제외, 펼쳐진 subtree에 `aria-hidden`; 시각적으로 숨기는 CSS는 없음 |
| Shape·Path·Image | `unsupported-node` 오류; 선택 View에 있으면 HTML 제공 안 함(숨김도 동일) |
| layout·크기·transform·clipping·paint·텍스트 스타일 | `visuals-not-exported`; CSS 생성·측정·기하 재현 없음 |
| layout 참조·modifier | 해당 node ID를 포함한 별도 누락 진단 |
| control·binding·상태·token·Variant·Context·Motion | 저장 모델과 연결되지 않아 추정하지 않음; `static-accessibility-only` 진단 |
| 프로젝트 target_profiles | `profiles-not-applied`; profile·registry pin·capability를 가정하지 않음 |
| 문서 manifest/tree 이외 chunk | `chunk-not-exported`; kind·개수 진단, 내용을 임의로 해석하지 않음 |

기존 접근성 생성기 → snapshot → codegen 접근성 전용 `IrTree` → `web/aria/map_accessibility`와 정적
검사를 재사용합니다. 전체 `CodegenSources`가 요구하는 LayoutInput·profile capabilities·상태·token을
빈 가상 값으로 채우지 않습니다. 매핑 노트나 정적 검사 오류가 있으면 실패합니다. 정적 검사 통과는
브라우저·스크린리더·색 대비·키보드 상호작용 전체 적합성을 보증하지 않습니다.

성공 시 `result`는 `revision`(generation, manifest hash, token), `document`, `view`, `root_frame`,
`format: "static-accessibility-html-fragment"`, `html`을 담습니다. 진단은 envelope의 `diagnostics`와
stderr에 함께 나옵니다. 같은 snapshot·문서·View는 같은 HTML·진단 순서를 냅니다. 정상 실행 후에도
선택 범위·표현 한계 경고가 있으므로 stderr가 비어 있을 것으로 가정하지 않습니다.

| 상황 | 종료 코드·오류 |
|---|---|
| 성공(범위·표현 한계 경고 포함) | 0 |
| 미지원 node, 매핑 손실, 정적 검사 오류 | 1, `accessibility-export-failed`; revision·진단과 `html: null` |
| 인자 부족·추가, 잘못된 UUID, 알 수 없는 옵션 | 64, stdout 없음 |
| 저장 전 프로젝트·없는 문서·없는 View | 66, `unsaved-project`·`unknown-document`·`unknown-view` |
| 손상된 문서 chunk·지원하지 않는 generation schema | 65, `document`·`unsupported-schema` |
| 프로젝트 열기·출력 I/O 오류 | 기존 CLI 오류 분류 사용 |

CLI 자체는 출력 파일을 만들지 않고 프로젝트·Git을 쓰지 않습니다. 셸 리다이렉션의 출력 위치는 사용자가
정합니다. 실제 두 View 저장 fixture와 CLI subprocess 테스트는 HTML golden, 선택 범위, 오류 경로,
반복 출력, 원본 파일·Git index·HEAD·objects 무변경을 검사합니다. text-source apply는 이 명령과 별개입니다.

## 지정 View의 고정 Frame HTML+CSS

```sh
stellar export-web /path/to/project DOCUMENT_UUID VIEW_UUID > /tmp/view-result.json
# 종료 코드가 0일 때만 HTML을 추출합니다.
jq -jer '.result.html' /tmp/view-result.json > /tmp/view.html
```

저장된 문서의 선택 View를 CSS가 내장된 독립 HTML 문서로 출력합니다. 고정 크기 Free Frame·Fragment의
크기·affine·색공간/RGBA·solid 테두리·개별 radius·clipping을 투영합니다. root 캔버스 이동은 생략하고
자식은 부모 border를 보정한 좌표를 사용합니다. Fragment를 펼치며 숨김과 OrderKey paint 순서를 유지합니다.
미저장 active document나 앱의 활성 창은 읽지 않으므로 저장 후 문서 ID·View ID를 명시합니다.

Text·layout 참조·modifier·Path clip·해석할 수 없는 record/profile·지원 숫자 범위 밖 입력은 node ID
진단과 종료 코드 1, `html: null`로 실패합니다. 색 선언은 보존하지만 부분 투명 색의 브라우저 합성은
별도 경고를 냅니다. `result.format`은 `fixed-free-frame-html`입니다. 다른 출력·오류 계약은
[명세](../spec/web-frame-export.md)에 있습니다. 일반 render/export와 전체 코드 생성 완료는 아닙니다.

실제 Chrome headless 검증은 명세의 명령으로 별도 실행합니다. 일반 테스트의 ignored 결과만으로
브라우저 적합성을 보고하지 않습니다. 기존 `export-accessibility`의 Text·ARIA 경로는 유지됩니다.

## 출력 계약

출력 계약은 `tools/stellar-cli/src/output.rs`가 정의하고, JSON Schema는
`tools/stellar-cli/schema/cli-result.v1.schema.json`입니다.

### stdout과 stderr

- **stdout**: 명령이 실행되면(성공·실패 모두) 결과 envelope JSON 문서 정확히 한 개와 개행 하나만
  씁니다. 예외는 도움말뿐입니다.
- **JSON 표기**: writer는 공백 없는 결정론적 표기를 만들고, 문자열 안의 개행도 이스케이프합니다.
  그래서 결과는 항상 한 줄입니다.
- **사용법 오류(64)**: 명령이 실행되지 않은 상태라 stdout에는 아무것도 쓰지 않습니다. stderr에
  메시지와 `` `stellar --help`로 사용법을 확인하세요. `` 안내를 씁니다.
- **stderr**: 사람이 읽는 진단입니다. 형식은 계약이 아니며 입력 경로 같은 지역 정보를 담을 수
  있습니다. 현재 형식은 다음과 같습니다.
  - 실패: `stellar <명령>: 오류(<error.code>): <error.message> [<입력 경로>]`
  - 진단마다 한 줄: `stellar <명령>: <severity>(<code>): <message>`
  - 사용법 오류: `stellar: <메시지>` 다음 줄에 안내
- **schema 식별자**: `stellar-cli/result`는 실행 이름과 무관한 계약입니다. 실행 이름을 `stellar`로
  정한 XXIV.4.1에서도 바꾸지 않았습니다.

### envelope v1

모든 key가 항상 이 순서로 나옵니다. schema는 다른 key를 허용하지 않습니다(`additionalProperties: false`).

| key | 값 |
|---|---|
| `schema` | `"stellar-cli/result"` |
| `schema_version` | `1`. 호환되지 않는 변경에서만 올립니다 |
| `command` | 실행한 명령 이름(`"version"`, `"inspect"`). 별칭으로 실행해도 정규 이름입니다 |
| `ok` | 성공이면 `true` |
| `exit_code` | 프로세스 종료 코드와 같은 값 |
| `exit_name` | 종료 코드의 고정 이름(아래 표) |
| `result` | 명령별 결과 object. 실패하면 `null` |
| `error` | `{"code", "message"}`. 성공하면 `null` |
| `diagnostics` | `{"severity", "code", "message"}` 배열. 비어 있을 수 있습니다 |

- `severity`는 `"error"`, `"warning"`, `"info"` 중 하나입니다.
- `error.code`와 `diagnostics[].code`는 고정된 kebab-case 코드입니다. `message`는 사람이 읽는
  설명이며 경로를 담지 않습니다.
- 현재 `version`과 `inspect`는 성공할 때 진단을 만들지 않습니다.

## 종료 코드

0은 성공, 1은 도메인 실패이고, 64 이상은 BSD `sysexits.h` 값을 그대로 씁니다
(`tools/stellar-cli/src/exit.rs`). 외부 스크립트와 CI가 의존하는 공개 계약이므로 값을 바꾸거나 다른
뜻으로 재사용하지 않습니다. `exit_codes_are_golden` 테스트가 값과 이름을 고정합니다.

| 코드 | `exit_name` | 도움말 설명 | 현재 명령에서 나오는 경우 |
|---|---|---|---|
| 0 | `ok` | 성공 | `help`, `version`, `inspect` 성공 |
| 1 | `failure` | 도메인 실패(검증 진단 등) | `inspect`: 디렉터리가 아님, `stellar.yaml` 없음, `.git` 없음 |
| 64 | `usage` | 사용법 오류 | 명령 없음, 알 수 없는 명령·옵션, 인자 개수 오류, UTF-8이 아닌 인자 |
| 65 | `dataerr` | 입력 데이터 형식 오류 | `inspect`: YAML 문법·제한 위반, 스키마 오류, UTF-8이 아닌 `stellar.yaml` |
| 66 | `noinput` | 입력 없음·읽을 수 없음 | `inspect`: 경로가 없음 |
| 69 | `unavailable` | 필요한 기능 사용 불가 | 현재 쓰지 않음 |
| 70 | `software` | 내부 오류 | 현재 명령 경로에서는 쓰지 않음. 실행 파일이 범위 밖 코드를 종료 byte로 바꿀 때의 대체 값 |
| 73 | `cantcreat` | 출력 생성 불가 | io 오류 종류가 `AlreadyExists`일 때만 |
| 74 | `ioerr` | 입출력 오류 | stdout 쓰기 실패, 분류되지 않은 io 오류 |
| 75 | `tempfail` | 일시적 실패(재시도 가능) | io 오류 종류가 `Interrupted`, `WouldBlock`, `TimedOut`일 때만 |
| 76 | `protocol` | 프로토콜 오류 | 현재 쓰지 않음 |
| 77 | `noperm` | 권한 부족 | `inspect`: 경로나 `stellar.yaml`에 접근 권한 없음 |
| 78 | `config` | 설정 오류 | 현재 쓰지 않음 |

io 오류 종류는 `ExitStatus::from_io_kind`가 다음처럼 분류합니다.

| `std::io::ErrorKind` | 종료 코드 |
|---|---|
| `NotFound` | 66 |
| `PermissionDenied` | 77 |
| `AlreadyExists` | 73 |
| `Interrupted`, `WouldBlock`, `TimedOut` | 75 |
| `InvalidData` | 65 |
| 그 밖 | 74 |

## inspect 오류 분류

| 상황 | 종료 코드 | `error.code` | `diagnostics[].code` |
|---|---|---|---|
| 경로가 없음 | 66 | `no-input` | 없음 |
| 경로에 접근 권한 없음 | 77 | `no-permission` | 없음 |
| 경로 metadata를 읽는 그 밖의 io 오류 | io 오류 종류에 따름 | `io-error` | 없음 |
| 경로가 디렉터리가 아님 | 1 | `not-a-stellar-project` | `not-a-directory` |
| `stellar.yaml`이 없음 | 1 | `not-a-stellar-project` | `missing-manifest` |
| `.git`이 없음 | 1 | `not-a-stellar-project` | `missing-git` |
| `stellar.yaml` 열기·읽기 실패 | io 오류 종류에 따름 | `manifest-read` | 없음 |
| `stellar.yaml`이 UTF-8이 아님 | 65 | `manifest-read` | 없음 |
| `stellar.yaml`이 1 MiB를 넘음 | 65 | `manifest-syntax` | `input-too-large` |
| YAML 문법 오류·제한 위반 | 65 | `manifest-syntax` | loader 오류 종류(예: `anchor`) |
| 스키마 오류 | 65 | `manifest-schema` | `schema` |

`tools/stellar-cli/tests/fixtures/results/`에 이 표의 주요 경우에 대한 golden 출력이 있고,
`tests/output_fixtures.rs`가 stdout을 byte 단위로 비교합니다.

## 실행 예시

아래 출력은 커밋 `993d7d0`, macOS에서 실제로 실행해 얻은 결과입니다. `$ stellar ...`는
`cargo run -q --manifest-path <저장소>/Cargo.toml -p stellar-cli -- ...`로 실행했습니다. 각 예시는
stdout, stderr, 종료 코드를 `[stdout]`, `[stderr]`, `[exit]`로 나눠 적었습니다.

### 연습용 프로젝트 준비

```sh
mkdir -p cli-demo/demo-project && cd cli-demo
(cd demo-project && git init -q)
cat > demo-project/stellar.yaml <<'EOF'
schema_version: 1
project:
  id: 0195c530-3acd-7bc1-8414-c213f61cb965
  name: Demo Project
target_profiles:
  - id: apple-desktop
    platform: apple
    minimum_os: "26.0"
dependencies: {}
EOF
```

`inspect`는 `.git`이 있는지만 확인하므로, 실패 사례용 디렉터리는 `mkdir -p <디렉터리>/.git`으로
만들었습니다.

### `version`

```text
$ stellar version
[stdout]
{"schema":"stellar-cli/result","schema_version":1,"command":"version","ok":true,"exit_code":0,"exit_name":"ok","result":{"name":"stellar-cli","version":"0.0.0","output_schema_version":1},"error":null,"diagnostics":[]}
[stderr]
[exit] 0
```

`stellar -V`도 같은 결과를 냈습니다.

### `inspect` 성공

```text
$ stellar inspect demo-project
[stdout]
{"schema":"stellar-cli/result","schema_version":1,"command":"inspect","ok":true,"exit_code":0,"exit_name":"ok","result":{"manifest_schema_version":1,"project":{"id":"0195c530-3acd-7bc1-8414-c213f61cb965","name":"Demo Project"},"git_link":"directory","target_profiles":[{"id":"apple-desktop","platform":"apple","minimum_os":"26.0","baseline":null}],"dependencies":[]},"error":null,"diagnostics":[]}
[stderr]
[exit] 0
```

실행 전후로 `demo-project` 아래 파일의 SHA-256과 `ls -laR` 출력이 같았습니다.

의존성이 있는 매니페스트입니다. `with-dep/stellar.yaml`에는
`repository: https://example.com/design-system.git`과 `library_id`가 있지만 출력에는 나오지
않습니다.

```yaml
schema_version: 1
project:
  id: 0195c530-3acd-7bc1-8414-c213f61cb965
  name: Demo Project
target_profiles:
  - id: web-modern
    platform: web
    baseline: modern
dependencies:
  design-system:
    source: git
    library_id: 0195c530-3acd-7bc1-8414-c213f61cb966
    repository: https://example.com/design-system.git
    version: v2.4.0
```

```text
$ stellar inspect with-dep | jq .
{
  "schema": "stellar-cli/result",
  "schema_version": 1,
  "command": "inspect",
  "ok": true,
  "exit_code": 0,
  "exit_name": "ok",
  "result": {
    "manifest_schema_version": 1,
    "project": {
      "id": "0195c530-3acd-7bc1-8414-c213f61cb965",
      "name": "Demo Project"
    },
    "git_link": "directory",
    "target_profiles": [
      {
        "id": "web-modern",
        "platform": "web",
        "minimum_os": null,
        "baseline": "modern"
      }
    ],
    "dependencies": [
      {
        "name": "design-system",
        "source": "git",
        "version": "v2.4.0"
      }
    ]
  },
  "error": null,
  "diagnostics": []
}
```

`.git`이 gitfile(`gitdir: ../demo-project/.git` 한 줄)인 프로젝트입니다.

```text
$ stellar inspect gitfile-project | jq -r .result.git_link
gitfile
```

### `inspect` 도메인 실패(1)

```text
$ stellar inspect plain-dir
[stdout]
{"schema":"stellar-cli/result","schema_version":1,"command":"inspect","ok":false,"exit_code":1,"exit_name":"failure","result":null,"error":{"code":"not-a-stellar-project","message":"Stellar 프로젝트 root가 아닙니다"},"diagnostics":[{"severity":"error","code":"missing-manifest","message":"같은 경로에 stellar.yaml이 없습니다"}]}
[stderr]
stellar inspect: 오류(not-a-stellar-project): Stellar 프로젝트 root가 아닙니다 [plain-dir]
stellar inspect: error(missing-manifest): 같은 경로에 stellar.yaml이 없습니다
[exit] 1
```

```text
$ stellar inspect no-git
[stdout]
{"schema":"stellar-cli/result","schema_version":1,"command":"inspect","ok":false,"exit_code":1,"exit_name":"failure","result":null,"error":{"code":"not-a-stellar-project","message":"Stellar 프로젝트 root가 아닙니다"},"diagnostics":[{"severity":"error","code":"missing-git","message":"같은 경로에 .git(디렉터리 또는 gitfile)이 없습니다"}]}
[stderr]
stellar inspect: 오류(not-a-stellar-project): Stellar 프로젝트 root가 아닙니다 [no-git]
stellar inspect: error(missing-git): 같은 경로에 .git(디렉터리 또는 gitfile)이 없습니다
[exit] 1
```

```text
$ stellar inspect demo-project/stellar.yaml
[stdout]
{"schema":"stellar-cli/result","schema_version":1,"command":"inspect","ok":false,"exit_code":1,"exit_name":"failure","result":null,"error":{"code":"not-a-stellar-project","message":"Stellar 프로젝트 root가 아닙니다"},"diagnostics":[{"severity":"error","code":"not-a-directory","message":"입력 경로가 디렉터리가 아닙니다"}]}
[stderr]
stellar inspect: 오류(not-a-stellar-project): Stellar 프로젝트 root가 아닙니다 [demo-project/stellar.yaml]
stellar inspect: error(not-a-directory): 입력 경로가 디렉터리가 아닙니다
[exit] 1
```

### `inspect` 입력 오류(65, 66)

`broken-yaml/stellar.yaml`은 anchor와 alias를 쓴 두 줄(`schema_version: &a 1`, `project: *a`)입니다.

```text
$ stellar inspect broken-yaml
[stdout]
{"schema":"stellar-cli/result","schema_version":1,"command":"inspect","ok":false,"exit_code":65,"exit_name":"dataerr","result":null,"error":{"code":"manifest-syntax","message":"stellar.yaml YAML 문법 오류"},"diagnostics":[{"severity":"error","code":"anchor","message":"1행 19열: anchor는 허용되지 않습니다"}]}
[stderr]
stellar inspect: 오류(manifest-syntax): stellar.yaml YAML 문법 오류 [broken-yaml]
stellar inspect: error(anchor): 1행 19열: anchor는 허용되지 않습니다
[exit] 65
```

`bad-schema/stellar.yaml`은 올바른 매니페스트에 최상위 키 `extra: 1`을 더한 파일입니다.

```text
$ stellar inspect bad-schema
[stdout]
{"schema":"stellar-cli/result","schema_version":1,"command":"inspect","ok":false,"exit_code":65,"exit_name":"dataerr","result":null,"error":{"code":"manifest-schema","message":"stellar.yaml 스키마 오류"},"diagnostics":[{"severity":"error","code":"schema","message":"알 수 없는 키: <root>.extra"}]}
[stderr]
stellar inspect: 오류(manifest-schema): stellar.yaml 스키마 오류 [bad-schema]
stellar inspect: error(schema): 알 수 없는 키: <root>.extra
[exit] 65
```

```text
$ stellar inspect does-not-exist
[stdout]
{"schema":"stellar-cli/result","schema_version":1,"command":"inspect","ok":false,"exit_code":66,"exit_name":"noinput","result":null,"error":{"code":"no-input","message":"입력 경로가 없습니다"},"diagnostics":[]}
[stderr]
stellar inspect: 오류(no-input): 입력 경로가 없습니다 [does-not-exist]
[exit] 66
```

`stellar inspect -`도 `-`라는 경로가 없어서 같은 66 결과를 냈습니다(stderr 끝은 `[-]`).

### 사용법 오류(64)

사용법 오류는 stdout에 아무것도 쓰지 않습니다.

```text
$ stellar inspect
[stdout]
[stderr]
stellar: inspect: inspect에는 프로젝트 경로가 필요합니다
`stellar --help`로 사용법을 확인하세요.
[exit] 64
```

```text
$ stellar inspect --json
[stdout]
[stderr]
stellar: inspect: 알 수 없는 옵션: --json
`stellar --help`로 사용법을 확인하세요.
[exit] 64
```

```text
$ stellar inspect --json demo-project
[stdout]
[stderr]
stellar: inspect: inspect는 경로 하나만 받습니다: demo-project
`stellar --help`로 사용법을 확인하세요.
[exit] 64
```

```text
$ stellar version extra
[stdout]
[stderr]
stellar: version: version은 인자를 받지 않습니다: extra
`stellar --help`로 사용법을 확인하세요.
[exit] 64
```

```text
$ stellar frobnicate
[stdout]
[stderr]
stellar: 알 수 없는 명령: frobnicate
`stellar --help`로 사용법을 확인하세요.
[exit] 64
```

```text
$ stellar
[stdout]
[stderr]
stellar: 명령이 필요합니다.
`stellar --help`로 사용법을 확인하세요.
[exit] 64
```

## 스크립트에서 쓰는 법

- 종료 코드를 먼저 확인합니다. 64면 stdout이 비어 있으므로 JSON을 parse하지 않습니다.
- 64가 아니면 stdout 한 줄이 envelope이고, `exit_code`는 프로세스 종료 코드와 같습니다.
- 분기에는 `error.code`와 `diagnostics[].code`를 씁니다. `message`는 사람용 설명이고, stderr 형식은
  계약이 아닙니다.
- stdout만 받으려면 stderr를 버립니다. 다음은 실제로 실행한 결과입니다.

```text
$ stellar inspect demo-project | jq -c .result.project
{"id":"0195c530-3acd-7bc1-8414-c213f61cb965","name":"Demo Project"}

$ stellar inspect plain-dir 2>/dev/null | jq -r '.exit_name, .error.code, .diagnostics[0].code'
failure
not-a-stellar-project
missing-manifest
```

## 계획: ADR-0005 L1

이 절은 ADR-0005 §1, §2, §7, §8과 Phase XXIV의 범위를 설명합니다. 조회·검증·apply·CI 등
현재 구현된 명령은 위 명령 표를 따릅니다. 일반 render/export·merge driver·설치는 별도 범위이며,
세부와 수치는 각 WBS 구현 작업에서 정합니다(ADR-0005 §9).

### 계층 경계

- L1(`stellar`, `stellar-ops`)에는 AI, MCP, 모델, 에이전트 관련 명령과 의존이 없습니다. L2
  (`stellar-mcp`)는 L1 library를 in-process로 호출하고, 아래 계층은 위 계층을 알지 못합니다.
- `stellar`는 실행 중 앱의 얇은 클라이언트가 아닙니다. 파일이 원본이고 CI와 merge driver가 앱 없이
  돌아야 하므로, 실행 중 앱은 프로젝트 파일을 쓰는 여러 writer 중 하나입니다.
- XXIV.4.7은 `stellar`에 AI·MCP·모델·에이전트 명령과 해당 crate로의 직접·전이 의존이 없음을 검사하는
  inventory test를 추가합니다.

### 명령 로드맵과 범위

| 명령 | 계획 내용 | WBS |
|---|---|---|
| `validate`, `semantic-diff`, `semantic-merge`, `migrate` | fixture corpus를 통과하는 기존 계획 명령 | XXIV.1.3 |
| `render`/`export` | Phase XX 이후 일반 내보내기. 현재 `export-accessibility`·`export-web`은 위에서 정한 좁은 경로만 제공 | XXIV.1.4 |
| `inspect` 확장, `query` | 프로젝트 구조, 문서 트리(cursor 분할), 노드 속성, 토큰, 컴포넌트를 revision과 함께 JSON으로 출력. 기존 `inspect` 결과 필드는 호환을 유지하고 파일을 쓰지 않음 | XXIV.4.3 |
| `apply` | 파일이나 표준 입력으로 CommandTransaction JSON과 base revision을 받아 closed enum으로 검증하고 세대 전환 저장으로 원자적으로 기록. generation metadata에 writer 종류 headless CLI를 남김. `--dry-run`은 semantic diff만 출력 | XXIV.4.4 |
| Git merge driver 모드 | base, ours, theirs, result 경로를 받아 Semantic Merge를 비대화형으로 실행. 해결되지 않은 semantic conflict가 있으면 원본을 보존하고 0이 아닌 코드로 끝남. `.gitattributes`·Git config 등록은 사용자 승인 명령으로만 하고 전역 config를 자동으로 바꾸지 않음 | XXIV.2.1\~XXIV.2.3 |
| CI 모드 | lockfile resolve check, project validation, migration-required check. semantic diff summary artifact는 프로젝트 텍스트·이름·경로 redaction을 기본으로 함. performance replay CLI | XXIV.3.1\~XXIV.3.3 |

출력 원칙은 지금과 같습니다. 표준 출력에는 JSON 결과만, 표준 오류에는 진단만 씁니다.

### crate 구조

- `stellar-ops`가 headless 공용 library이며 `stellar-cli`는 ops에 직접 의존합니다. 정적 접근성
  HTML 연결은 ops에서 codegen·accessibility·target-profile을 재사용합니다(ADR-0049 후속 결정).
  local-ipc·agent-protocol·mcp-core·agent-host·ffi로의 금지 경계는 유지합니다.
- ADR-0020은 `stellar` JSON 입출력에 쓸 `serde`·`serde_json`을 승인했고, 도입 예정 행에 XXIV.4.3과
  XXIV.4.4가 있습니다. 현재 JSON은 외부 crate 없는 자체 모듈(`src/json.rs`)이며, `stellar-mcp`의 같은
  모듈과 본문이 일치하는지 `json_copy_sync` 테스트가 검사합니다.

### 쓰기와 실행 중 앱과의 공존

- 세대 전환 저장 직전에 교차 프로세스 저장 잠금을 잡고, 디스크 generation과 base generation을
  비교합니다. 잠금은 프로젝트 폴더와 Git 밖의 OS 사용자 데이터에 둡니다. 잠금이 사용 중이면 일시적
  실패(75)로 끝납니다(XXIV.4.5).
- 저장한 generation에는 writer 종류(앱 창, headless CLI)를 기록합니다.
- merge driver 모드는 Git이 넘긴 임시 경로만 쓰므로 저장 잠금과 앱 공존 검사 대상이 아닙니다.
- 앱이 같은 프로젝트를 열고 있어도 L1 쓰기를 막지 않습니다. 앱은 writer 종류가 앱 창이 아닌
  generation을 두 선택지 정책이 아니라 semantic reconciliation 경로로 처리합니다(XXIV.4.6).
- 변경이 에이전트 lease와 겹치면 그 에이전트 세션을 끝냅니다(자동 rollback 없음). L1 쓰기는 에이전트
  출처 표시 대상이 아닙니다.

### 종료 코드의 계획 의미

종료 코드 값과 이름은 지금 표 그대로이고, 표는 XXIV.1.2가 소유합니다. ADR-0005 §2 초안은 앞으로의
명령에서 다음 뜻을 더합니다.

| 코드 | 계획 의미 |
|---|---|
| 1 | 검증 진단 있음, 미해결 semantic conflict, 거부된 command |
| 65 | 입력 schema 오류, base revision 불일치 |
| 66 | 프로젝트를 찾거나 열 수 없음 |
| 69 | 요청한 동작에 필요한 Stellar 구성 요소를 쓸 수 없음 |
| 73 | 저장 실패 |
| 75 | 저장 잠금 사용 중 |
| 76 | 내부 protocol 버전 불일치 |

Git merge driver 모드에서는 0이 아닌 코드를 모두 충돌로 취급한다는 점을 문서화합니다. 128 이상은
신호용으로 남겨 둡니다.

### 배포와 설치

- macOS 앱 번들에 `stellar`를 helper로 넣고, helper와 앱을 Apple 공식 코드 서명·공증 문서에 따라
  서명·공증합니다. helper에는 앱 entitlement를 주지 않습니다(XXIV.5.1).
- 앱이 없는 macOS headless·CI 머신용으로 `stellar` 단독 서명·공증 배포물을 만듭니다. `stellar-mcp`는
  넣지 않습니다(XXIV.5.2).
- 앱 첫 실행 때 명령을 사용자 셸 PATH에 설치하는 화면을 제공하고, 사용자가 명시적으로 동의하기 전에는
  어떤 파일도 만들거나 바꾸지 않습니다(XXIV.5.3\~XXIV.5.6).
- 설치 이름은 기본 `stellar`입니다. PATH에서 `stellar`가 Stellar가 설치한 자기 링크가 아닌 다른
  프로그램으로 먼저 해석되면 그 파일을 덮어쓰지 않고 `stellar-cli`라는 이름으로 설치합니다. 그 이름도
  쓰이고 있으면 설치하지 않고 알립니다.
- 설치 상태는 앱 사용자 설정(OS 사용자 데이터)에만 두고 프로젝트, Git, 텔레메트리에는 남기지 않습니다.
- Windows 배포는 ADR-0006에 따라 보류합니다.

## 관련 코드와 문서

- `tools/stellar-cli/src/cli.rs`: 명령 registry, 도움말, 인자 해석, 출력 dispatch
- `tools/stellar-cli/src/commands.rs`: `version`, `inspect` 구현
- `tools/stellar-cli/src/output.rs`: envelope, stdout·stderr 역할
- `tools/stellar-cli/src/exit.rs`: 종료 코드
- `tools/stellar-cli/src/json.rs`: 엄격한 JSON reader·writer
- `tools/stellar-cli/schema/cli-result.v1.schema.json`: envelope JSON Schema
- `tools/stellar-cli/tests/`: 도움말 snapshot, golden 결과 fixture, JSON 사본 일치 검사
- [ADR-0005: 3계층 에이전트 도구 구조](../adr/ADR-0005-agent-tool-layers.md)
- [ADR-0020: 에이전트 도구 계층 외부 의존성 승인](../adr/ADR-0020-agent-tool-dependencies.md)
- [구현 계획서](../PROJECT_STELLAR_IMPLEMENTATION_PLAN.md) Phase XXIV
- [crate 지도](crate-map.md)
