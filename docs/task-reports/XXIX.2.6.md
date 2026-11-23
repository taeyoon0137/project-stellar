# 작업 보고서: XXIX.2.6 launch profile fixture

- WBS: XXIX.2.6
- 작업명: CLI 종류와 지원 버전 범위별 Stellar 소유 launch profile fixture
- 상태: `[~]` 부분 완료 — fixture 형식·검증·거부·대역 세션 검사는 끝났고, 공식 profile 두 개는 약관 검토와 Team ID 확인 전이라 비활성입니다
- 선행: XXIX.2.4, XXIX.2.5, XIX.1.8
- 설계 기준: ADR-0005 §4·§6·§8, ADR-0023 결정 9, ADR-0092

## 필수 실행 환경

- `DEV-MAC` (macOS 프로세스·파이프·process group·Code Signing Services, 로컬 검증). 네트워크와 실제 공급자 계정은 쓰지 않습니다.

## 변경 파일 목록

- `crates/agent-host/src/profile.rs`(신규): `LaunchProfile` 형식·검증, `official_profiles`
- `crates/agent-host/profiles/claude-code.json`, `crates/agent-host/profiles/codex.json`(신규)
- `crates/agent-host/tests/launch_profiles.rs`, `crates/agent-host/tests/fixtures/profiles/test-double-{claude,codex}.json`

## 구현 요약

- **형식** `stellar-launch-profile/1`(JSON)의 field는 다음과 같습니다. 모르는 key는 거부합니다.
  - `id`, `cli`, `display_name`(일반 텍스트), `enabled`
  - `versions {min, max_exclusive}`, `team_id`
  - `version_command`, `status_command`, `status_rule`, `session_command`(자리 표시자 `{mcp_config}`만)
  - `input`, `decoder {id, version}`, `env`(이름 배열), `timeouts_ms`
  - `review {terms_date, terms_result, docs_date, docs, note}`
- **환경 변수**: 이름만 허용합니다. `=` 포함, object, 중복, 형식 밖 이름은 거부합니다. 값 field는 없습니다.
- **활성 조건**: `enabled`는 약관 검토 `allowed`와 Team ID가 있어야 load됩니다. 비활성 profile과 지원 범위 밖 버전은 실행 전 확인이 거부합니다.
- 공식 profile 두 개는 2026-11-23 약관 검토 `pending`, Team ID null이라 비활성입니다(ADR-0005 §4.1: 약관 검토상 허용이 명확하지 않은 CLI는 활성 목록에 넣지 않음).

### profile `claude-code-2.1`

- 근거 문서(2026-11-23 확인): Claude Code CLI reference(`https://code.claude.com/docs/en/cli-reference`), Agent SDK TypeScript reference(`SDKMessage`).
- 지원 버전 `[2.1.0, 2.2.0)`, `version_command` `--version`("Output the version number").
- 비대화형 인터페이스: `-p`(print), `--input-format stream-json`, `--output-format stream-json`, `--verbose`(stream-json 출력의 필수 조합). decoder `claude-stream-json@1`. 입력은 stream JSON user message(여러 turn).
- 불변식 1(문서 변경은 Stellar MCP tool로만)을 지키는 공식 설정:
  - `--tools ""`: 내장 tool 전부 끔(Bash·Edit 등 파일·명령 경로 없음).
  - `--strict-mcp-config --mcp-config <scratch 파일>`: `.mcp.json`·사용자 설정의 MCP 서버를 건너뛰고 Stellar MCP만 load.
  - `--allowedTools mcp__stellar`: Stellar MCP tool은 CLI 쪽 확인 없이 호출. 승인은 Stellar 앱이 받음(ADR-0005 §4.3).
- 불변식 2(흔적 없음)를 지키는 공식 설정: `--no-session-persistence`("sessions are not saved to disk and cannot be resumed. Print mode only"). 실행기 scratch cwd(XXIX.2.4). 사용자 설정 파일은 쓰지 않습니다.
- 연결 상태: `auth status`, 종료 코드 규칙(XXIX.2.5).
- 환경 변수 이름과 근거:
  - `HOME`: CLI가 자기 설정·로그인 상태를 사용자 홈 기준으로 찾음(CLI reference의 `~/.claude.json`·설정 디렉터리 언급). Stellar는 그 위치를 읽지 않음.
  - `PATH`: CLI가 하위 실행 파일을 찾는 표준 탐색 경로(POSIX).
  - `USER`: 사용자 이름 표준 변수(POSIX). CLI 실행 환경 유지.
  - `LOGNAME`: 로그인 이름 표준 변수(POSIX).
  - `TMPDIR`: macOS 사용자별 임시 디렉터리(Apple `confstr(3)` `_CS_DARWIN_USER_TEMP_DIR`와 같은 값).
  - `LANG`: 문자 encoding·지역 설정(POSIX locale).
  - `LC_ALL`: locale 강제 설정(POSIX locale).
  - `LC_CTYPE`: 문자 분류 locale(POSIX locale).
  - `CLAUDE_CONFIG_DIR`: 사용자가 설정 디렉터리를 바꾼 경우 CLI가 자기 로그인을 찾는 변수(CLI `auth status`의 `configDirectory` field가 가리키는 설정 디렉터리, env-vars 문서).
  - `HTTPS_PROXY`: 사용자 network proxy 설정을 CLI가 따름(Claude Code network 설정 문서).
  - `HTTP_PROXY`: 위와 같음.
  - `NO_PROXY`: 위와 같음.
