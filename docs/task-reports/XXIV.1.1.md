# 작업 보고서: XXIV.1.1 stellar-cli headless binary 골격

- WBS: XXIV.1.1
- 작업명: `stellar-cli`를 app UI와 분리된 binary로 생성하고 core crate만 사용
- 상태: 완료 (`[x]`)
- 선행: G8(gate 수준 선행 — 사용자 지시 2026-11-14에 따라 완화, 아래 "차이" 참조)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `tools/stellar-cli/Cargo.toml`(신규) — binary+lib package, 외부·내부 의존성 없음.
- `tools/stellar-cli/src/lib.rs`, `src/cli.rs`, `src/main.rs`(신규) — 명령 registry, 도움말, 진입점.
- `tools/stellar-cli/tests/headless_help.rs`(신규) — headless 실행·의존성 inventory 검증.
- `Cargo.toml` — workspace member에 `tools/stellar-cli` 추가.
- `Cargo.lock` — cargo 자동 갱신(`stellar-cli` package 항목).
- `tools/xtask/src/crate_graph.rs` — `stellar-cli` 허용 edge(core-types, project, document, storage,
  diagnostics)와 실제 manifest 검사 테스트.
- `docs/task-reports/XXIV.1.1.md`(신규), 계획서(XXIV.1.1 `[ ]`→`[x]`).

## 구현 요약

- **분리된 binary**: `tools/stellar-cli`는 앱 UI·FFI·renderer·MCP와 무관한 독립 실행 파일입니다.
  `main.rs`는 인자를 `cli::run`(순수 함수, stdout·stderr를 인자로 받음)에 넘기고 종료 코드로
  끝납니다. UTF-8이 아닌 인자는 panic 없이 사용법 오류(64)로 처리합니다.
- **도움말**: `help`·`--help`·`-h`는 도움말을 stdout에 쓰고 0으로 끝납니다. 명령 없음·알 수 없는
  명령·`help`의 여분 인자는 stderr 안내와 함께 64, 출력 실패는 74(sysexits)입니다. 종료 코드
  전체 표와 JSON 출력은 XXIV.1.2에서 정의합니다.
- **core crate만 사용**: crate graph 정책에 `stellar-cli` → core-types·project·document·storage·
  diagnostics만 허용합니다. MCP·FFI·renderer edge는 `forbidden-edge`입니다. 현재 단계는 어떤
  의존성도 쓰지 않으며, 링크 결과도 `libSystem.B.dylib` 하나뿐입니다(`otool -L`).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-cli` | unit 4 passed, integration 4 passed, 0 failed |
| `cargo clippy -p stellar-cli --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-cli -- --check` | 통과 |
| `cargo test -p xtask` | 115 passed(+2, +2), 0 failed |
| `cargo clippy -p xtask --all-targets -- -D warnings` | 경고 0 |
| `cargo run -q -p xtask -- validate-crate-graph` | 종료 코드 0 |
| `otool -L target/debug/stellar-cli` | `/usr/lib/libSystem.B.dylib`만 링크 |

| 검사 | 결과 |
|---|---|
| 환경 변수 전부 제거(HOME·PATH·DISPLAY·TMPDIR 없음)·빈 cwd·닫힌 stdin에서 `--help`/`-h`/`help` 종료 0, stdout 도움말, stderr 비어 있음, cwd에 파일 생성 없음(`help_runs_without_app_gui_or_environment`) | 통과 |
| 명령 없음·알 수 없는 명령 종료 64, 안내는 stderr(`usage_errors_exit_64_with_hint_on_stderr`) | 통과 |
| UTF-8이 아닌 인자 종료 64, panic 없음(`non_utf8_argument_is_usage_error_without_panic`) | 통과 |
| manifest 의존성이 core crate path 의존뿐, build script 없음(`manifest_depends_only_on_core_crates`) | 통과 |
| crate graph: 실제 manifest edge 통과, `stellar-cli`→mcp/ffi/renderer 금지(`stellar_cli_uses_only_core_crates`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 G8(초기 릴리스 gate)은 gate 수준 선행이라 사용자 지시(2026-11-14)에 따라 완화했습니다.
  이 작업에 필요한 코드는 모두 3002e6d에 이미 있습니다.
- "headless machine"은 별도 기기 대신 환경 변수를 모두 제거한 프로세스 실행과 링크 라이브러리
  확인(`libSystem`만)으로 검증했습니다.

## 남은 위험 / 한계

- `docs/architecture/crate-dependency-graph.md`(사람용 표)는 이 작업의 허용 경로 밖이라 갱신하지
  않았습니다. 강제 정책(`crate_graph.rs`)에는 반영되어 있으며, 문서 표는 후속 문서 작업에서
  맞춰야 합니다.
- `validate-crate-graph` 명령은 `crates/*`만 스캔합니다. `tools/stellar-cli`의 edge는
  `crate_graph.rs` 단위 테스트가 실제 manifest를 읽어 검사합니다.
