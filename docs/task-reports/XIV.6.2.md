# 작업 보고서: XIV.6.2 Git 소유 잠금 아래 실제 index 적용

## WBS

- WBS: XIV.6.2
- 작업명: 검증된 mode/OID 후보를 공식 update-index로 적용
- 상태: `[x]` — 아래 macOS·Git 버전·index 지원 범위에서 완료
- 선행: XIV.6.1 (`[x]`)

## 변경 파일

- `crates/git-porcelain/src/runner.rs`, `trust.rs`: 기존 runner의 stdin 승인 경계. 승인 전 EOF도 보내지 않으며 기존 timeout·취소·process-group 종료·trace를 사용합니다.
- `crates/git-porcelain/src/index_publish.rs`, `index_publish/ownership.rs`: 자식 FD 소유 증명, 잠금 안 snapshot/bytes/flags 검사, 단일 NUL index-info 적용, Index provider 재조회.
- `index_publish/tests.rs`, `snapshot/index.rs`, `blob_write.rs`, `lib.rs`: 회귀 검증·취소 가능한 기존 provider·모듈 등록·테스트 생성자.
- `crates/runtime/src/staging/tests.rs`: 기존 준비·직렬화·blob pipeline을 재사용한 실제 A/B 부분 stage 검증.
- 아키텍처 §26, 구현 계획, 이 보고서와 기존 차단 보고서 요약. 새 dependency·실행기·UI/FFI 연결은 없습니다.

## 명령과 종료 코드

Cargo 명령은 모두 `CARGO_TARGET_DIR="$PWD/target/semantic-index-publish"`로 실행했습니다.

| 명령 | 결과 |
|---|---|
| `cargo check -p stellar-git-porcelain` (최초 연결) | 101 — 기존 validate_entries 인자 형식 불일치. D/F 검사를 올바른 path-prefix 검사로 수정 |
| `cargo test -p stellar-git-porcelain index_publish:: -- --nocapture` | 0 — 최종 14개 통과 |
| 같은 명령, `STELLAR_INDEX_PUBLISH_TEST_GIT=<임시 upstream git-2.54.0/git>` | 0 — 14개 통과 |
| `cargo test -p stellar-git-porcelain runner:: -- --nocapture` | 0 — 22개 통과 |
| `cargo test -p stellar-git-porcelain -- --nocapture` | 0 — 344 unit + 3 doctest 통과. compile_fail 예제의 의도된 컴파일 오류 포함 |
| `cargo test -p stellar-runtime staging::tests --no-default-features` | 0 — 4개 통과 |
| `cargo clippy -p stellar-git-porcelain -p stellar-runtime --all-targets --no-default-features -- -D warnings` | 최초 101(is_multiple_of 권고), 수정 후 0·경고 0 |
| `cargo fmt -p stellar-git-porcelain -p stellar-runtime -- --check` / `git diff --check` | 모두 0 |
| `cargo xtask validate-task-report docs/task-reports/XIV.6.2.md` | 0 |

## 환경과 증거

- `DEV-MAC`: macOS 27.2, Apple Git 2.54.0 (Apple Git-157), Rust/Cargo 1.96.0.
- upstream Git v2.54.0의 공식 소스를 새 임시 디렉터리에서 `make -j4 NO_GETTEXT=YesPlease NO_CURL=YesPlease NO_TCLTK=YesPlease NO_OPENSSL=YesPlease NO_EXPAT=YesPlease git`로 빌드(0)했습니다. 시스템 설치·config 변경은 없습니다.
- 최초 구현은 위 두 Git 2.54.0 실행 파일을 검증하고 패치 버전을 고정했습니다. 현재 macOS에서는 아래의 실제 명령과 잠금 검사를 사용합니다.
- 모든 실험·Git 변경은 새 GitFixture에 한정했습니다. 제품 저장소의 실제 index·worktree를 fixture로 쓰지 않았습니다. 컴파일 로그는 전용 target 아래 있습니다.

## 테스트

