# 작업 보고서: XXIV.4.5 `stellar-ops` 교차 프로세스 저장 잠금

- WBS: XXIV.4.5
- 작업명: headless 쓰기 경로에 프로젝트·Git 밖 OS 사용자 데이터의 프로젝트별 배타 잠금을 두고, 잠금을 잡은 뒤 generation 비교·교체, 사용 중이면 종료 코드 75
- 상태: 부분 완료 (`[~]`) — 구현과 작업 행의 검증 기준은 모두 통과했으나, 선행 XXIV.4.4가 `[~]`입니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`가 아니면 완료로 두지 않음)에 따라 XXIV.4.4가 `[x]`가 될 때까지 `[~]`로 둡니다.
- 선행: XXIV.4.4(이 lane에서 구현, `[~]`: 남은 일은 storage writer 종류 필드뿐이며 잠금이 쓰는 저장 경로는 모두 있음), IV.2.3(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (macOS `flock(2)`, 로컬 검증).

## 변경 파일 목록

- `crates/ops/src/lock.rs`(신규) — `LockPolicy`(Required/None), `SaveLock::try_acquire`, `default_lock_dir`, `lock_key`, `LockError`.
- `crates/ops/src/apply.rs` — `apply`에 `lock: &LockPolicy` 인자, 쓰기 모드에서 잠금을 잡은 뒤 snapshot을 열어 base revision 비교·저장, `ApplyError::Lock`, 잠금 test 3개.
- `crates/ops/src/lib.rs` — `lock` 모듈.
- `tools/stellar-cli/src/apply.rs` — 쓰기 모드에서 `default_lock_dir()` 잠금 사용, 종료 코드 분류(75·78·74).
- `tools/stellar-cli/tests/output_fixtures.rs` — `run_with_stdin`에 `HOME` 지정, 잠금 test 3개.
- `tools/stellar-cli/tests/fixtures/results/apply-lock-busy.json`, `apply-no-user-data-dir.json`(신규).
- `docs/task-reports/XXIV.4.5.md`(신규), 계획서(XXIV.4.5 `[ ]`→`[x]`).

## 구현 요약

- **순서**: 쓰기 모드 `apply`는 command 해석 → `stellar.yaml` probe로 project id 확인 → **잠금 획득** → snapshot 열기(generation 읽기) → base revision 비교 → 적용 → 세대 전환 저장(V.5) → 잠금 해제(drop) 순입니다. 잠금 안에서 generation 비교와 current pointer 교체가 일어나므로 headless writer끼리는 같은 base 위의 동시 저장이 생기지 않습니다. 후속 runtime 통합으로 앱 창 Save Now도 같은 잠금을 잡아 base 확인부터 pointer 교체까지 직렬화합니다.
- **위치**: `<OS 사용자 데이터>/xyz.taeyoon.stellar/locks/<key>.lock`. OS 사용자 데이터는 기존 `stellar_project::trust_store::os_user_data_base`(macOS `$HOME/Library/Application Support`)를 그대로 쓰고, 하위 폴더는 앱 bundle identifier(`apps/macos` 프로젝트의 `PRODUCT_BUNDLE_IDENTIFIER`)입니다. Apple File System Programming Guide는 Application Support 아래 앱 데이터를 bundle identifier 이름의 하위 폴더에 두라고 안내합니다. 디렉터리는 `0700`, 잠금 파일은 `0600`으로 만들고 내용은 쓰지 않습니다.
- **key**: `project id ‖ canonical target`(IV.2.3 `ProjectIdentity`)의 SHA-256 앞 16바이트 16진수. symlink 경로로 열어도 같은 잠금이고 다른 worktree는 다른 잠금입니다.
- **OS API**: Rust std `File::try_lock`(Unix에서 `flock(2)` `LOCK_EX | LOCK_NB`). Apple `flock(2)` 매뉴얼 기준으로 advisory 잠금이며 open file description에 붙어, 그 description을 가리키는 마지막 fd가 닫히면(프로세스 비정상 종료 포함) 해제됩니다. 그래서 남은 잠금 파일이 다음 writer를 막지 않습니다. 같은 프로세스 안에서도 따로 연 fd끼리 경합합니다(ops thread test로 확인).
- **사용 중**: 기다리지 않고 `LockError::Busy` → `stellar apply`는 `save-lock-busy`, 종료 코드 75(`EX_TEMPFAIL`)이며 아무것도 읽거나 쓰지 않습니다. `HOME`이 없어 위치를 정할 수 없으면 78(`no-user-data-dir`)입니다.
- **dry-run과 merge driver 모드**: dry-run은 쓰지 않으므로 잠금을 잡지 않습니다. Git merge driver 모드(임시 경로만 씀, ADR-0005 §2)를 위한 `LockPolicy::None`을 두었고, 이 정책은 잠금 디렉터리를 만들지도 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops` | 23 passed, 0 failed |
| `cargo test -p stellar-cli` | unit 19, headless_help 6, json_copy_sync 2, output_fixtures 18 passed, 0 failed |
| `cargo clippy -p stellar-cli -p stellar-ops --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt --all -- --check` | 통과 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| 동시 쓰기 손실 0(thread): 6개 writer가 서로 다른 node 이름 변경, 잠금 사용 중·base 불일치면 재시도, 계획 뒤 교체(`ConcurrentWrite`)는 즉시 실패 → generation 1+6, 모든 이름 보존(`concurrent_writers_lose_no_update`) | 통과 |
| 동시 쓰기 손실 0(프로세스): `stellar apply` 4개 프로세스 동시 실행, 75 `save-lock-busy`·65 `base-revision-mismatch`만 재시도하고 65 `concurrent-write`를 포함한 다른 결과는 실패 → revision `g5:`, 모든 이름 보존(`concurrent_apply_processes_lose_no_update`) | 통과 |
| CLI가 계획 뒤 교체를 base 불일치와 다른 코드 `concurrent-write`(65)로 보고(`concurrent_write_is_distinguished_from_base_mismatch`) | 통과 |
| 잠금 디렉터리 자리가 symlink·파일이면 `NotADirectory`로 거부, symlink 대상 권한 무변경·잠금 파일 0(`symlinked_lock_dir_is_refused_without_touching_the_target`) | 통과 |
| 잠금 경합 시 exit 75, golden `apply-lock-busy.json`, 파일 hash 동일, 해제 후 성공(`apply_exits_75_while_another_writer_holds_the_save_lock`) | 통과 |
| ops: 잠금 보유 중 쓰기 `Lock(Busy)`·파일 변경 0, dry-run은 잠금 무관하게 동작(`held_lock_makes_write_fail_without_touching_files_and_dry_run_skips_the_lock`) | 통과 |
| merge driver용 `LockPolicy::None` 쓰기는 잠금 디렉터리를 만들지 않음(`lock_policy_none_never_creates_the_lock_dir`) | 통과 |
| 잠금 파일이 프로젝트 밖에만 존재, 디렉터리 `0700`(`second_holder_is_busy_until_the_first_is_dropped`, 명령 test의 프로젝트 digest 검사) | 통과 |
| symlink 경로 같은 key, 다른 worktree 다른 key(`key_follows_project_identity`) | 통과 |
| `HOME` 없는 쓰기 78, dry-run 성공(`apply_write_without_user_data_dir_is_config_error_and_dry_run_still_works`) | 통과 |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다.

