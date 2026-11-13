# 작업 보고서: XV.3.3 content-addressed immutable library cache entry

- WBS: XV.3.3
- 작업명: exact commit의 Stellar project files를 content-addressed immutable cache entry로 materialize
- 상태: 완료 (`[x]`)
- 선행: XV.3.2(`[x]`, 2026-11-14 상태 복원), XIV.2.6(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 파일 시스템·시스템 Git, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/cache_entry.rs`(신규) — `Sha256Digest`, `MaterializeLimits`,
  `EntryFileMode`, `EntryFileRecord`, `entry_listing`, `entry_content_hash`, `EntryReuse`,
  `MaterializedEntry`, `MaterializeError`, `LibraryEntryStore::{new, with_limits, entry_dir, lookup,
  materialize}`, `recompute_entry_hash`, `CachedContentSource` 구현, 실제 Git fixture 테스트.
- `crates/dependencies/src/git.rs` — `GitCommandRunner::run_git_with_input`(stdin 입력, `cat-file
  --batch`용).
- `crates/dependencies/src/cache_privacy.rs` — `PrivateCacheIndex::{read_from, touch}`,
  `LocalSourceConfig::{upsert, read_from}`.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `crates/dependencies/src/test_support.rs` — `SystemGit` stdin 실행, 테스트 digest `FakeSha256`.
- `crates/project/src/manifest_source.rs` — 테스트 실행기 `TestGit`의 새 trait 메서드 구현(테스트 전용).
- `docs/adr/ADR-0017-library-cache-entry-content-hash.md`(신규).
- `docs/task-reports/XV.3.3.md`(신규), 계획서(XV.3.3 `[ ]`→`[x]`).

## 구현 요약

- **범위**: commit tree에서 `stellar.yaml`, `stellar.lock.yaml`, `design/**`, `assets/**`만
  materialize합니다(아키텍처 19장). `.git`·README·소스는 만들지 않습니다. `stellar.yaml`이 없으면
  `NotAStellarProject`입니다.
- **Git 경계**: fetch된 mirror(XV.3.2 `FetchedRefs`)에서 `cat-file -t`(commit 확인),
  `ls-tree -r -l -z --full-tree`, `cat-file --batch`(256개·64 MiB 단위) 출력만 해석합니다. XIV.2.6과
  같은 방식이고 Git 객체를 직접 파싱하지 않습니다. batch 해석은 크기 overflow 없이 경계를 검사합니다
  (git lane 리뷰 G1과 같은 결함을 처음부터 피함).
- **content address(ADR-0017)**: `stellar-library-entry 1` listing(mode, size, 파일 SHA-256, path —
  path byte 순)의 SHA-256이 entry 이름(`entries/sha256-<hex>`)이자 lock `content_hash`입니다.
- **immutable 공개**: staging 디렉터리에 쓰고 `rename` 한 번으로 공개한 뒤 `entries/`를 fsync합니다.
  파일은 0444/0555입니다. 실패하거나 같은 내용이 이미 있으면 staging을 지웁니다(drop guard).
- **재사용**: (repository, commit)이 privacy index(XV.3.5)에 있고 디렉터리가 있으면 Git을 실행하지
  않고 같은 entry를 돌려주며 최근 사용 시각을 갱신합니다(`CommitHit`). 새 commit이라도 내용이 같으면
  기존 entry를 씁니다(`ContentHit`). index에는 URL 원문 대신 keyed identifier만 기록하고, 원본 URL은
  `sources.stlsecure`(0600)에 둡니다.
- **입력 검증**: symlink·submodule, `.git`·`.`·`..`·빈 component, 제어 문자·`\`, non-UTF-8, 대소문자
  충돌, 파일 수·총 크기 상한 위반을 공개 전에 거부합니다. 오류에는 경로·URL이 없습니다.
- **XV.4.4 연결**: `recompute_entry_hash`는 디스크 entry를 반복 순회(symlink 미추종)해 같은 hash를
  다시 계산합니다. `LibraryEntryStore`가 `CachedContentSource`를 구현하므로
  `verify_lock_against_cache`를 실제 materialize된 entry에 쓸 수 있습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 63 passed, 0 failed; doctest 1 passed |
| `cargo test -p stellar-project` | lib 265 passed, 통합 17 passed, 0 failed |
| `cargo clippy -p stellar-dependencies -p stellar-project --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -p stellar-project -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **같은 commit 재요청 bytes 재사용**: 실제 Git library(binary chunk·실행 파일·asset·README·.gitignore) materialize → project files 5개만 원본 bytes 그대로, 재계산 hash·크기 일치, 0444/0555. 새 store·새 실행기로 같은 commit 재요청 시 `CommitHit`, Git 실행 0회(mirror 포함), 같은 디렉터리·같은 inode·같은 bytes, 최근 사용 시각 갱신, index bytes에 URL 없음. README만 바뀐 commit은 `ContentHit`(같은 inode), project file 변경 commit은 새 entry. staging 잔여물 없음(`same_commit_rerequest_reuses_entry_bytes`) | 통과 |
| 실제 materialize hash로 만든 lock은 `verify_lock_against_cache` 통과, entry 파일 변조 시 `Corrupted`, entry 안 symlink는 `ReadFailed(InvalidData)`(`lock_integrity_detects_tampered_materialized_entry`) | 통과 |
| 일반 Git 저장소 `NotAStellarProject`, `design/` symlink `Symlink`, gitlink `Submodule`, 개행 포함 경로 `BadCharacter`, `design/Chunk`·`design/chunk` `PathCollision`, 공개 entry·staging 없음(`unsupported_trees_are_rejected_before_publishing`) | 통과 |
| 파일 수·총 크기 상한, batch 상한 1 byte에서 파일별 batch(cat-file 6회)로 같은 결과, 없는 commit `CommitNotFound`(`limits_and_unknown_commit_are_rejected`) | 통과 |
| `cat-file` missing·id/종류/크기 불일치, `u64::MAX` 크기·잘림·끝 LF 없음·남는 bytes를 panic 없이 거부, `ls-tree` `.GIT`·빈 component·`\`·잘못된 크기·TAB 없음·파일/디렉터리 대소문자 충돌·non-UTF-8 거부(`batch_and_tree_parsers_reject_malformed_output`) | 통과 |
| listing 형식 golden, 입력 순서 무관(`listing_is_canonical_and_order_independent`) | 통과 |
| 변이 확인: `CommitHit` 조기 반환을 끄면 재사용 테스트가 실패함을 확인한 뒤 원복 | 확인 |

## 차이(작업 지시와 다른 점)

- SHA-256 구현은 storage crate에 있어 `Sha256Digest` trait으로 받습니다. 테스트는 결정적
  비암호 digest(`FakeSha256`)를 씁니다. 운영 연결(storage `sha256`)은 project/runtime 통합 범위입니다.
- 테스트는 구현과 같은 변경에서 작성했습니다. 실패 확인은 위 변이 확인으로 대신했습니다.
- `crates/project/src/manifest_source.rs`는 trait 메서드 추가에 따른 테스트 실행기 수정만 있습니다.

## 남은 위험 / 한계

- Unicode NFC 정규화 충돌(아키텍처 19.2)과 Windows 예약 이름은 검사하지 않습니다(Windows 보류,
  ADR-0006). 대소문자 충돌은 Unicode 소문자 변환으로만 판정합니다.
- 같은 entry를 여러 프로세스가 동시에 읽고 쓰는 경우의 lock 분리는 XV.3.6 범위입니다. 지금은 같은
  내용을 동시에 공개하면 나중 rename이 실패하고 `ContentHit`으로 처리됩니다. index 갱신은 read-modify-
  write이므로 동시 갱신에서 한쪽 기록이 사라질 수 있습니다(XV.3.6에서 write lock으로 직렬화).
- materialize된 entry를 ProjectProbe·storage validator로 검증하는 연결(XV.3.4)은 아직 `[~]`입니다.
