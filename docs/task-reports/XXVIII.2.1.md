# 작업 보고서: XXVIII.2.1 valid/invalid binary·YAML golden corpus

- WBS: XXVIII.2.1
- 작업명: valid/invalid binary/YAML golden corpus를 라이선스와 함께 공개 가능한 형태로 정리(synthetic 내용만)
- 상태: 부분 완료 (`[~]`) — corpus·manifest는 완료, 라이선스는 미결정
- 선행: XXVIII.1.2(기준 commit에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (문서·데이터 작업, 로컬 검증).

## 변경 파일 목록

- `docs/spec/conformance/README.md`(신규) — corpus 목적·내용 정책·파일 형식·manifest 형식·대상과 판정·오류 표기·
  라이선스 상태·검증 방법·관찰 동작.
- `docs/spec/conformance/manifest.yaml`(신규) — corpus manifest(항목 61개, 제외 7개, 관련 fixture 1개).
- `docs/spec/conformance/binary/*.hex`(신규 33개), `docs/spec/conformance/yaml/*.yaml`(신규 25개) — 입력.
- `docs/task-reports/XXVIII.2.1.md`(신규), 계획서(XXVIII.2.1 `[ ]`→`[~]`).
- 코드 변경 없음. 문서·데이터 파일만 변경했습니다.

## 구현 요약

- **구성**: 대상 13종(저장 형식 binary 6, 문서 의미 레코드 binary 4, YAML 3). 항목 61개 = valid 22, invalid 39.
  파일 58개(같은 YAML 파일 3개는 loader 단계와 스키마 단계 두 항목).
- **복사(origin copied, 28개 항목)**: storage seed corpus 중 파일 형식 입력 12개, 제한 YAML seed 12개(loader 단계),
  그중 3개의 스키마 단계 항목, stellar-cli 유효 `stellar.yaml` fixture 1개. 모두 원본과 byte 동일하며 `source`를
  기록했습니다. storage seed `stream_*.hex` 7개는 fuzz harness op stream이라 `excluded`에 이유와 함께 남겼습니다.
- **생성(origin generated, 33개 항목)**: 기준 commit의 공식 encoder 출력 또는 그 bytes의 지정 위치를 바꾼 입력.
  envelope magic, valid pointer, generation 0, CRLF current pointer, ViewTree 잔여 byte, chunk 길이·hash 불일치,
  DocumentDefinition·DocumentManifest·SubtreeRecords·ModifierStack의 valid/invalid, `stellar.yaml`(기본, 의존성,
  branch 선택자, 알 수 없는 키, project 누락, 선행 0 버전, BOM, `-` 시작 repository), `stellar.lock.yaml`(빈 lock,
  XV.4.1 golden과 같은 resolved lock, 알 수 없는 키, requested 누락).
- **기대 결과**: 모든 항목을 기준 구현으로 실행해 `expect`(ok 또는 오류 variant 경로)와 `canonical`(다시 쓰면
  같은 bytes인지)을 기록했습니다.
- **synthetic**: 식별자는 고정 seed UUIDv7 형태, 이름은 예시 값, URL은 `example.com`·`example` 경로입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| scratch 생성 도구 `corpusgen generate`(작업 트리 밖, path 의존 crate) | 항목 61개 생성·관찰, 종료 코드 0 |
| scratch 검증 도구 `corpusgen verify` | manifest를 Stellar 제한 YAML loader로 읽고 61개 항목 재실행: `verify: 61 passed, 0 failed`, 종료 코드 0 |
| manifest 완전성 검사 스크립트(scratchpad `check_corpus_manifest.py`) | 1228개 검사 통과, 0 실패(항목 61, 파일 58, 제외 7), 종료 코드 0 |
| 완전성 검사 음성 테스트(sha256 1자리, validity 1개, 항목 1개 삭제를 일부러 틀린 사본) | 주입한 3개 오류를 모두 실패로 보고, 종료 코드 1 |
| `cargo xtask sync-plan --check` | 종료 코드 0 |
| `cargo xtask validate-task-status` | 종료 코드 0 |
| `cargo xtask validate-adr <변경 경로>` | 종료 코드 0(보호 경로 변경 없음) |
| `git diff --check`(새 파일 intent-to-add 포함) | 출력 없음 |
| `cargo xtask validate-task-report docs/task-reports/XXVIII.2.1.md` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| `binary/`·`yaml/`의 모든 파일이 항목에 있고 모든 항목의 파일이 있음 | 통과 |
| 모든 항목 `sha256`이 파일 bytes와 같음, copied 항목은 원본과 byte 동일 | 통과 |
| storage·YAML seed 디렉터리의 모든 파일이 복사되었거나 `excluded`에 있음 | 통과(31개 중 복사 24, 제외 7) |
| 복사한 storage seed의 `# expect:` 주석과 manifest `expect` 일치 | 통과(12개) |
| `validity`와 `expect` 일치, `canonical`은 ok 항목에만 | 통과 |
| valid binary 항목의 재인코딩이 입력과 같음 | 통과(valid binary 10개 모두 `canonical: "true"`) |
| valid YAML 스키마 항목의 canonical 기록 비교 | 7개 중 4개 `"true"`, 3개 `"false"`(비인용 `schema_version`·YAML 1.1 혼동 스칼라·비인용 `local_override` 입력) |
| ADR-0021 이전 형식 seed(`manifest_full.yaml`, `lock_multi_version.yaml`)가 스키마 단계에서 `MissingField[...library_id]`로 거부됨 | 관찰됨, 항목으로 기록 |

## 차이(작업 지시와 다른 점)

- **라이선스를 붙이지 않았습니다.** 저장소에 라이선스가 없고 corpus 라이선스 선택은 product owner 결정입니다.
  manifest `license.status: undecided`와 README §6에 남겼고, 그래서 상태를 `[~]`로 두었습니다. "공개 가능한
  형태"의 나머지(자기 완결 디렉터리, synthetic 내용, manifest, 원본 추적)는 갖췄습니다.
- 저장소 안 conformance runner와 기대 결과 schema는 만들지 않았습니다(XXVIII.2.2 범위, 허용 경로 밖 코드).
  검증 도구는 scratchpad에 있습니다.
- node payload 종류별 단독 입력(Frame·Shape·Path·Text·Image)과 Semantic Snapshot·diff 교환 bytes는 넣지
  않았습니다. 각각 형식 명세 범위 밖과 XXVIII.2.3 범위입니다.

## 남은 위험 / 한계

- 라이선스 결정 전에는 외부 공개 불가입니다. 결정 후 manifest `license`와 라이선스 본문을 추가해야 `[x]`가 됩니다.
- 오류 표기는 기준 구현 오류 variant 이름입니다. 오류 enum을 바꾸면 manifest를 다시 만들어야 하며, 이를
  강제하는 저장소 검사는 없습니다(XXVIII.2.2 runner가 필요).
- 생성 항목을 다시 만드는 도구가 저장소 밖에 있습니다. 파일은 sha256으로 고정되지만 재생성 절차는 runner
  작업에서 저장소로 옮겨야 합니다.
- 형식 명세 §14.5·§14.6(`stellar.yaml` 의존성·lock entry)과 §14.2(loader 오류 열 번호)는 기준 commit 구현과
  다릅니다(ADR-0021 단일화, parser 단계 오류의 열은 0부터). 별도 문서 수정 commit으로 고칩니다.
