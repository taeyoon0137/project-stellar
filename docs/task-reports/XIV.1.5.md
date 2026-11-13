# 작업 보고서: XIV.1.5 Git 출력 UTF-8 우선 디코딩과 `-z` parse

- WBS: XIV.1.5
- 작업명: stdout/stderr UTF-8 우선 디코딩, raw bytes 보존, invalid sequence replacement 구분, machine parse는 `-z`·stable format만 사용
- 상태: 완료 (`[x]`)
- 선행: IV.3.4(Git 명령 결과)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증; non-UTF8 경로 fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/decode.rs` (신규: `GitText`, `TextEncoding`, `split_nul_terminated`, `ZParseError`, `GitPath`, `parse_nul_paths`, `path_from_git_bytes`, 테스트)
- `crates/git-porcelain/src/command.rs` (`GitCommandResult::stdout_text`/`stderr_text` 추가)
- `crates/git-porcelain/src/location.rs` (내부 경로 변환을 `decode::path_from_git_bytes`로 교체, 동작 동일)
- `crates/git-porcelain/src/lib.rs` (`pub mod decode;`)
- `docs/task-reports/XIV.1.5.md`(신규), 계획서(XIV.1.5 `[ ]`→`[x]`)

## 구현 요약

- `GitText`는 원시 바이트(정본)와 표시용 텍스트를 함께 가집니다. 유효한 UTF-8이면
  `TextEncoding::Utf8`, 아니면 U+FFFD로 바꾼 표시 텍스트와 바뀐 invalid sequence 수를
  `TextEncoding::Lossy { replaced_sequences }`로 구분합니다. `as_utf8()`은 유효한 UTF-8일 때만
  값을 주므로 대체 문자 텍스트가 경로·식별자로 다시 쓰이지 않습니다.
- `split_nul_terminated`는 `-z` 출력을 NUL 종료 레코드로 나눕니다. 마지막 레코드가 NUL로 끝나지
  않으면 잘린 출력으로 보고 `ZParseError::Unterminated`(offset·남은 바이트 보존)를 반환합니다.
- `GitPath`는 경로 원시 바이트를 보존하고, `to_path_buf`로 unix는 바이트 그대로, Windows는
  UTF-8일 때만 로컬 경로로 바꿉니다. `parse_nul_paths`는 `-z` 경로 목록 parser입니다.
- `-z` 없는 경로 출력은 `core.quotePath`로 C 스타일 quote되므로 machine parse에 쓰지 않는다고
  모듈 문서에 명시하고, 테스트로 그 차이를 확인합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 75 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |
| `cargo xtask validate-crate-graph` | exit 0 |
| `cargo check -p stellar-project --all-targets` | exit 0 |

| 검사 | 결과 |
|---|---|
| **non-UTF8 filename fixture**: index에 `caf\xe9.txt`(Latin-1)와 `디자인.txt` 경로 등록·커밋 → `ls-files -z`, `ls-tree -r -z --name-only HEAD` 모두 원시 바이트 그대로 parse, Latin-1 경로는 `Lossy{1}` 표시(`caf\u{FFFD}.txt`)·`to_path_buf` 바이트 보존, 한글 경로는 `Utf8` | 통과 |
| 대조: `-z` 없는 `ls-files`는 `\351`로 quote되어 원시 바이트가 나타나지 않음 | 통과 |
| invalid sequence 수 집계(`a\xffb\xfe\xfdc` → 3, 잘린 `\xe2\x82` → 1), 원시 바이트 보존, `as_utf8() == None` | 통과 |
| `-z` 분해: 빈 입력 0개, 개행 포함 레코드 보존, 빈 레코드, NUL 미종료 → `Unterminated` | 통과 |
| 실제 Git stderr(`rev-parse --verify` 실패) → `Utf8`, 원시 바이트 일치 | 통과 |

## 차이(작업 지시와 다른 점)

- macOS APFS는 UTF-8이 아닌 파일 이름을 만들 수 없으므로, non-UTF8 fixture는 작업 트리 파일 대신
  `git hash-object -w`로 blob을 만들고 `git update-index --add --cacheinfo`로 원시 바이트 경로를
  index에 등록한 뒤 커밋했습니다. Git 경로 출력 처리는 같은 경로를 거칩니다.

## 남은 위험 / 한계

- Windows는 UTF-8이 아닌 Git 경로를 로컬 경로로 바꾸지 않습니다(`to_path_buf == None`). Windows
  fixture는 이 환경에서 실행하지 않았습니다.
- 개별 명령 parser(status, ls-tree 등)는 후속 작업(XIV.2.x)에서 이 모듈 위에 구현합니다. trace
  redaction(XIV.1.6)은 이 작업 범위가 아닙니다.
