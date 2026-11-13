# 작업 보고서: XIV.1.1 trusted 프로젝트 전용 GitRunner gate

- WBS: XIV.1.1
- 작업명: trusted project에서만 GitRunner 활성화, untrusted Git command request는 `ProjectNotTrusted`로 거부
- 상태: 완료 (`[x]`)
- 선행: IV.5.3(비신뢰 probe), IV.3.4(Git 명령 결과)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증; 악성 repo fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/trust.rs` (신규: `GitTrust`, `GatedGitRunner`, `GatedRunError::ProjectNotTrusted`, 악성 repo fixture 테스트)
- `crates/git-porcelain/src/runner.rs` (`GitRunner::spawn_count` 계측 추가, 사본 간 카운터 공유, 계측 테스트)
- `crates/git-porcelain/src/test_support.rs` (신규, 테스트 전용: TempDir, hermetic 실행기, fixture 헬퍼)
- `crates/git-porcelain/src/lib.rs` (`pub mod trust;`, `#[cfg(test)] mod test_support;`)
- `docs/task-reports/XIV.1.1.md`(신규), 계획서(XIV.1.1 `[ ]`→`[x]`)

## 구현 요약

- 신뢰 상태의 원본은 project crate trust store(IV.5.2)이지만 git-porcelain은 project crate에 의존할
  수 없으므로(crate graph 정책), crate 내부에 `GitTrust { Trusted, NotTrusted }`를 두고 호출자가
  변환해 공급하는 gate `GatedGitRunner`를 만들었습니다.
- `GatedGitRunner::run`/`runner()`는 `NotTrusted`이면 **프로세스를 만들기 전에**
  `GatedRunError::ProjectNotTrusted`를 반환합니다. `set_trust`로 신뢰 부여·철회를 반영합니다.
  gate는 메모리 상태만 가지며 Git 전역 설정을 변경하지 않습니다.
- `GitRunner`에 실제 `spawn()` 시도 횟수를 세는 `spawn_count()`를 추가했습니다(`Arc<AtomicU64>`,
  `Clone` 사본 간 공유). gate 거부는 spawn 전이므로 세지 않습니다. 기존 `new`/`run` 시그니처는
  바꾸지 않았습니다.
- 후속 저장소 단위 연산(XIV.1.2 이후)은 `GatedGitRunner`를 받도록 해 gate 우회 경로를 두지
  않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 49 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |
| `cargo xtask validate-crate-graph` | exit 0 |
| `cargo check -p stellar-project --all-targets` (하위 사용 crate 컴파일 확인) | exit 0 |

| 검사 | 결과 |
|---|---|
| **악성 repo fixture(fsmonitor hook, clean/smudge filter, pre-commit hook) + untrusted: status·add·commit·diff·rev-parse·log 6개 요청 전부 `ProjectNotTrusted`, `spawn_count == 0`, marker 0개** | 통과 |
| 대조군: 같은 fixture + trusted에서 같은 요청 실행 시 3개 marker 모두 생성(fixture가 실제로 악성임을 확인) | 통과 |
| 존재하지 않는 executable + untrusted: spawn 시도 0 / trusted 전환 후 spawn 1 / 철회 후 다시 0 증가 | 통과 |
| `spawn_count`가 실패한 spawn 시도도 세고 `Clone` 사본과 공유 | 통과 |

## 차이(작업 지시와 다른 점)

- 신뢰 상태 자체(trust store)는 project crate에 있으므로, 이 작업은 git-porcelain 쪽 gate와
  계측만 구현했습니다. project crate의 `TrustStatus` → `GitTrust` 변환 연결은 호출자(project/app
  계층) 몫이며 이 작업에서 다른 crate를 수정하지 않았습니다.
- 기존 IV.4·IV.6 함수(`git_init_main`, `git_clone` 등, 생성·clone 직후 사용)는 `&GitRunner`를
  그대로 받습니다. 기존 동작 보존을 위해 이 작업에서 시그니처를 바꾸지 않았습니다.

## 남은 위험 / 한계

- `GitRunner`를 직접 받는 기존 함수는 gate를 거치지 않습니다. 신뢰되지 않은 기존 저장소에 대해
  이 함수들을 호출하지 않도록 하는 것은 호출자 책임입니다(신규 XIV 연산은 gate만 받음).
- untrusted 모드 capability matrix 전체(XVI.7.1)와 commit hook 정책(XIV.8.5, XVI.7.3)은 후속
  작업입니다.
