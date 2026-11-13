# 작업 보고서: XV.3.1 사용자별 library cache root와 cache index

- WBS: XV.3.1
- 작업명: 사용자별 OS app data에 library cache root와 cache index 정의, 프로젝트 폴더 안에 의존성 checkout 금지
- 상태: 완료 (`[x]`)
- 선행: V.1.5(`[x]`), XV.2.1(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 Git, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/content_hash.rs`(신규) — `ContentHashText`(`sha256:<64 hex>`, ADR-0004 형식).
- `crates/dependencies/src/cache_root.rs`(신규) — `LibraryCacheRoot`(`in_app_data`, `index_path`,
  `mirrors_dir`, `entries_dir`, `ensure_outside_project`, `create_dirs`), `CacheRootError`.
- `crates/dependencies/src/cache_index.rs`(신규) — `CacheIndex`, `CacheIndexEntry`, canonical LE
  인코딩/엄격 디코딩, 원자적 파일 쓰기, `CacheIndexError`.
- `crates/dependencies/src/node.rs` — `ValueError::InvalidContentHash`.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.3.1.md`(신규), 계획서(XV.3.1 `[ ]`→`[x]`).

## 구현 요약

- **cache root**: 플랫폼 셸이 주입하는 앱 데이터 경로(정규화된 절대 경로만) 아래 `library-cache/`
  (`index.stlcache`, `mirrors/`, `entries/`). `ensure_outside_project`는 symlink를 해석한 뒤 cache가
  프로젝트 안에 있거나 프로젝트가 cache 안에 있으면 거부합니다. 오류에는 경로 원문을 넣지 않습니다.
- **cache index**: entry = (repository, exact commit, content hash, 크기, 최근 사용 시각). 형식은
  magic `STLDCIDX` + version u16 + mode u8(0 = plain) + count u32 + (repository 비교 키, commit)
  오름차순 entry의 LE binary입니다. 디코딩은 magic·버전·mode·길이·필드 규칙·canonical 순서·중복·남는
  bytes를 모두 검사합니다. 쓰기는 같은 디렉터리 임시 파일 → fsync → rename입니다.
- `touch`는 최근 사용 시각을 단조 증가로만 갱신합니다(XV.8.1 GC 입력).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 36 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 실제 Git 프로젝트 fixture + 별도 app data에 cache dir·index 기록 → 프로젝트 파일 목록 불변, `git status --porcelain=v1 --untracked-files=all --ignored` 기록 전후 동일하고 빈 출력, 임시 파일 잔존 없음(`cache_lives_in_app_data_and_project_git_status_is_unchanged`) | 통과 |
| cache가 프로젝트 안(`InsideProject`)·프로젝트가 cache 안(`ProjectInsideCache`)·상대/`..` 경로 거부(`cache_root_inside_project_is_rejected`) | 통과 |
| index round-trip, 삽입 순서 무관 동일 bytes, header golden(`encode_decode_round_trip_is_canonical`) | 통과 |
| 잘못된 magic·버전·mode, 잘림, 남는 bytes, 비 canonical 순서, 잘못된 content hash 거부(`decode_rejects_malformed_bytes`) | 통과 |
| 비교 키 조회·최근 사용 시각 단조 갱신(`touch_and_lookup`), content hash 형식(`format_is_sha256_lowercase_hex`) | 통과 |

## 차이(작업 지시와 다른 점)

- OS 앱 데이터 경로 결정(macOS `~/Library/Application Support/<app>`)은 플랫폼 셸이 주입합니다(복구
  저장소 V.9.1과 같은 방식). 이 crate는 주입된 경로 아래 구조만 정의합니다.
- 원자적 파일 쓰기는 storage crate `atomic_file`을 쓸 수 없어(의존 방향) 같은 절차를 이 crate에
  작게 구현했습니다.

## 남은 위험 / 한계

- plain mode index는 repository URL 원문을 앱 데이터에 저장합니다. URL을 남기지 않는 privacy
  mode는 XV.3.5입니다.
- 여러 프로세스의 동시 index 갱신 잠금은 XV.3.6 범위입니다(현재는 마지막 rename이 이깁니다).

## 상태 정정(통합 리뷰, 2026-11-14)

- `[x]` → `[~]`: 엄격 선행 사슬에 `[~]` 행(XV.1.1 등)이 있어 함께 `[~]`로 둡니다. 이 행 자체의 검증은 dependencies crate 안에서 통과했습니다.

## 상태 복원(deps_wiring lane, 2026-11-14)

- `[~]` → `[x]`: 엄격 선행 V.1.5·XV.2.1 `[x]`.
- 실제 파서 경로 검증: cache root·index 검증은 파서와 무관하며 기존 `cache_lives_in_app_data_and_project_git_status_is_unchanged`(실제 Git status 불변)가 계속 통과합니다. 원자 쓰기에 부모 디렉터리 fsync를 추가했습니다(리뷰 P7, `atomic_write_syncs_parent_directory`).
- 공통 근거: `Fix: stellar.yaml 의존성 선언을 실제 매니페스트 파서에 연결`(리뷰 P2), `Fix: stellar.lock.yaml entry 형식을 IV.1.6 모델 하나로 통합`(리뷰 P3, ADR-0016), `Fix: resolver library 매니페스트 읽기를 실제 stellar.yaml 파서로 연결`.
- 실행(`DEV-MAC`): `cargo test -p stellar-project` lib 261 passed·통합 17 passed, `cargo test -p stellar-dependencies` 57 passed·doctest 1 passed, `cargo clippy -p stellar-project -p stellar-dependencies --all-targets -- -D warnings` 경고 0, `cargo fmt -p stellar-project -p stellar-dependencies -- --check` 통과.
