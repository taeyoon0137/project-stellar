# 작업 보고서: XIV.6.4 원자적 다중 경로 적용과 조건부 복원

## WBS

- WBS: XIV.6.4
- 작업명: 다중 경로 transaction과 외부 변경을 덮지 않는 rollback
- 상태: `[x]` — 확인된 성공만 복원 증표를 발급하고 결과 미확정은 진단
- 선행: XIV.6.2 (`[x]`)

## 변경 파일

- `crates/git-porcelain/src/index_publish.rs`, `blob_write.rs`: 비공개 필드의 일회용 PublishedIndex, inverse mode/OID/path, 전체 index bytes 비교를 통한 조건부 복원.
- `runner.rs`, `index_publish/tests.rs`: Git index-read 준비 신호와 자식 FD 증명을 함께 사용하고 취소 정리 상태를 진단합니다.
- 구현 계획·아키텍처·이 보고서. 새 dependency·실행기는 없습니다.

## 명령과 종료 코드

전용 `CARGO_TARGET_DIR="$PWD/target/semantic-index-publish"`를 사용했습니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain index_publish:: -- --nocapture` (첫 복원 연결) | 101 — 17 passed/1 failed: 사전 경합 취소 뒤 index.lock 잔류 발견 |
| 같은 명령 (준비 신호 추가 직후) | 101 — 17/1: post-commit fault fixture가 hook 도달 전에 종료됨. 단독 실행은 통과; 해당 fixture deadline을 400ms→2s로 조정 |
| 같은 명령 (최종) | 0 — Apple Git 18개 통과 |
| 같은 명령, 임시 upstream Git 2.54.0 | 0 — 18개 통과 |
| compiled test binary의 `change_between_snapshot_and_lock_is_rejected_including_flags_only` 10회 | 모두 0 — 각 실행 4종 경합, lock 잔류 없음 |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 0·경고 0 |
| `cargo fmt -p stellar-git-porcelain` / `git diff --check` | 모두 0 |
| `cargo test -p stellar-git-porcelain` | 0 — 348 unit + 3 doctest 통과 |
| `cargo xtask validate-task-report docs/task-reports/XIV.6.4.md` | 0 |

## 환경과 증거

- `DEV-MAC`, macOS 27.2의 Apple Git 및 임시 빌드 upstream Git 2.54.0. 새 fixture만 사용했습니다.
- [Git v2.54.0 tempfile.c](https://github.com/git/git/blob/v2.54.0/tempfile.c)의 `create_tempfile_mode`는 파일 open 뒤 `activate_tempfile`에서 정리 handler를 등록합니다. FD 관찰만으로 즉시 취소하면 그 사이를 칠 수 있음을 실제 테스트에서 발견했습니다.
- [read-cache.c](https://github.com/git/git/blob/v2.54.0/read-cache.c)의 `read_index_from`에 있는 `index/do_read_index` region-leave는 없는 index에도 발생합니다. 자식 PID가 일치하는 Trace2 신호를 준비 완료 확인에만 쓰며, lock 소유는 별도의 macOS FD device/inode/FWRITE 검사로 증명합니다.

## 테스트

- 잘못된 마지막 index-info 레코드의 경우 Git 자체 rollback으로 원본 index bytes가 유지됩니다. 한 번의 입력에서 일부 path만 commit하지 않습니다.
- 성공 확인과 provider 읽기 전후 index bytes가 일치해야 복원 증표가 나옵니다. 증표는 복제·외부 생성이 불가능하고 해당 worktree에 묶입니다.
- 복원은 같은 승인 경로로 inverse 여러 path를 전달합니다. 원래 snapshot·mode·삭제·D/F 전환과 unrelated flags/REUC가 유지됩니다.
- 증표 이후 same-path·unrelated 내용·flags-only 변경은 StaleBase로 거부하고 외부 bytes를 그대로 둡니다. 실제 복원도 잠금 안에서 다시 bytes/snapshot을 검사합니다.
- fixture 밖 hooks 디렉터리와 공식 Git config로 post-index-change 대기를 주입했습니다. Git commit 이후 취소/timeout은 실제 index가 반영됐어도 OutcomeUnknown이며 증표가 없습니다. 자동 역쓰기하지 않습니다.
- 조기 취소 뒤 남은 lock은 소유/정리 미확정 진단이며 직접 삭제하지 않습니다. 정상 준비 신호 이후 거부·취소의 정리를 검증했습니다.

## 성능

- O(index 크기) bytes와 snapshot을 증표에 보관합니다. 복원은 추가 update-index 1회입니다. REF 예산은 측정하지 않았습니다.

## 차이

- 사용자 요구에 따라 무조건 과거 tree를 쓰는 복원을 사용하지 않습니다. 확인된 성공 증표와 현재 전체 index 표현이 같을 때만 복원하고, 경합·미확정 상태는 구조화된 오류로 남깁니다.
- 성공 뒤 복원은 Git tree/mode/flags/REUC 보존이며 변경 항목의 stat cache bytes까지 과거로 되돌리는 것은 아닙니다. cold start 복원은 빈 index 파일을 남길 수 있습니다.

## 남은 위험

- 입력 승인 이후 결과 미확정, 조기 강제 종료의 잔류 lock, 권한 부족은 자동 삭제/복원하지 않습니다. 재조회와 사용자 진단이 필요합니다.
- Git lock 규약을 무시하는 직접 파일 교체나 강제 unlink는 지원 범위가 아닙니다. 지원 OS/버전/index 상태 제한은 XIV.6.2와 같습니다.
- 제품 semantic 실패 처리는 XIV.6.5에서 이 증표를 사용해 연결합니다. UI/FFI·원격·release 작업은 실행하지 않았습니다.
