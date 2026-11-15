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
- 아키텍처 §26, 구현 계획, 이 보고서. 새 dependency·실행기·UI/FFI 연결은 없습니다.

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
- macOS에서 Apple/upstream Git 2.54.0만 승인합니다. GNU/Linux는 실행하지 않았으며 production도 진단으로 거부합니다. 다른 Git 버전도 버전별 검증 전에는 거부합니다.
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

## 차이

- [Git update-index v2.54.0](https://github.com/git/git/blob/v2.54.0/builtin/update-index.c#L1047-L1055)은 lock 실패 뒤에도 index를 읽을 수 있습니다. 따라서 파일 존재가 아닌 실제 자식 FD 소유를 확인합니다. 수정·commit·잠금 정리는 Git만 수행합니다.
- [Git lockfile API](https://git-scm.com/docs/api-lockfile)의 exclusive-create/rename 규약을 지키는 writer를 대상으로 합니다. 검증되지 않은 OS/버전/flags는 우회하지 않습니다.
- 초기 차단 당시의 재현은 commit `39bf1612`에 남아 있습니다. 해소된 차단 보고서는 2026-11-15 검토에서 삭제했습니다. 기존 부분 선택·직렬화는 재구현하지 않았습니다.

## 남은 위험

- 입력 승인 뒤 timeout·취소·Git 실패·재조회 불일치는 결과 미확정입니다. 이 단계는 무조건 rollback하지 않습니다. 성공이 확인된 publish의 조건부 복원과 semantic 검증 실패 처리(XIV.6.4/6.5)는 후속 단위입니다.
- selected assume/skip/ITA 등 flags, split/sparse index, Git 저장소/index 환경 경로 재지정, Linux/Windows·다른 Git 버전은 지원하지 않습니다. unrelated flags/REUC는 유지합니다.
- 후속 입력 보강은 XIV.6.4 보고서에 기록했습니다. 현재 적용·복원 양방향의 보수적 index-info 길이가 512 bytes를 넘으면 입력 전에 거부합니다.
- 직접 index/lock을 강제 조작하는 writer 및 SIGKILL 뒤 잔류 lock을 자동 삭제하지 않습니다. FD 조회 권한·형식이 불확실하면 입력을 보내지 않습니다.
- 동기 filesystem 읽기 자체는 OS에서 막힐 수 있습니다. Git 자식 실행/승인 대기에는 deadline·취소가 적용됩니다. 실제 앱 QA·CI·REF·release는 실행하지 않았습니다.

## 2026-11-15 재검증

- Codex 작업 검토(계획 B9)에서 이 행의 상태를 다시 정했습니다. 위 기록은 Codex 작업 시간에 작성·통합되어 상태의 근거로 쓰지 않고, 아래 실행 결과를 근거로 씁니다.
- 논리 검토: 계획 U047·U048 수정. B3에서 격리 작업 트리와 hook 차단을 지우고(`dd1a871f`) Git 2.54.0 게이트를 되살렸습니다(`f5ffbc5d`).
- 환경: `DEV-MAC`(Apple M3 Max, macOS 27.2, rustc 1.96.0, Apple Git 2.54.0). 코드 기준은 `3463761b`이며 이 검토의 문서 커밋은 코드를 바꾸지 않습니다.

| 명령 | 결과 |
|---|---|
| `cargo test --workspace --no-fail-fast` | 0 — 4,039 통과, 0 실패, 10 ignored(수동 측정·장기 fuzz) |

- 검증 기준(partial same-file stage fixture): `staging::tests::actual_partial_stage_reopens_index_and_preserves_working_bytes_and_memory_revision`과 `index_publish::tests` 24개(`unverified_git_version_is_rejected_before_index_update` 포함) 통과.
- 판정: `[x]` 유지.
