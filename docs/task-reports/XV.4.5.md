# 작업 보고서: XV.4.5 resolve 결과 lockfile canonical 기록 transaction

- WBS: XV.4.5
- 작업명: resolve 성공 후 lockfile을 canonical write하고 autosave와 별도 project file transaction으로 기록, 자동 commit 없음
- 상태: 완료 (`[x]`)
- 선행: XV.4.1(`[x]`, 같은 lane에서 완료), IV.1.7(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 파일 시스템·시스템 Git, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/lockfile_write.rs`(신규) — `LOCK_FILE`, `LockfileWriteOutcome`,
  `LockfileWriteError`, `write_resolved_lockfile`, 실제 Git fixture 테스트.
- `crates/project/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.4.5.md`(신규), 계획서(XV.4.5 `[ ]`→`[x]`).

## 구현 요약

- **입력은 resolver 결과뿐**: `write_resolved_lockfile(project_root, &LockDocument)`.
  `Lockfile::from_document` → `write_checked`(모델 규칙·제한 loader 재로드 검사, ADR-0016)로 텍스트를
  만듭니다. 사용자 편집 필드를 받는 경로가 없습니다(IV.1.7). 텍스트를 만들 수 없으면 파일을 건드리지
  않습니다.
- **별도 project file transaction**: `design/` generation을 쓰는 문서 자동 저장(XIII.1)과 경로·상태를
  공유하지 않습니다. 같은 디렉터리 임시 파일(`.stellar.lock.yaml.tmp-<pid>-<n>`, `create_new`) →
  `fsync` → `rename` → 부모 디렉터리 `fsync` 순서입니다. 실패하면 임시 파일을 지우고 기존 lockfile을
  그대로 둡니다.
- **불필요한 변경 없음**: 기존 bytes가 같으면 쓰지 않습니다(`Unchanged`, mtime 유지). 기존 권한 bit를
  새 파일에 옮겨 Git mode 변경을 만들지 않습니다. lockfile이 symlink면 rename이 링크를 일반 파일로
  바꾸게 되므로 거부합니다(`SymlinkLockfile`, 링크와 대상 모두 유지).
- **자동 commit 없음**: Git을 실행하지 않습니다. 오류에는 단계와 io 종류만 있고 경로 원문은 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-project` | lib 265 passed, 통합 17 passed, 0 failed |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-project -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **Git status에 lockfile만 예상대로 변경**: scaffold를 commit한 실제 Git 저장소에서 기록 후 `git status --porcelain=v1 --untracked-files=all --ignored`가 정확히 ` M stellar.lock.yaml`, HEAD 불변(자동 commit 없음), `stellar.yaml` 불변, 기록 텍스트 canonical·실제 parser로 같은 문서, 같은 결과 재기록은 `Unchanged`(mtime 유지), 빈 결과로 되돌리면 status 깨끗(`resolved_lockfile_write_changes_only_the_lockfile_in_git_status`) | 통과 |
| 기존 권한(0600) 유지, symlink lockfile 거부·링크/대상 불변(`permissions_are_kept_and_symlinked_lockfile_is_not_replaced`) | 통과 |
| 쓰기 금지 root에서 `create-temp` 단계 `PermissionDenied`, 기존 lockfile 유지, 임시 파일 없음, status 깨끗, 오류 메시지에 경로 없음(`failed_write_keeps_existing_lockfile_and_leaves_no_temp_file`) | 통과 |
| 변이 확인: `Unchanged` 조기 반환을 끄면 첫 테스트가 실패함을 확인한 뒤 원복 | 확인 |

## 차이(작업 지시와 다른 점)

- 테스트는 구현과 같은 변경에서 작성했습니다. 실패 확인은 위 변이 확인으로 대신했습니다.
- 셸·runtime에서 resolve 완료 시 이 함수를 호출하는 연결은 이후 통합 작업 범위입니다. 이 작업은
  transaction 자체만 제공합니다.

## 남은 위험 / 한계

- 같은 프로젝트에 두 프로세스가 동시에 lockfile을 기록하는 경우의 직렬화는 다루지 않습니다. rename은
  원자적이라 파일이 깨지지는 않지만 마지막 기록이 이깁니다.
- Windows 디렉터리 fsync는 ADR-0006 보류로 생략합니다.
