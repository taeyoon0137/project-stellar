# 작업 보고서: XXIV.1.2 stellar-cli 종료 코드와 JSON 출력 스키마

- WBS: XXIV.1.2
- 작업명: exit code와 machine-readable JSON output schema 정의, stdout/stderr 역할 분리
- 상태: 완료 (`[x]`)
- 선행: XXIV.1.1(같은 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `tools/stellar-cli/src/exit.rs`(신규) — sysexits 기반 `ExitStatus`(0·1·64~78), io 오류 분류.
- `tools/stellar-cli/src/json.rs`(신규) — 엄격한 JSON reader·writer(크기·깊이 제한, serde 없음).
- `tools/stellar-cli/src/output.rs`(신규) — 결과 envelope v1, stdout/stderr 계약, `CliError`·`Diagnostic`.
- `tools/stellar-cli/src/commands.rs`(신규) — `version`, `inspect <경로>`(읽기 전용 project probe).
- `tools/stellar-cli/src/cli.rs`, `src/lib.rs` — 명령 dispatch·도움말(출력·종료 코드 표) 확장.
- `tools/stellar-cli/Cargo.toml` — 내부 path 의존 `stellar-project` 추가(정책 허용 edge).
- `Cargo.lock` — cargo 자동 갱신.
- `tools/stellar-cli/schema/cli-result.v1.schema.json`(신규) — envelope JSON Schema.
- `tools/stellar-cli/tests/output_fixtures.rs`(신규), `tests/fixtures/projects/*.stellar.yaml`,
  `tests/fixtures/results/*.json`(신규) — success/error golden fixture.
- `docs/task-reports/XXIV.1.2.md`(신규), 계획서(XXIV.1.2 `[ ]`→`[x]`).

## 구현 요약

- **종료 코드**: 0 성공, 1 도메인 실패(명령은 실행됐지만 결과가 부정적), 64 usage, 65 dataerr,
  66 noinput, 69 unavailable, 70 software, 73 cantcreat, 74 ioerr, 75 tempfail, 76 protocol,
  77 noperm, 78 config. 값과 이름은 golden 테스트로 고정했고 도움말에 전체 표를 출력합니다.
  io 오류 종류는 NotFound→66, PermissionDenied→77, AlreadyExists→73, 일시적 오류→75 등으로
  분류합니다.
- **stdout/stderr 분리**: 명령이 실행되면 stdout에 envelope JSON 정확히 한 줄만 씁니다(성공·실패
  모두). 사람용 진단은 stderr에만 쓰며, 입력 경로 같은 지역 정보는 stderr에만 넣고 stdout JSON에는
  넣지 않습니다. 사용법 오류(64)는 명령이 실행되지 않은 상태이므로 stdout에 쓰지 않습니다.
  도움말(`help`)은 사용자가 요청한 텍스트 결과라 stdout에 씁니다.
- **envelope v1**: `schema`·`schema_version`·`command`·`ok`·`exit_code`·`exit_name`·`result`·`error`·
  `diagnostics` 9개 key가 항상 고정 순서로 있습니다. `schema/cli-result.v1.schema.json`이 같은 계약을
  JSON Schema로 기술하고, 테스트가 스키마 문서와 코드(key 목록·종료 코드 enum)의 일치를 검사합니다.
- **JSON reader·writer**: RFC 8259 엄격 파싱(후행 쉼표·주석·NaN·선행 0·BOM·제어 문자·잘못된 escape·
  짝 없는 surrogate·중복 key·후행 데이터 거부), 기본 1 MiB·깊이 64 제한(깊이 초과는 재귀 전에
  거부해 stack overflow 없음). 숫자는 검증된 원문을 보존합니다. writer는 공백 없는 결정론적 표기이며
  개행·제어 문자·U+2028/2029를 이스케이프해 출력이 항상 한 줄입니다.
- **명령**: `version`(이름·버전·출력 스키마 버전), `inspect <경로>`(IV.5.1 `probe_project`로 root와
  `stellar.yaml`만 검사, 문서 청크는 읽지 않고 파일을 변경하지 않음; 의존성 repository URL은
  결과에서 제외).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-cli` | unit 19, headless_help 4, output_fixtures 9 passed, 0 failed |
| `cargo clippy -p stellar-cli --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-cli -- --check` | 통과 |
| `cargo test -p xtask crate_graph` | 4 passed(`stellar_cli_uses_only_core_crates` 포함) |
| `cargo run -q -p xtask -- validate-crate-graph` | 종료 코드 0 |

| 검사(fixture) | 종료 코드 | 결과 |
|---|---|---|
| `version`·`--version` → `version.json` | 0 | 통과 |
| valid 프로젝트 `inspect` → `inspect-valid.json`(repository URL 미포함) | 0 | 통과 |
| `.git`만 있음 / `stellar.yaml`만 있음 / 파일 경로 → `inspect-not-a-project.json`·`inspect-missing-git.json`·`inspect-not-a-directory.json` | 1 | 통과 |
| anchor 손상 YAML / 알 수 없는 key → `inspect-corrupt-manifest.json`·`inspect-unknown-key.json` | 65 | 통과 |
| 없는 경로 → `inspect-no-input.json` | 66 | 통과 |
| 읽기 권한 없는 `stellar.yaml` → `inspect-no-permission.json` | 77 | 통과 |
| 명령 없음·알 수 없는 명령·인자 누락/초과·알 수 없는 옵션: stdout 비어 있음, stderr 안내 | 64 | 통과 |
| 모든 fixture: stdout golden byte 일치, JSON 한 줄, 엄격 reader 파싱, 스키마 검사 통과, `exit_code` = 프로세스 종료 코드, 경로는 stdout에 없음 | — | 통과 |
| 스키마 검사기가 추가 key·잘못된 종료 코드·누락 key를 거부(`schema_validator_rejects_contract_violations`) | — | 통과 |
| JSON malformed 입력 36종·깊이/크기 제한·invalid UTF-8 거부, writer 개행 없음·round-trip(`json::tests`) | — | 통과 |

## 차이(작업 지시와 다른 점)

- 사용법 오류(64)는 stdout에 JSON을 쓰지 않고 stderr로만 안내합니다. XXIV.1.1에서 정한 "사용법
  오류는 stdout에 결과 없음" 계약을 유지했고, 명령이 실행된 경우에만 결과 문서를 냅니다.
- success/error fixture를 만들기 위해 읽기 전용 `inspect` 명령을 추가했습니다. `validate` 등
  본격 명령은 XXIV.1.3 범위입니다.
- 69·70·73·74·75·76·78은 코드와 스키마에 정의했지만 현재 명령에서 재현 가능한 경로가 없어
  fixture 대신 golden 표·io 분류 단위 테스트로만 검증했습니다.

## 남은 위험 / 한계

- `stellar-project`가 승인된 외부 crate(`yaml-rust2`)를 전이 의존합니다. stellar-cli의 직접 의존은
  core crate뿐입니다.
- 오류 메시지는 한국어 고정 문자열입니다. 기계 처리는 `error.code`·`diagnostics[].code`·
  `exit_code`로 하도록 계약했습니다. 메시지 문구 변경은 golden fixture 갱신이 필요합니다.
- `version` fixture는 package 버전(`0.0.0`)을 담으므로 버전을 올리면 fixture도 함께 갱신해야 합니다.
