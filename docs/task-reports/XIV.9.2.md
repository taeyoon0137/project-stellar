# 작업 보고서: XIV.9.2 GitHub URL 형식 검증

- WBS: XIV.9.2
- 작업명: GitHub URL 형식 검증은 HTTPS/SSH canonical patterns만 확인하고 API 호출은 하지 않음
- 상태: 완료 (`[x]`)
- 선행: XIV.9.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증; 네트워크·Git 실행 없음).

## 변경 파일 목록

- `crates/git-porcelain/src/remote/github.rs` (신규: `classify_github_url`, `GitHubUrlCheck`, `GitHubRepo`, `GitHubUrlForm`, `GitHubUrlProblem`, 테스트)
- `crates/git-porcelain/src/remote/mod.rs` (`github` 모듈)
- `docs/task-reports/XIV.9.2.md`(신규), 계획서(XIV.9.2 `[ ]`→`[x]`)

## 구현 요약

- 문자열 검사만 수행합니다(네트워크·API·Git 실행 없음). canonical 형식은 `https://github.com/<owner>/<repo>[.git]`,
  `git@github.com:<owner>/<repo>[.git]`, `ssh://git@github.com/<owner>/<repo>[.git]` 세 가지입니다.
- host가 정확히 `github.com`이 아니면 `NotGitHub`(외부 터미널로 등록하는 remote, 아키텍처 §25.3).
  `github.com.evil.example`·대문자 host도 `NotGitHub`입니다.
- GitHub host이지만 canonical이 아니면 `Invalid(이유)`: URL 내 자격 증명(token이 Git config에 남으므로 거부),
  `http://`, port, query·fragment, 추가 경로·끝 `/`, owner 규칙(영숫자·`-` 1~39자, `-`로 시작·끝 불가),
  repo 규칙(영숫자·`.`·`_`·`-` 1~100자, `.`/`..` 불가, 끝 `.git` 한 번 제거), SSH 사용자 `git` 아님.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 168 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **github fixture** 7종(HTTPS·`.git` 유무·SCP·SSH URL·owner/repo 문자) → `GitHub{form, owner, repo}`(`canonical_github_urls_are_accepted`) | 통과 |
| **non-github fixture** 11종(gitlab·유사 host·대문자 host·로컬 경로·file URL·빈 문자열 등) → `NotGitHub` | 통과 |
| GitHub host 비canonical 23종 → 이유별 `Invalid`(자격 증명·http·port·query·경로·owner·repo·SSH 사용자) | 통과 |

## 차이(작업 지시와 다른 점)

- GitHub host이지만 형식이 맞지 않는 URL을 `NotGitHub`와 구분해 `Invalid(이유)`로 돌려줍니다. UI가
  "GitHub URL 형식 오류"와 "GitHub가 아닌 remote(외부 터미널 사용)"를 다르게 안내하기 위해서입니다.

## 남은 위험 / 한계

- GitHub Enterprise·`www.github.com`·SSH over 443(`ssh.github.com`)은 canonical로 보지 않습니다.
- owner의 연속 `-` 금지 등 GitHub 서버의 세부 규칙은 검사하지 않습니다(존재 여부는 XIV.9.3 `ls-remote`).