- timeout: version 5 s, status 10 s, startup 60 s(`MCP_TIMEOUT` 기본 30 s의 두 배), grace 3 s.
- 약관·문서 검토: 2026-11-23, 약관 결과 `pending`, Team ID 미확인 → 비활성.

### profile `codex-exec-0`

- 근거 문서(2026-11-23 확인): Codex non-interactive mode(`https://learn.chatgpt.com/docs/non-interactive-mode`).
- 지원 버전 `[0.1.0, 1.0.0)`, `version_command` `--version`.
- 비대화형 인터페이스: `exec --json`(stdout이 JSON Lines event stream). decoder `codex-exec-jsonl@1`. 입력은 첫 메시지를 stdin에 쓰고 닫음(`-`).
- 불변식 2: `--ephemeral`("Skips persisting session rollout files to disk"). 실행기 scratch cwd이고, `--skip-git-repo-check`는 scratch가 Git 저장소가 아니어서 필요합니다.
- 불변식 1: `--sandbox read-only`(문서 기본값과 같음)로 파일 쓰기를 막습니다. Stellar MCP 서버를 연결하는 공식 설정은 이번 실행에서 확인하지 못했습니다(비활성 사유).
- 연결 상태: `login status`, 종료 코드 규칙. 종료 코드 의미는 미확인입니다(비활성 사유).
- 환경 변수 이름과 근거:
  - `HOME`: CLI가 자기 설정·로그인 상태를 사용자 홈 기준으로 찾음(문서: 저장된 CLI 인증 재사용). Stellar는 그 위치를 읽지 않음.
  - `PATH`: 표준 실행 파일 탐색 경로(POSIX).
  - `USER`: 사용자 이름 표준 변수(POSIX).
  - `LOGNAME`: 로그인 이름 표준 변수(POSIX).
  - `TMPDIR`: macOS 사용자별 임시 디렉터리(Apple `confstr(3)`).
  - `LANG`: POSIX locale.
  - `LC_ALL`: POSIX locale.
  - `LC_CTYPE`: POSIX locale.
  - `CODEX_HOME`: 사용자가 바꾼 CLI 홈(문서의 `$CODEX_HOME/config.toml`)에서 CLI가 자기 설정·로그인을 찾음.
  - `HTTPS_PROXY`: 사용자 network proxy 설정.
  - `HTTP_PROXY`: 위와 같음.
  - `NO_PROXY`: 위와 같음.
- API key 환경 변수(`CODEX_API_KEY`, `OPENAI_API_KEY`)는 목록에 넣지 않습니다. 이 변수들은 사용자 로그인 대신 API key를 쓰는 경로이므로 ADR-0005 §4.1 범위 밖입니다.
- 약관·문서 검토: 2026-11-23, 약관 결과 `pending` → 비활성.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-agent-host` | 통과(아래 표의 대상별 수, 실패 0) |
| `cargo clippy -p stellar-agent-host --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-agent-host -- --check` | 통과 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |
| `cargo xtask validate-adr --base edfea9db` | 종료 코드 0 |
| `STELLAR_CHECK_BASE=edfea9db cargo xtask check-core` | lane 최종 실행 결과는 lane 보고(통합 기록)에 적음 |

| 검사 | 결과 |
|---|---|
| profile별(대역 claude·codex) 테스트 대역 세션에서 프로젝트 변경 0, 사용자 설정 동일 | 통과 |
| 비활성 profile(대역 profile을 끔, 공식 profile) 실행 거부, 버전 확인도 실행 0 | 통과 |
| 지원 범위 밖 버전(2.2.0) 거부, 세션 실행 0 | 통과 |
| 모든 profile에 환경 변수 이름 목록 존재, 값 0(`=`·object 항목 거부) | 통과 |
| 테스트 대역 세션의 child 환경 이름 집합 ⊆ profile 목록 | 통과 |
| 공식 profile 목록의 이름마다 이 보고서에 근거 항목 존재(test가 이 보고서를 읽어 검사) | 통과 |
| 모르는 key·약관 미승인 활성·모르는 decoder 버전 load 거부 | 통과 |

- 성능 예산 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- 공식 profile 두 개를 활성으로 만들지 않았습니다. 이유는 제품 소유자의 약관 검토와 공급자 서명 Team ID 확인이 없기 때문입니다(ADR-0005 §4.1). 비활성이 작업 지시의 실행 조건과 맞는 상태입니다.

## 남은 위험 / 한계

- **`[x]`로 올리기 위한 남은 일**:
  - 제품 소유자 약관 검토(`terms_result`)와 공급자 서명 Team ID(`team_id`) 확인
  - Codex의 Stellar MCP 연결 공식 설정 확인
  - `codex login status` 종료 코드 확인
  - 활성화 뒤 XXIX.6.8 수동 검증
- Claude Code profile의 env 근거 중 proxy 변수 문서 위치는 이번 실행에서 원문을 다시 열어 보지 않았습니다. 활성화 때 재확인합니다.
