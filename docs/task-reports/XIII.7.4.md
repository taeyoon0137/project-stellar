# 작업 보고서: XIII.7.4 기존 이력 LFS 변환 미지원

- WBS: XIII.7.4
- 작업명: 기존 history migration은 지원하지 않고 요청 시 UnsupportedOperation 반환
- 상태: 완료 (`[x]`)
- 선행: XIII.7.3 (이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증).

## 변경 파일 목록

- `crates/git-porcelain/src/lfs/migrate.rs`(신규) — `LfsMigrationMode`, `LfsMigrationRequest`,
  `UnsupportedOperation`(`unsupported-operation`), `migrate_lfs_history`.
- `crates/git-porcelain/src/lfs.rs` — `migrate` 모듈 등록.
- `docs/task-reports/XIII.7.4.md`(신규), 계획서(XIII.7.4 `[ ]`→`[x]`).

## 구현 요약

- `migrate_lfs_history(git, worktree_root, request)`는 import·export 어느 쪽이든 항상
  `Err(UnsupportedOperation { operation: "git lfs migrate import|export" })`를 반환합니다. 성공 타입이
  `Infallible`이라 성공 값을 만들 수 없고, Git 실행기를 받지만 쓰지 않습니다(프로세스 생성 0).
- 오류 코드는 고정 문자열 `unsupported-operation`이며 메시지는 "기존 Git 이력을 다시 쓰지 않는다"는
  정책을 설명합니다.
- LFS 추적 적용 경로(XIII.7.3)의 argv는 항상 `lfs track ...`으로 고정되어 migrate 명령을 만들 수
  없음을 테스트로 고정했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | lib 253 passed + doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **이력 변경 명령 실행 0**: trusted·untrusted 모두 import·export 요청이 `unsupported-operation`, executable 없는 실행기 spawn 0 | 통과 |
| 실제 Git: 이력이 있는 저장소에서 요청 후 이 호출의 spawn 0, `for-each-ref`·`HEAD` 불변 | 통과 |
| 승인 후 실행 경로(`LfsTrackRequest`) argv가 `lfs track`으로 고정, migrate 미포함 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- UI·agent tool·CLI에서 이 거부를 사용자 안내로 노출하는 연결은 상위 계층 작업입니다.
