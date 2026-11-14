# Stellar 파일 형식 golden corpus

- 문서 종류: conformance corpus 설명서. corpus 자체는 [`manifest.yaml`](manifest.yaml)과 `binary/`·`yaml/`
  아래 파일입니다.
- WBS: XXVIII.2.1
- 기준 commit: `24728e3`. 모든 기대 결과는 이 commit의 구현으로 실행해 얻었습니다.
- 공개 상태: **공개 전**. 라이선스가 정해지지 않았습니다(§6).
- 관련 명세: [`../stellar-format.md`](../stellar-format.md)(형식 명세),
  [`../stellar-semantic-model.md`](../stellar-semantic-model.md)(의미 레코드 §2),
  [`../stellar-format-security.md`](../stellar-format-security.md)(malformed 입력 동작).

## 1. 목적과 범위

reader·writer 구현이 Stellar 프로젝트 파일 형식을 같은 방식으로 읽고 거부하는지 확인하기 위한 입력 모음입니다.
입력마다 대상 형식, 유효 여부, 기대 결과, canonical 여부, 근거 명세 절을 manifest에 적습니다.

포함하는 형식:

| 계층 | 대상(`target`) |
|---|---|
| 저장 형식 binary | `chunk-file`, `envelope`, `pointer`, `manifest`, `view-tree`, `current-pointer` |
| 문서 의미 레코드 binary | `document-definition`, `document-manifest`, `subtree`, `modifier-stack` |
| YAML | `restricted-yaml`, `stellar-yaml`, `stellar-lock-yaml` |

포함하지 않는 것:

- Semantic Snapshot·diff·merge 교환 bytes(`SSNP`·`SDIF`·`SMIN`·`SMRS`). semantic conformance corpus
  (XXVIII.2.3) 범위입니다.
- conformance runner와 기대 결과 schema의 공식 정의(XXVIII.2.2). 이 corpus는 저장소 밖 검증 도구로
  확인했습니다(§7).
- node payload 종류별 내부 bytes만 다루는 입력(Frame·Shape·Path·Text·Image). subtree 입력 안에 Frame과
  Fragment payload가 들어 있습니다.

## 2. 내용 정책

- 모든 입력은 **synthetic**입니다(`content: synthetic`). 실제 사용자 프로젝트·디자인 내용은 없습니다.
  식별자는 고정 seed로 만든 UUIDv7 형태 값, 이름은 `Doc`·`Page`·`Home`·`root`·`group`·`Demo`·
  `Fixture Project` 같은 예시 값, URL은 `example.com` 또는 `example` 경로입니다.
- 입력은 두 출처입니다.
  - `origin: copied`: 저장소의 기존 seed corpus·fixture를 **byte 그대로** 복사했습니다. `source`에 원본 경로가
    있고, 원본과 bytes가 같아야 합니다.
  - `origin: generated`: 이 작업에서 기준 commit의 공식 encoder로 만든 bytes이거나, 공식 encoder 출력의
    지정 위치를 바꾼 bytes입니다. 바꾼 내용은 `description`에 있습니다.
- 기존 seed 중 corpus에 넣지 않은 파일은 manifest `excluded`에 이유와 함께 있습니다. 저장소 storage seed의
  `stream_*.hex`는 fuzz harness 전용 op stream 입력이며 파일 형식이 아니므로 제외했습니다.

## 3. 파일 형식

### 3.1 binary 입력 (`binary/*.hex`)

- UTF-8 텍스트입니다. `#`로 시작하는 줄(앞 공백 허용)은 주석입니다.
- 나머지 줄의 공백을 모두 지운 문자를 16진수 두 자리씩 byte로 읽습니다. 대소문자는 소문자로 씁니다.
  자리 수는 짝수여야 합니다.
- 복사한 storage seed에는 저장소 fuzz harness용 주석 `# target:`·`# expect:`가 남아 있습니다. 이 corpus에서
  대상과 기대 결과의 기준은 manifest입니다(두 값은 서로 맞습니다. §7).

### 3.2 YAML 입력 (`yaml/*.yaml`)

- 파일 bytes 그대로가 입력입니다. 선행 BOM과 tab이 있는 입력도 의도한 것입니다.