### 앱 Save Now와의 상호 운용 후속 검증

- 2026-11-15 검토에서 `DEV-MAC`의 `cargo test --workspace --no-fail-fast`(종료 0, 4,039 통과·0 실패)로 아래 두 runtime 테스트가
  통과하는 것을 다시 확인했습니다. 처음 적은 crate별 통과 수는 다시 확인하지 않아 지웠습니다.
- runtime의 `cli_generation_lock_and_window_save_share_one_identity`는 임시 사용자 데이터와 별도 프로세스로
  동일 경로·symlink의 경합, 잠금 해제 후 저장, HOME이 없어도 가능한 무변경 저장과 undo로 원상 복원한 저장을 검사합니다.
  `saving_after_a_symlink_is_repointed_updates_the_opened_project`는 열린 target에만 기록되는지 검사합니다.
- (2026-11-14 통합 때의 보고이며 이 검토에서 다시 실행하지 않았습니다.) 저장된 창 fixture를 복사해 실제 `stellar apply`와 Rust 창 session을 별도 프로세스로 실행했습니다.
  앱 측에서 프로젝트 잠금을 보유한 동안 CLI는 75를 반환했고, 해제 후 2세대를 기록했습니다. 먼저 열었던
  앱의 저장은 stale로 실패하고 편집을 유지했습니다. 새 앱 session은 다른 프로세스가 잠금을 보유하면
  dirty 상태를 유지했고, 해제 후 같은 session에서 재시도하여 이름 변경을 3세대로 저장했습니다.

## 차이(작업 지시와 다른 점)

- Git merge driver 모드 자체(XXIV.2.1)는 아직 없으므로 "merge driver 모드 잠금 미사용"은 그 모드가 쓸 `LockPolicy::None` 경로가 잠금을 만들거나 잡지 않음을 확인한 것입니다. XXIV.2.1은 이 정책을 써야 합니다.

## 남은 위험 / 한계

- 같은 OS 사용자 데이터의 잠금을 사용하는 generation writer에 적용합니다. 다른 사용자나 외부 Git 변경에 대한 semantic reconciliation은 XXIV.4.6 범위입니다.
- **후속(`[x]`로 올리기 위한 남은 일)**: XXIV.4.4의 generation metadata writer 종류 기록(보호 경로 `crates/storage`와 ADR)을 마쳐 XXIV.4.4를 `[x]`로 올린 뒤, 이 보고서의 검증 명령을 다시 실행해 `[x]`로 바꿉니다.
- 동시 쓰기 test는 잠금이 직렬화한 결과의 손실 0을 확인하지만, 잠금 없이 손실이 나는 경합을 결정적으로 재현하지는 못합니다. 대신 V.5.2 optimistic check만으로 통과하지 않도록, 잠금이 감싸야 하는 계획 뒤 교체(`ConcurrentWrite`, CLI `concurrent-write`)가 한 번이라도 나오면 두 test 모두 실패합니다. base 불일치 재시도는 base를 잠금 밖에서 읽는 호출 방식 때문에 정상입니다.
- 잠금 디렉터리의 상위 경로(`$HOME`, Application Support)는 symlink 여부를 검사하지 않습니다. 같은 사용자 `HOME` 안이라 위험이 낮습니다.
- 기다리지 않는 정책이라 경합이 잦으면 호출자가 재시도해야 합니다(75는 재시도 가능 코드).