- cold start(index 파일 없음)에서 정상 Git 자식의 쓰기 FD가 `index.lock`의 device/inode와 일치하는지 macOS `proc_pidinfo`·`proc_pidfdinfo`로 확인합니다. FD가 없는 외부 잠금은 승인하지 않습니다. 고정 sleep이나 trace 메시지를 잠금 증거로 쓰지 않습니다.
- snapshot 캡처 이후 Git 잠금 이전의 same-path·양방향 D/F·flags-only 변경을 deterministic barrier로 주입합니다. 승인 입력 0, 외부 index bytes 유지입니다.
- 별도 Git writer가 잠금을 보유한 경우 및 우리 검증 중 경쟁 writer가 쓰려는 경우를 각각 검증합니다.
- 잠금 안에서 candidate의 전체 mode/OID/path/stage를 현재 Index provider와 비교합니다. 캡처한 실제 index bytes도 다시 비교하므로 flags/확장 변경을 놓치지 않습니다. 변경 경로의 nonzero flags, unmerged, split/sparse index는 진단합니다.
- unrelated staged 내용·assume-unchanged·skip-worktree·ITA·REUC 보존, SHA-256 삭제, symlink/executable, 정상 D/F 전환, linked worktree의 독립 index를 검증했습니다.
- 승인 전 취소·timeout·거부와 잘못된 마지막 레코드는 index bytes를 보존하고 Git이 lock을 정리합니다. Git 종료 후 외부 writer가 덮은 경우 `OutcomeUnknown`이며 역쓰기하지 않습니다.
- 실제 runtime fixture에서 같은 chunk의 A만 stage하고 B는 Working Tree에 남깁니다. Index provider → materialize → 기존 loader의 semantic 결과가 후보와 같고, Working Tree hash와 메모리 records/revision 7이 유지됩니다.

## 성능

- 전체 index bytes와 entries를 비교하는 O(index 크기) 작업입니다. 하나의 update-index 입력에 모든 경로를 넣으며, 승인 전 읽기 명령 비용은 별도입니다. REF 성능·대형 index benchmark는 실행하지 않았습니다.

## macOS Git 호환성 재검증

기준 `636034ba`에서 설치된 Fork Git 2.50.1의 빈 index 적용 검사는 버전 제한으로 종료 101이었습니다.
수정 후 각 실행 파일을 `STELLAR_INDEX_PUBLISH_TEST_GIT`로 지정하고
`cargo test --locked --offline -p stellar-git-porcelain index_publish::`를 실행했습니다.
각 검사는 독립 임시 저장소와 사용자 데이터 경로를 사용했습니다.

| 실행 파일 | 실제 버전 | 결과 |
|---|---|---|
| `/usr/bin/git` | 2.54.0 (Apple Git-157) | 16개 통과 |
| `/Applications/Fork.app/Contents/Resources/git-instance/bin/git` | 2.50.1 | 16개 통과 |

두 실행 파일 모두 실제 index 반영·재조회, D/F 전환, flags·REUC, SHA-256, linked worktree,
잠금 경합과 승인 전 timeout·취소에서 기준 index 및 Working Tree 보존을 검증했습니다.
macOS 쓰기 경로는 선택한 Git의 실제 `rev-parse`·`ls-files` 응답을 확인합니다.
`ls-files --debug -z`는 기준 index의 모든 경로에 대해 같은 순서로 해석 가능한 flags를 반환해야 합니다.
각 publish에서 `update-index` 자식이 실제 잠금 inode의 쓰기 FD를 보유한 상태에서 기준을 다시 검사하고
입력을 허용합니다. 미지원 명령·출력 형식·불분명한 잠금 소유는 승인 전에 중단합니다.
IV.3.5의 Git 설치 상태와 최소 버전 정책은 변경하지 않았습니다. 표에 없는 버전은 이번 실행의 검증 범위 밖입니다.

## 차이

- [Git update-index v2.54.0](https://github.com/git/git/blob/v2.54.0/builtin/update-index.c#L1047-L1055)은 lock 실패 뒤에도 index를 읽을 수 있습니다. 따라서 파일 존재가 아닌 실제 자식 FD 소유를 확인합니다. 수정·commit·잠금 정리는 Git만 수행합니다.
- [Git lockfile API](https://git-scm.com/docs/api-lockfile)의 exclusive-create/rename 규약을 지키는 writer를 대상으로 합니다. macOS에서 해석 가능한 index와 flags를 검사합니다.
- 이전 장문 재현은 commit `39bf1612`에 남기고 차단 보고서를 핵심 관찰로 줄였습니다. 기존 부분 선택·직렬화는 재구현하지 않았습니다.

## 남은 위험

- 입력 승인 뒤 timeout·취소·Git 실패·재조회 불일치는 결과 미확정입니다. 이 단계는 무조건 rollback하지 않습니다. 성공이 확인된 publish의 조건부 복원과 semantic 검증 실패 처리(XIV.6.4/6.5)는 후속 단위입니다.
- selected assume/skip/ITA 등 flags, split/sparse index, Git 저장소/index 환경 경로 재지정, Linux/Windows는 지원하지 않습니다. unrelated flags/REUC는 유지합니다.
- 직접 index/lock을 강제 조작하는 writer 및 SIGKILL 뒤 잔류 lock을 자동 삭제하지 않습니다. FD 조회 권한·형식이 불확실하면 입력을 보내지 않습니다.
- 동기 filesystem 읽기 자체는 OS에서 막힐 수 있습니다. Git 자식 실행/승인 대기에는 deadline·취소가 적용됩니다. 실제 앱 QA·CI·REF·release는 실행하지 않았습니다.
