# 작업 보고서: XIV.14.1 Git fixture harness

- WBS: XIV.14.1
- 작업명: 임시 local bare repo와 working repo를 만드는 Git fixture harness(외부 네트워크 미사용)
- 상태: 부분 완료 (`[~]`) — ADR-0006 4번 규칙. macOS에서 harness 구현·결정론(golden commit ID)
  검증을 마쳤습니다. 완료 기준의 "macOS/Windows 동일 scenario 생성"은 시스템 Git과 파일 시스템(플랫폼
  구현)에 의존하므로, Windows 재개 시 같은 golden을 Windows Git에서 확인해야 `[x]`가 됩니다.
- 선행: XIV.1.2 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증). `CI-WIN`·`LAB-WIN`은 ADR-0006으로 보류.

## 변경 파일 목록

- `crates/git-porcelain/src/fixture.rs`(신규) — `GitFixture`, `FixtureError`, `StandardScenario`,
  `build_standard_scenario`, `BASE_EPOCH`·`FIXTURE_NAME`·`FIXTURE_EMAIL`.
- `crates/git-porcelain/src/lib.rs` — `fixture` 모듈(`cfg(any(test, feature = "test-support"))`).
- `crates/git-porcelain/Cargo.toml` — 기본값이 꺼진 `test-support` feature.
- `docs/task-reports/XIV.14.1.md`(신규), 계획서(XIV.14.1 `[ ]`→`[~]`).

## 구현 요약

- **구조**: fixture root 아래 `remote.git`(bare, `main`)과 `work`(`origin` = `remote.git` 경로)를 시스템
  Git으로 만듭니다. `write`·`commit_all`·`tag_annotated`·`push`·`rev_parse`·`clone_bare`(두 번째 working
  저장소)·`add_worktree`(linked worktree)·`gated()`(trusted `GatedGitRunner`)를 제공합니다. drop 시 root를
  지우고 `keep()`으로 보존할 수 있습니다. 상대 경로는 snapshot 경로 규칙으로 검증합니다.
- **네트워크 없음**: `GIT_ALLOW_PROTOCOL=file`로 Git이 https·ssh·git 전송을 거부합니다. 원격은 항상
  fixture 안 bare 저장소입니다.
- **hermetic**: 환경 allowlist는 `PATH`뿐이고, `GIT_CONFIG_GLOBAL`·`GIT_CONFIG_SYSTEM` = null device,
  `GIT_CONFIG_NOSYSTEM=1`, `GIT_ATTR_NOSYSTEM=1`, `GIT_TERMINAL_PROMPT=0`, `HOME` = fixture 안 `home`.
- **결정론**: `--object-format=sha1`, `-b main`, `core.autocrlf=false`, `core.safecrlf=false`, 서명 끔,
  고정 author/committer(`Stellar Fixture <fixture@stellar.invalid>`), commit·tag마다 1분씩 증가하는 고정
  시각(`BASE_EPOCH` = 2026-01-01T00:00:00Z).
- **표준 scenario**: `initial` → `main`(home 수정), `initial` → `feature`(about 추가), annotated `v1` →
  `main`. 모든 ref를 bare에 push하고 `main`을 checkout한 상태로 끝납니다. 이후 Git corpus(XIV.14.*)와
  Merge Composer source fixture가 이 scenario를 씁니다.
- `test-support` feature는 기본값이 꺼져 프로덕션 빌드에 포함되지 않습니다(다른 crate는
  `[dev-dependencies]`에서만 켬).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | lib 268 passed + doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-git-porcelain --all-targets --features test-support -- -D warnings` | 경고 0 |
| `cargo check -p stellar-git-porcelain --features test-support` | 통과 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **동일 scenario 생성**: 독립 fixture 두 개에서 만든 표준 scenario의 commit·tag ID가 서로 같고 golden(SHA-1)과 일치 — initial `430418a2…51b5`, main `6073a623…bf4e`, feature `1cf4a59f…5203`, tag v1 `0d5b6c89…f0a` | 통과 |
| bare 저장소의 `main`·`feature`·`v1` ref가 working 저장소 값과 같음, HEAD = main, `v1^{commit}` = main | 통과 |
| 두 번째 clone·linked worktree가 같은 이력을 봄, clone commit도 고정 시각·identity 규칙 사용 | 통과 |
| **네트워크 미사용**: https·ssh·git URL push가 연결 시도 없이 즉시 실패(10초 미만), 전역 config 미노출, `HOME` 격리 | 통과 |
| 잘못된 상대 경로 5종 거부, drop 시 root 삭제, `keep()` 보존 | 통과 |

## 차이(작업 지시와 다른 점)

- Windows에서 같은 scenario를 생성하는 검증은 ADR-0006(Windows 보류)으로 실행하지 않았습니다. 그래서
  상태를 `[~]`로 두었습니다.

## 남은 위험 / 한계

- Windows 재개 시 같은 golden ID가 Git for Windows에서 나오는지(`core.autocrlf=false`, 실행 비트·symlink
  없는 scenario) `CI-WIN`에서 확인해야 합니다.
- 기존 crate 내부 `test_support`(XIV.* 단위 테스트용)는 그대로 두었습니다. 새 corpus 테스트는 이
  harness를 사용합니다.
- golden은 Git 기본 commit 형식에 의존합니다. 향후 Git이 commit header 형식을 바꾸면 golden을 다시
  고정해야 합니다.
