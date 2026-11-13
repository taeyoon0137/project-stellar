# 작업 보고서: XXVIII.1.4 파일 포맷 보안 한도와 parser 요구사항 명세

- WBS: XXVIII.1.4
- 작업명: 보안 한도, 최대 크기, parser 요구사항, malformed input behavior 명시
- 상태: 완료 (`[x]`)
- 선행: XVI.6(section 단위 선행 — 완화 규칙 적용. XVI.6.1·XVI.6.2는 `[x]`이고 fuzz harness·corpus가
  있습니다. XVI.6.3~XVI.6.7은 `[ ]`이며, 해당 parser는 "fuzz 대상이 아닌 parser"로 명세에 적었습니다)

## 필수 실행 환경

- `DEV-MAC` (문서 작업, 로컬 검증).

## 변경 파일 목록

- `docs/spec/stellar-format-security.md`(신규) — 보안 한도·parser 요구·malformed 입력 동작·fuzz corpus
  연결.
- `docs/task-reports/XXVIII.1.4.md`(신규), 계획서(XXVIII.1.4 `[ ]`→`[x]`).
- 코드 변경 없음. 문서만 변경했습니다.

## 구현 요약

- **위협 모델**: 신뢰되지 않은 프로젝트의 입력 원천과 처리 코드를 표로 정리했습니다(§1).
- **공통 parser 요구 P1~P8**: panic 금지(release `panic = "abort"`), 길이 프리픽스 선검증, count 선할당
  상한, 반복 처리, 정확한 소비, 자동 수정 금지, 타입 기반 경로, 첫 오류 중단(§2).
- **한도 표**: binary 고정 크기·값 범위·decoder별 선할당 상한(1,024/4,096), YAML 1 MiB·`c-printable`·
  깊이 64·선읽기 32,768자·implicit key 1,024자, 에셋 import 512 MiB 등. 상한이 없는 항목도 "정하지
  않은 한도"로 따로 적었습니다(§3, §4, §8).
- **malformed 입력 동작**: canonical primitive, hash 문자열(ADR-0015 진입점별 결과), chunk 읽기, manifest,
  current pointer, 열기 결과, 문서 chunk payload, YAML 각각 입력 → 오류 variant → 열기 경로 효과(§5).
- **fuzz corpus 연결**: storage harness target 7개, seed corpus 19개(지시문 target·expect 포함), stress
  입력 48개(고정 이름 27개 + 다중 byte hash 7종 × 진입점 3), YAML harness seed corpus 12개, stress 입력
  22개를 모두 명세 절에 연결했습니다(§9).
- **알려진 위험 R1~R8**: 코드 분석과 재현으로 찾은 위험을 적었습니다(§10). 수정은 허용 경로 밖입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | unit 227 passed, `chunk_canonical_fuzz` 5 passed(corpus 기대 결과 검사 포함), `migration_git_fixture` 1 passed, doctest 2 passed, 0 failed |
| `cargo test -p stellar-project` | unit 248 passed, `restricted_yaml_fuzz` 4 passed 포함 통합 17 passed, 0 failed |
| `cargo test -p stellar-assets` | 97 passed, 0 failed |
| 명세·코드 일치 검사 스크립트(scratchpad `check_spec.py`, 형식 명세 + 보안 명세 대상) | 212개 검사 통과, 0 실패, backtick 식별자 417개 존재 확인, 종료 코드 0 |
| 일치 검사 스크립트 음성 테스트(YAML 상한 값·stress 입력 이름을 일부러 틀린 사본) | 주입한 오류를 모두 실패로 보고 |
| scratch 실행 파일 `probe`(작업 트리 밖 scratchpad crate, path 의존) | R1 첫 부분 재현: `design/objects/zz` → 프로젝트 밖 디렉터리 symlink, 그 안의 `sha256-<hex>.starchunk` 1개 삭제(`deleted 1 file(s); outside file exists after = false`) |
| scratch 실행 파일 `gcloop 1` | 자기 참조 symlink 1개: sweep 종료(1.37ms) |
| `timeout 30 gcloop 2` | 자기 참조 symlink 2개: 30초 안에 끝나지 않음(종료 코드 124) |
| `git diff --check` | 출력 없음 |
| `cargo xtask validate-task-report docs/task-reports/XXVIII.1.4.md` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| YAML 한도 상수(`MAX_INPUT_BYTES` 1,048,576, `MAX_NESTING_DEPTH` 64, `MAX_EVENT_LOOKAHEAD_CHARS` 32,768, implicit key 1,024)가 코드와 같음 | 통과 |
| `c-printable` 범위 5개가 `is_c_printable`과 같음 | 통과 |
| loader 오류 코드 13개가 명세에 모두 있음 | 통과 |
| 선할당 상한: manifest 2곳·ViewTree·DocumentManifest·`DocumentDefinition` 3곳 1,024, `SubtreeRecords` 2곳 4,096, 복구 스냅샷 1,024×2·4,096×1, canonical decoder 선할당 없음 | 통과 |
| 문서 깊이 64, 에셋 512 MiB·65,536 byte buffer·media type 127자·확장자 16자 | 통과 |
| storage corpus 19개 각 파일의 `target`·`expect` 지시문이 명세 표와 같음 | 통과 |
| YAML corpus 12개가 명세 표에 있음 | 통과 |
| storage stress 입력 이름 34개(다중 byte 7종은 접두어 3종으로 48개 입력이 됨)·YAML stress 입력 22개 이름이 명세에 모두 있음 | 통과 |
| harness target 7개, 반복 수 25,000, 입력당 3초, watchdog 60초, 변형 64 KiB, 할당 상한 식 | 통과 |
| 고정 크기 계산: header 99, pointer 99, manifest 최소 49/57, ViewTree 20+32n, current 72 | 수동 계산과 일치, current 72 byte는 재현 |

## 차이(작업 지시와 다른 점)

- 검증 기준 "fuzz corpus category 전부 문서 연결"은 저장소에 있는 두 harness(XVI.6.1, XVI.6.2)의 corpus와
  stress 입력 전부를 대상으로 했습니다. XVI.6.3~XVI.6.7 harness는 없으므로 연결할 corpus가 없고,
  명세 §9.3에 미대상 parser로 적었습니다.
- 위험 R1(GC symlink 순회)은 문서 작업 중 발견했습니다. 수정은 보호 경로 `crates/storage/`의 코드
  변경이라 하지 않았습니다.

## 남은 위험 / 한계

- R1: `storage::gc::sweep`이 symlink 디렉터리를 따라가 프로젝트 밖 파일을 지울 수 있고, 자기 참조
  symlink 둘이면 끝나지 않습니다. 신뢰되지 않은 프로젝트에서 GC가 돌면 데이터 손실·hang이 가능합니다.
  `symlink_metadata` 기반 순회와 방문 집합이 필요합니다(storage 보호 경로, ADR 필요).
- R2: chunk·manifest·`design/current`·`stellar.yaml`을 크기 제한 없이 메모리로 읽습니다.
- R3: 열기 경로가 모르는 required kind와 스키마 버전으로 거부하지 않습니다(호환성 명세 §4).
- R5·R7·R8: lock 값 미검사, fallback 후보 수 무제한, 고아 임시 파일 미정리.
- 문서 loader·DocumentManifest·복구 스냅샷·AssetRecord decoder에는 fuzz target이 없습니다(XVI.6.3 등).
- 시간 상한(입력당 3초)은 debug build 기준이며, 느린 CI에서는 오탐이 날 수 있다는 이전 리뷰 지적이
  있습니다(이 작업에서 바꾸지 않음).
