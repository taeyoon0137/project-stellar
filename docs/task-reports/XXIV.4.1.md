# 작업 보고서: XXIV.4.1 사용자 명령 이름 `stellar` 고정

- WBS: XXIV.4.1
- 작업명: `stellar-cli` package의 실행 파일 이름을 `stellar`로 고정하고 help·usage·stderr 접두어를 `stellar`로 변경
- 상태: 완료 (`[x]`)
- 선행: XXIV.1.2(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `tools/stellar-cli/Cargo.toml` — `[[bin]] name = "stellar"`, `path = "src/main.rs"` 추가. package 이름 `stellar-cli`, library 이름 `stellar_cli` 유지.
- `tools/stellar-cli/src/cli.rs` — 실행 이름 상수 `PROGRAM = "stellar"`, 도움말·사용법 오류·출력 실패 진단에 사용.
- `tools/stellar-cli/src/output.rs` — stderr 진단 줄 접두어를 `PROGRAM`으로 변경, stdout/stderr 계약 문서에 접두어와 schema 식별자 유지 명시.
- `tools/stellar-cli/src/main.rs`, `src/lib.rs` — UTF-8이 아닌 인자 진단 접두어, crate 문서.
- `tools/stellar-cli/tests/headless_help.rs` — `CARGO_BIN_EXE_stellar` 사용, 실행 파일 이름·help snapshot·stderr 접두어 검사 추가.
- `tools/stellar-cli/tests/output_fixtures.rs` — `CARGO_BIN_EXE_stellar` 사용, 오류 fixture의 stderr 줄 접두어 검사 추가.
- `tools/stellar-cli/tests/fixtures/help.txt`(신규) — help snapshot.
- `docs/task-reports/XXIV.4.1.md`(신규), 계획서(XXIV.4.1 `[ ]`→`[x]`).

## 구현 요약

- **실행 이름**: `[[bin]]`이 `src/main.rs`를 `stellar`로 빌드합니다. 같은 경로를 명시했으므로 Cargo가 package 이름(`stellar-cli`)으로 추론하던 실행 파일은 더 이상 만들어지지 않습니다. `cargo build -p stellar-cli --message-format=json`의 산출물은 lib `stellar_cli`와 bin `stellar`(`target/debug/stellar`) 두 개였습니다.
- **표기**: 도움말 제목·사용법(`stellar <명령> [인자]`)·재안내(`` `stellar --help` ``), 사용법 오류(`stellar: ...`), 명령 진단(`stellar <명령>: 오류(...)`)이 모두 `stellar`를 씁니다(ADR-0005 §2 "문서, 도움말, 종료 코드 설명은 `stellar <명령>`").
- **바꾸지 않은 것**: 결과 envelope schema 식별자 `stellar-cli/result`, `version` 결과의 `name: "stellar-cli"`(package 이름), `schema/cli-result.v1.schema.json`, `tests/fixtures/results/*.json`. 도움말에 남은 `stellar-cli` 문자열은 schema 식별자 `"stellar-cli/result"` 하나뿐이며 테스트가 이를 고정합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo build -p stellar-cli` (`--message-format=json`으로 산출물 확인) | 종료 코드 0, 실행 파일 `target/debug/stellar`만 생성(bin `stellar-cli` 없음) |
| `cargo test -p stellar-cli` | unit 19, headless_help 6, json_copy_sync 2, output_fixtures 9 passed, 0 failed |
| `cargo clippy -p stellar-cli --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-cli -- --check` | 통과 |
| `cargo test -p xtask crate_graph` | 8 passed(`stellar_cli_uses_only_core_crates` 포함, manifest의 package 이름 해석 유지) |
| `git diff --stat 78a574a -- tools/stellar-cli/tests/fixtures/results tools/stellar-cli/schema` | 변경 없음 |

| 검사 | 결과 |
|---|---|
| 산출 실행 파일 이름 `stellar`, package 이름 `stellar-cli`, `CARGO_BIN_EXE_stellar-cli` 미정의(`built_executable_is_named_stellar`) | 통과 |
| `--help`·`-h`·`help` 출력이 `tests/fixtures/help.txt`와 byte 일치, 작업 디렉터리에 파일 생성 0(`help_runs_without_app_gui_or_environment`) | 통과 |
| help에서 `stellar-cli`는 schema 식별자에만 존재(`help_uses_stellar_except_the_result_schema_id`) | 통과 |
| 사용법 오류 stderr가 `stellar: `로 시작하고 `` `stellar --help` `` 안내, `stellar-cli` 없음(`usage_errors_exit_64_with_hint_on_stderr`) | 통과 |
| XXIV.1.2 JSON fixture 9종 golden byte 일치·스키마 통과, 오류 fixture의 stderr 모든 줄이 `stellar `로 시작(`output_fixtures`) | 통과 |
| 변경 전 실행 파일(`stellar-cli --help`)과 snapshot 비교 | 제목·사용법·재안내 3줄 불일치(테스트가 이전 동작을 거부함을 확인) |

## 차이(작업 지시와 다른 점)

- 없음. 작업 지시대로 `[[bin]] name = "stellar"`만 추가했고, `stellar` 이름을 쓸 수 없을 때의 `stellar-cli` 대체 설치(ADR-0005 결정 7)는 설치 작업(XXIV.5)의 범위로 남겼습니다.

## 남은 위험 / 한계

- 이전 실행 이름 `stellar-cli`를 직접 호출하던 로컬 스크립트는 실행 파일을 찾지 못합니다. `rg "CARGO_BIN_EXE_stellar-cli|target/debug/stellar-cli"` 결과, 저장소에서 이전 이름을 호출하는 코드는 없고 부재를 검사하는 테스트 1건과 과거 보고서 `docs/task-reports/XXIV.1.1.md`의 실행 기록 1건만 남았습니다(과거 기록은 고치지 않음).
- `stellar` 이름이 사용자 PATH의 다른 도구와 겹칠 때의 처리는 명령줄 설치 화면(XXIV.5)에서 정합니다.
