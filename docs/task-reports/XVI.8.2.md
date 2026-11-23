# 작업 보고서: XVI.8.2 금지 필드 scanner

- WBS: XVI.8.2
- 작업명: project text/name/path, Git URL/branch/commit, terminal/MCP payload를 진단 번들에서 탐지하는 금지 필드 scanner
- 상태: 완료 (`[x]`) — 검증 기준(secret corpus 탐지 100%)을 끝냈습니다.
- 결정: ADR-0109 결정 4, 결정 6(번들 형식)
- 선행: XVI.8.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/diagnostics/src/scanner.rs`(신규).
- `crates/diagnostics/src/bundle.rs`(신규: 번들 형식 `stellar.diagnostic-bundle v1`, `env`·`end` 줄, encode·decode).
- `crates/diagnostics/src/store.rs`(CPU 구조 `Arch`, schema 어휘 `schema_vocabulary` 추가).
- `crates/diagnostics/src/lib.rs`(module 등록 두 줄).
- `docs/task-reports/XVI.8.2.md`(신규).

## 구현 요약

- 번들 형식: 머리 줄, `env app_version= os_version= arch=`, `meta dropped=`, record 줄, `end records=<n>`. 줄 codec과 record schema는 store와 같은 것을 씁니다. 운영체제 version은 macOS `SystemVersion.plist`의 `ProductVersion`을 숫자로만 읽습니다.
- `scan_bundle`은 세 층으로 봅니다.
  - 내용 규칙 19개: 제어 문자·shell prompt·shell 이름 같은 터미널 입출력, JSON-RPC·MCP method·에이전트 단어·JSON 괄호 같은 MCP·에이전트 payload, 자격 증명 표기와 `a@b` 계정 식별, URL scheme·scp 원격·`.git` 같은 Git URL, `refs/heads/`·`origin/`·`HEAD` 같은 branch, 7\~64자 hex(숫자와 문자 섞임)나 21자 이상 숫자열 commit, 사용자 홈·볼륨 경로·Windows drive·경로 구분자·파일 확장자 같은 경로, 비ASCII·따옴표·대문자·문장 부호 같은 프로젝트 텍스트.
  - 문맥 값: 앱이 아는 금지 값(`ScanContext`)을 ASCII 대소문자 무시로 찾습니다. 4 bytes 미만이나 schema 어휘·정수·version과 같은 값은 비교하지 않습니다.
  - 구조 검사: 줄마다 자리(머리, env, meta, record, end)에 맞는 schema로 보고, 첫 오류에서 멈추지 않습니다. 줄마다 맞으면 순서·중복·개수 정규형을 `decode_bundle`로 한 번 더 봅니다.
- `scan_text`는 구조 검사 없이 임의 진단 text(로그, fixture)에 내용 규칙과 문맥 값만 씁니다.
- `Finding`은 줄 번호, 범주, 규칙 이름만 담습니다. `ScanContext`의 `Debug`도 값 수만 냅니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-diagnostics --lib` | 56 passed(scanner 7개, bundle 3개 포함), 0 failed |
| `cargo clippy -p stellar-diagnostics --all-targets -- -D warnings` | 경고 0 |
| 최종 검증(`check-core` 등) | lane 결과 보고에 기록 |

## 테스트

- `secret_corpus_is_detected_completely`: 검증 기준(secret corpus 탐지 100%). 10개 범주 전부를 덮는 60개 secret을 번들의 네 위치(field 값, 덧붙인 token, 새 줄, key)에 넣은 240개 번들에서 기대 범주를 모두 탐지합니다(240/240). 구조 범주를 뺀 항목은 `scan_text`(내용 규칙만)로도 범주가 맞습니다.
- `valid_bundles_have_no_findings`: 모든 enum 값·경계 정수·두 CPU 구조를 쓴 정상 번들과 빈 번들의 탐지 0건(오탐 0).
- `schema_vocabulary_triggers_no_rule`: schema 어휘 하나하나가 어떤 내용 규칙에도 걸리지 않음.
- `findings_never_carry_the_secret`, `context_skips_values_that_cannot_be_told_apart`, `structure_errors_report_every_bad_line`(여러 줄 오류, end 개수, 끝 줄바꿈, 잘림, counter 순서, 비UTF-8), `oversized_input_is_one_finding`.
- bundle: `bundle_bytes_are_golden_and_round_trip`, `bundle_layout_errors_are_rejected`, `product_version_reads_numbers_only`.

## 성능

- 번들 bytes에 선형이고 입력 상한은 8 MiB입니다. 문맥 값 비교는 줄 수 × 값 수입니다.

## 차이(작업 지시와 다른 점)

- 계획 문구에 없는 범주 두 가지를 더했습니다. 자격 증명·계정 식별(`credential`)은 아키텍처 §31·XXIX.2.5가 진단에 넣지 말라고 한 계정 식별 정보를 막고, `unrecognized`는 schema 밖 내용을 막습니다.
- 레이어·토큰 이름은 프로젝트 이름과 같은 범주(`project_name`)로 봅니다.

## 남은 위험 / 한계

- 문맥 없는 소문자 단어(예: 사용자 정의 이름 `checkout`)는 범주를 알 수 없어 `unrecognized`로만 잡힙니다. 번들 안에서는 구조 검사가 막지만, `scan_text`로 자유 text를 볼 때는 문맥 값이 있어야 이름으로 잡힙니다.
- schema 어휘와 같은 문맥 값(예: branch 이름 `main`)은 비교에서 빠집니다. 번들에서는 정해진 field 자리에서만 그 어휘가 나올 수 있어 정보가 새지 않습니다.
- 앱이 문맥 값을 넘기는 연결은 UI 재설계 때 합니다.
