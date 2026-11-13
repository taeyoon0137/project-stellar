# 작업 보고서: XIV.1.6 Git command trace와 debug log redaction

- WBS: XIV.1.6
- 작업명: Git command trace에는 command kind·duration·exit code만 기록하고 URL·branch·path·commit
  hash를 telemetry에서 제외, 로컬 debug log에도 redaction 적용
- 상태: 완료 (`[x]`)
- 선행: II.3.2, XIV.1.5 (모두 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증; 프로세스 fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/trace.rs`(신규) — `GitCommandKind`(고정 목록), `GitTraceExit`,
  `GitCommandTrace`, `GitDebugLine`, `GitTraceSink`, `redact_argv`, `redact_debug_text`,
  `MAX_DEBUG_TEXT_BYTES`.
- `crates/git-porcelain/src/runner.rs` — `GitRunner::with_trace_sink`, 실행마다 trace·debug line 전송
  (`emit_trace`), 기존 실행 본문을 `spawn_and_wait`로 분리, secret fixture 테스트.
- `crates/git-porcelain/src/lib.rs` — `trace` 모듈 등록.
- `docs/task-reports/XIV.1.6.md`(신규), 계획서(XIV.1.6 `[ ]`→`[x]`).

## 구현 요약

- **trace(telemetry 허용 필드만)**: `GitCommandTrace { kind, duration, exit }`. 세 필드가 모두 고정
  enum·`Duration`인 `Copy` 타입이라 문자열(URL·branch·경로·hash)을 담을 수 없습니다(문자열 필드가 생기면
  `Copy` 단언 테스트가 컴파일에 실패). `kind`는 global option(`-c`·`-C`·`--git-dir` 등)을 건너뛴 첫
  인자를 고정 subcommand 목록 44개에서만 고르고, 모르면 `Other`(이름 미기록)입니다. `exit`는 종료 코드
  또는 `Signal`·`SpawnFailed`·`TimedOut`·`Cancelled`·`WaitFailed`입니다. `kind.as_str()`는
  `&'static str`이라 II.3.2 `TelemetryEvent { name: &'static str, .. }`에 그대로 쓸 수 있습니다.
- **runner 연결**: `with_trace_sink(Arc<dyn GitTraceSink>)`로 연결하면 프로세스 생성을 시도한 모든 실행
  (spawn 실패·timeout·취소 포함)마다 `record(&trace)` 후 `record_debug(&GitDebugLine)`을 호출합니다.
  실행 전 취소처럼 프로세스를 만들지 않은 요청은 기록하지 않습니다. 사본은 같은 sink를 공유합니다.
- **debug log redaction**: `GitDebugLine`은 생성자가 redaction을 수행하므로 원문을 담을 수 없습니다.
  - argv: subcommand와 고정 2단계 subcommand(`worktree add`, `lfs track` 등)·option 이름만 유지하고,
    option 값(`--x=<value>`), `-c` 값(`<config>`), `-C` 값(`<path>`), 위치 인자(`<url>`·`<oid>`·`<ref>`·
    `<path>`·`<arg>`), `--` 뒤 인자를 모두 바꿉니다.
  - stderr(실패한 실행만): URL(자격 증명 포함)·scp 형식 원격 → `<url>`, 따옴표 안 → `<redacted>`(Git이
    branch·경로·remote 이름을 따옴표로 출력), 절대 경로 → `<path>`, `refs/...` → `<ref>`, 7~64자 16진수
    조각 → `<oid>`. 입력은 16 KiB에서 자릅니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | lib 264 passed + doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 통과 |
| `cargo check -p stellar-project --all-targets` | 통과(하위 crate 영향 없음) |

| 검사 | 결과 |
|---|---|
| **secret fixture redaction**: 토큰 포함 URL, `refs/heads/secret-branch` refspec, `credential.helper` `-c` 값, 절대 경로, 40자 hash를 argv로 받고 같은 비밀을 stderr로 출력하는 가짜 Git 실행 → 수집한 trace Debug 출력과 debug line 전체에 비밀 11종 미노출, debug line은 `git -c <config> push --porcelain <url> <ref> -- <path> <oid> kind=push exit=1 ...` | 통과 |
| trace 종류·종료 상태: `push` Code(1), 사본 실행 `status` Code(1), spawn 실패 `clone` SpawnFailed, 실행 전 취소는 미기록 | 통과 |
| 명령 종류 고정 목록 왕복, global option 건너뛰기, 모르는 subcommand `Other`, `--version`·global option만 | 통과 |
| argv redaction 12개 사례(option 값·2단계 subcommand·scp URL·`-C` 경로 등) | 통과 |
| 텍스트 redaction: URL·따옴표 이름·경로·hash·범위(`abc1234..def5678`)·`refs/` 미노출, 닫히지 않은 따옴표·길이 상한·다중 byte 경계 | 통과 |
| `GitCommandTrace`·`GitCommandKind`·`GitTraceExit` `Copy` 단언 | 통과 |

## 차이(작업 지시와 다른 점)

- II.3.2의 `stellar-diagnostics` 타입을 직접 쓰지 않았습니다. crate graph(`tools/xtask/src/crate_graph.rs`)
  가 git-porcelain → core-types만 허용하기 때문입니다. 대신 trace 자체를 문자열 없는 `Copy` 타입으로
  두어 같은 보장(민감 값 수용 불가)을 타입으로 만들고, 상위 계층이 `kind.as_str()`(`&'static str`)로
  `TelemetryEvent`를 만들 수 있게 했습니다.
- 종료 코드 필드는 종료 코드가 없는 경우(시그널·timeout·취소·spawn 실패)를 구분하는 enum
  `GitTraceExit`로 표현했습니다.

## 남은 위험 / 한계

- 따옴표 없는 자유 문장 속 branch 이름(예: `! [rejected] main -> main`)은 일반 단어와 구분할 수 없어
  debug log에 남을 수 있습니다. 그래서 stderr는 로컬 debug log에만 redaction 후 넣고 telemetry
  (`GitCommandTrace`)에는 넣지 않습니다.
- argv에서 `--` 앞의 option 모양 token(`-x`, `--name`)은 option 이름으로 유지합니다. Stellar는 사용자
  텍스트(commit message 등)를 stdin으로 넘기므로 해당 없지만, option 값 자리에 `-`로 시작하는 사용자
  값을 argv로 넘기는 새 호출은 이 규칙을 고려해야 합니다.
- 앱 진단 저장소·telemetry 전송 경로에 sink를 연결하는 작업은 상위 계층(runtime·diagnostics) 몫입니다.