## 4. manifest 형식

`manifest.yaml`은 제한 YAML(형식 명세 §14)이며 Stellar canonical writer로 기록했습니다.

| 키 | 뜻 |
|---|---|
| `schema_version` | manifest 형식 버전. 현재 `"1"` |
| `name` | corpus 이름 |
| `license.status`, `license.note` | 라이선스 상태(§6) |
| `content` | 입력 내용 종류. `synthetic` |
| `baseline_commit` | 기대 결과를 얻은 commit |
| `entries[]` | 입력 항목(아래 표) |
| `excluded[]` | 넣지 않은 seed(`source`, `reason`) |
| `related_fixtures[]` | corpus 밖에서 같은 형식을 고정하는 fixture(`path`, `check`, `spec`) |

`entries[]` 항목:

| 키 | 필수 | 뜻 |
|---|---|---|
| `id` | 예 | 유일한 항목 이름 |
| `file` | 예 | `manifest.yaml` 기준 입력 경로 |
| `target` | 예 | 대상 형식(§5) |
| `validity` | 예 | `valid` 또는 `invalid`. `expect`가 `ok`이면 `valid` |
| `expect` | 예 | `ok` 또는 `error:<오류>`(§5.2) |
| `canonical` | 대상에 따라 | `"true"`: 입력을 읽어 다시 쓰면 bytes가 같음, `"false"`: 읽을 수는 있지만 canonical 기록과 다름. `expect: ok`이고 대상에 writer가 있을 때만 있음 |
| `spec` | 예 | 근거 명세 절 |
| `description` | 예 | 입력 설명 |
| `origin` | 예 | `copied` 또는 `generated` |
| `source` | `copied`일 때 | 원본 경로(저장소 root 기준) |
| `sha256` | 예 | 입력 **파일** bytes의 content hash(`sha256:` + 소문자 hex 64자) |

같은 파일이 다른 대상으로 두 번 나올 수 있습니다(예: 제한 loader 단계와 `stellar.yaml` 스키마 단계).

## 5. 대상과 판정

### 5.1 대상

| target | 입력 | 판정 절차 | canonical 판정 | 근거 |
|---|---|---|---|---|
| `chunk-file` | `.starchunk` 파일 전체 | header를 읽고, 요청 hash·기대 kind를 header 값으로 두어 형식 명세 §6.4 읽기 검증 | header 재인코딩 ‖ payload = 입력 | 형식 §6.1, §6.4 |
| `envelope` | header 하나(입력 전체) | 형식 §6.1 header decode. 끝에 bytes가 남으면 거부 | 재인코딩 = 입력 | 형식 §6.1 |
| `pointer` | chunk pointer 하나(입력 전체) | 형식 §8.1 decode. 끝에 bytes가 남으면 거부 | 재인코딩 = 입력 | 형식 §8.1 |
| `manifest` | `.starmanifest` 파일 전체 | 형식 §10.1·§10.2 decode와 구조 검증 | 재인코딩 = 입력 | 형식 §10 |
| `view-tree` | ViewTree payload | 형식 §9.1 | 재인코딩 = 입력 | 형식 §9.1 |
| `current-pointer` | `design/current` 파일 내용 | 형식 §11 reader 규칙 | hash 문자열 ‖ LF = 입력 | 형식 §11 |
| `document-definition` | `DocumentDefinition` bytes | 의미 명세 §2.5 | 재인코딩 = 입력 | 의미 §2.5 |
| `document-manifest` | DocumentManifest payload | 형식 §9.2 | 재인코딩 = 입력 | 형식 §9.2 |
| `subtree` | subtree payload | 의미 §2.4(node 레코드 §2.2, payload 분기 §2.3, stack §2.6) | 재인코딩 = 입력 | 의미 §2.4 |
| `modifier-stack` | `ModifierStack` bytes | 의미 §2.6 | 재인코딩 = 입력 | 의미 §2.6 |
| `restricted-yaml` | YAML 텍스트 | 제한 loader만(형식 §14.1\~§14.3) | 없음 | 형식 §14.1\~§14.3 |
| `stellar-yaml` | YAML 텍스트 | 제한 loader → `stellar.yaml` 스키마 | canonical writer 출력 = 입력 | 형식 §14.5 |
| `stellar-lock-yaml` | YAML 텍스트 | 제한 loader → `stellar.lock.yaml` 스키마 | canonical writer 출력 = 입력 | 형식 §14.6 |

