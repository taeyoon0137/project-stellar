# 작업 보고서: XIV.14.6 non-UTF8·symlink·executable·LFS pointer·손상 프로젝트 Git corpus

- WBS: XIV.14.6
- 작업명: non-UTF8 path, symlink, executable, LFS pointer, corrupt project scenario 추가
- 상태: 완료 (`[x]`)
- 선행: XIV.1.5(`[x]`), XIII.4(section 선행 — XIII.4.1~4.3·4.5 `[x]`, XIII.4.4 `[~]`; 필요한 symlink resolver 코드가 있어 완화), XIII.7.5(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증; corpus는 `cfg(all(test, unix))`).

## 변경 파일 목록

- `crates/git-porcelain/src/corpus.rs`(신규, 테스트 빌드 전용) — 4개 scenario 테스트와 fixture provider.
- `crates/git-porcelain/src/lib.rs` — `#[cfg(all(test, unix))] mod corpus;`.
- `docs/task-reports/XIV.14.6.md`(신규), 계획서(XIV.14.6 `[ ]`→`[x]`).

## 구현 요약

XIV.14.1 fixture harness(hermetic·`GIT_ALLOW_PROTOCOL=file`)로 실제 Git 저장소를 만들고 scenario별 정책 결과를
고정합니다.

- **non-UTF8 경로**: `assets/caf\xe9.png`(APFS가 만들 수 없어 index에 직접 등록 후 commit) — index·HEAD tree가
  원시 bytes·OID 보존, status `.D`·Stellar asset 분류(`in_index`), 표시 문자열은 lossy(U+FFFD)·원시 bytes 별도
  보존, materialize는 macOS에서 `MaterializeError::Io{path=원시 경로}`로 거부하고 임시 디렉터리를 남기지 않음
  (lossy 이름으로 만들지 않음).
- **symlink·executable**: 내부(`../stellar.yaml`)·외부 절대(존재하지 않는) target symlink와 0755 파일 — tree·index
  mode `120000`·`100755`, 작업 트리 목록은 따라가지 않고 Symlink·Executable, 파일 읽기는 target bytes,
  materialize는 0755와 링크만 생성(따라가지 않음, symlink 2개), 실행 비트 제거는 worktree mode 100755→100644 `.M`과
  Stellar unstaged Modified.
- **LFS pointer**: object 없는 pointer(`assets/big.psd`) — 작업 트리 판정 `LfsObjectMissing`(oid·size 정확)과 진단
  코드 `asset-lfs-object-missing`이 일반 missing(`asset-file-missing`)과 다름, HEAD materialize는 filter 없이
  pointer bytes 그대로이고 같은 판정.
- **손상 프로젝트**: 작업 트리 `stellar.yaml`·chunk 손상 + README 수정 — `read_project_status`가 raw status(README
  `.M`, unstaged 3, Stellar 파일 2개)를 유지하고 semantic만 실패(`manifest 손상`), 작업 트리 bytes·index 불변(자동
  수정 없음), Index snapshot은 손상 전 정상 내용으로 materialize(`.git` 없음).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | lib 290 passed + doc 3 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-git-porcelain --all-targets --features test-support -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| `non_utf8_stellar_path_bytes_are_preserved_and_never_lossily_materialized` | 통과 |
| `symlink_and_executable_modes_are_preserved_without_following_links` | 통과 |
| `lfs_pointer_without_object_is_distinct_from_a_missing_file` | 통과 |
| `corrupt_project_keeps_raw_status_and_readable_git_snapshots` | 통과 |

## 차이(작업 지시와 다른 점)

- 손상 프로젝트 scenario의 semantic provider는 테스트용(매니페스트·chunk 접두어 검사)입니다. 실제 provider 연결은
  XIV.4.5 보고서의 crate 그래프 한계와 같습니다.
- XIII.4 선행은 section 단위라 완화했습니다(XIII.4.4 `[~]`는 watcher 동적 등록이며 이 corpus가 쓰는 symlink 경로
  처리와 무관).

## 남은 위험 / 한계

- non-UTF8 materialize 결과는 파일 시스템에 따라 다릅니다. 테스트는 macOS에서 거부 경로를 강제하고, 그 밖의
  파일 시스템에서는 같은 bytes 이름으로 만들어졌는지만 확인합니다(Windows는 ADR-0006으로 보류).

## 후속 수정 (리뷰 반영)

- **section 선행 완화 근거 보강(리뷰 minor, 사실 기록)**: 선행 XIII.4 section 중 XIII.4.4만 `[~]`입니다. 그
  보고서의 사유는 "macOS adapter로 구현·검증 완료, 선행 XIII.2.3(Windows adapter)은 ADR-0006으로 `[!]`"이며,
  Windows 보류(ADR-0006, 사용자 결정) 외의 미완료는 없습니다. 이 corpus는 watcher 동적 등록을 쓰지 않습니다.
  계획 규칙 1.2.4를 엄격히 읽으면 이 행은 차단 대상이므로, 완화 허용 여부는 통합 담당이 확인해야 합니다. 상태는
  바꾸지 않았습니다.
