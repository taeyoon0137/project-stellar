# 작업 보고서: IV.3.1 Git executable 탐색 순서

- WBS: IV.3.1
- 작업명: Git executable 탐색 우선순위(사용자 절대 경로 → 알려진 시스템 경로 → PATH), shell 미사용
- 상태: 완료 (`[x]`)
- 선행: Phase II

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/git-porcelain/src/discovery.rs` (신규: `git_candidates`, `select_existing`, `discover_git`, `GitDiscoveryConfig`, 테스트)
- `crates/git-porcelain/src/lib.rs` (`pub mod discovery;`)
- `docs/task-reports/IV.3.1.md`(신규), 계획서(IV.3.1 `[ ]`→`[x]`)

## 구현 요약

- `git_candidates`는 우선순위 순서로 후보 경로를 만듭니다: (1) 사용자 지정 **절대** 경로
  (절대일 때만), (2) 알려진 시스템 경로(플랫폼별 cfg), (3) `PATH` 각 디렉터리 + Git 이름.
- **shell을 거치지 않습니다**: `which`·shell 해석 없이 경로를 직접 만들고 존재를 확인합니다.
- `select_existing`은 존재 검사를 주입받아 후보 중 처음으로 존재하는 경로를 고릅니다(우선순위
  결정론적 테스트 가능). `discover_git`은 실제 파일시스템(`is_file`)으로 탐색합니다.
- 플랫폼별 Git 이름(`git`/`git.exe`)·알려진 경로(macOS/Windows/기타 unix)를 cfg로 분기.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-git-porcelain` (5 unit) | 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 0 |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| 사용자 절대 경로가 후보 첫 번째 | 통과 |
| 상대 사용자 경로는 무시 | 통과 |
| 우선순위 순서(사용자 < 알려진 < PATH) | 통과 |
| 존재하는 후보 중 우선순위 앞 선택, 없으면 None | 통과 |
| PATH 없어도 알려진 경로 후보 생성 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 탐색 순서·shell 미사용을 충족합니다.

## 남은 위험 / 한계

- 실제 실행(`git --version`)과 버전 파싱은 IV.3.2에서 추가합니다. 본 작업은 경로 탐색에 한정.
- 알려진 시스템 경로 목록은 일반적 설치 위치 기준이며, 비표준 설치는 사용자 지정 경로 또는
  PATH로 처리합니다.