문서 의미 레코드 대상(`document-definition`, `subtree` 등)은 decoder 단계만 봅니다. 문서 validator(의미
§2.7)는 실행하지 않습니다.

### 5.2 오류 표기

- binary 대상과 YAML 스키마 단계: 기준 구현의 오류 값에서 variant 이름을 바깥부터 `/`로 이은 문자열입니다.
  `reason`·`path` 문자열이 있으면 그 variant 뒤에 `[...]`로 붙입니다. 예:
  `Decode/TrailingBytes`, `Malformed[trailing bytes]`,
  `Manifest/MissingField[dependencies.design-system.library_id]`. 스키마 단계 접두어는 `stellar-yaml`이
  `Manifest`, `stellar-lock-yaml`이 `Lock`입니다. `chunk-file`에서 header를 읽지 못하면 `Envelope/...`입니다.
- `restricted-yaml`: `<loader 오류 코드>@<line>:<col>`. `stellar-yaml`·`stellar-lock-yaml`에서 loader 단계가
  실패하면 `loader:<코드>@<line>:<col>`입니다. line·col은 기준 구현 `LoaderError`의 값 그대로입니다
  (§8 관찰 참고).
- 다른 구현은 이 이름을 자기 오류 체계로 대응시켜야 합니다. 이름 자체는 형식 계약이 아닙니다. 형식 계약은
  "거부한다"와 근거 절의 거부 조건입니다.

## 6. 라이선스

**정하지 않았습니다.** 저장소에 라이선스 파일이 없고, corpus에 붙일 라이선스(예: 코드와 같은 라이선스,
public-domain 계열 등)를 고르는 것은 product owner 결정입니다. 결정 전에는 이 corpus를 외부에 공개하면 안
됩니다. 결정되면 manifest `license`를 갱신하고 같은 디렉터리에 라이선스 본문을 둡니다.

## 7. 검증

기준 commit에서 저장소 밖 검증 도구(path 의존 scratch crate)로 다음을 확인했습니다. 저장소 안 runner는
XXVIII.2.2 범위입니다.

1. manifest를 Stellar 제한 YAML loader로 읽을 수 있음.
2. 모든 항목을 기준 구현으로 다시 실행해 `expect`·`canonical`이 같음.
3. 모든 항목의 `sha256`이 파일 bytes와 같고, `copied` 항목은 `source` 원본과 bytes가 같음.
4. `binary/`·`yaml/`의 모든 파일이 어떤 항목에 있고, 모든 항목의 파일이 있음.
5. 원본 seed 디렉터리의 모든 파일이 복사되었거나 `excluded`에 있음.
6. 복사한 storage seed의 `# expect:` 주석(`ok`·`err`)이 manifest `expect`와 맞음.

## 8. 관찰한 현재 구현 동작

corpus를 만들며 확인한 동작입니다. 명세와 다른 점은 해당 명세에서 다룹니다.

- `stellar.yaml` 의존성 entry는 `library_id`가 필수입니다(ADR-0021). `library_id` 없는 이전 형식 seed는
  `stellar-yaml-full`, 6필드 lock seed는 `stellar-lock-multi-version`으로 거부 항목입니다.
- 제한 loader의 parser 단계 오류(`parse`, `anchor`, `custom-tag`, `duplicate-key`, `multiple-documents` 등)의
  `col`은 하위 parser marker 값 그대로라 0부터 셉니다(`duplicate-key@2:0`은 2번째 줄 첫 열). `line`은 1부터
  셉니다.
- 선행 BOM이 있는 `stellar.yaml`은 첫 키가 `\u{feff}schema_version`이 되어 `UnknownKey`로 거부됩니다
  (`stellar-yaml-bom`, 형식 §14.1 "현재 구현").
- `chunk-file-payload-hash-mismatch`의 오류 이름에 붙은 `ChunkObjectPath`는 오류 값에 들어 있는 경로 타입
  이름입니다.
