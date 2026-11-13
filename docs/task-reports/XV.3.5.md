# 작업 보고서: XV.3.5 cache index privacy mode

- WBS: XV.3.5
- 작업명: cache index에 repository URL 원문 대신 keyed identifier와 commit/content hash만 저장하는 privacy mode, 원본 URL은 local secure config에만
- 상태: 완료 (`[x]`)
- 선행: XV.1.4, XV.3.1(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/cache_privacy.rs`(신규) — `PrivateCacheIndex`, `PrivateIndexEntry`,
  `LocalSourceConfig`, `CacheIndex::split_private`, `export_diagnostic`, 소유자 전용 원자 쓰기.
- `crates/dependencies/src/cache_index.rs` — mode 문서(1 = private), 인코딩 helper `pub(crate)`.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.3.5.md`(신규), 계획서(XV.3.5 `[ ]`→`[x]`).

## 구현 요약

- **privacy index(mode 1)**: entry = `RepositoryKey`(XV.3.2 keyed identifier) + commit + content hash +
  크기 + 최근 사용 시각. URL 필드가 형식에 없습니다. plain decoder는 mode 1을, privacy decoder는
  mode 0을 거부합니다. URL로 조회할 때는 keyer로 key를 계산합니다(URL 저장 없음).
- **local source config**: `sources.stlsecure`(magic `STLDSRCS`)에 key → URL만 저장하고 index와 다른
  파일로 둡니다. unix에서 `0600`으로 만든 임시 파일 → fsync → rename으로 씁니다. 로컬 UI 표시는
  `source_for(key)`로만 원문을 얻습니다.
- **진단 export**: `export_diagnostic`(및 `Display`)은 privacy index만 쓰므로 key·commit·hash·크기·
  시각만 출력합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 43 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| exported diagnostic·privacy index `Debug`/`Display`/bytes에 URL·host·경로 조각 없음, mode byte 1, round-trip, mode 교차 거부, keyer 조회·source config 원문 복원, 다른 key로 조회 불가(`exported_diagnostic_and_private_index_contain_no_url`) | 통과 |
| index 파일과 source config 파일 분리, index 파일 bytes에 URL 없음, config round-trip·canonical, config 권한 `0600`, 잘린 config 거부(`files_are_separate_and_source_config_is_owner_only`) | 통과 |

## 차이(작업 지시와 다른 점)

- "local secure config"는 소유자 전용 권한 파일로 구현했습니다. 아키텍처 37.3("Stellar 자체 비밀
  저장소 없음")에 따라 OS keychain 같은 별도 비밀 저장소는 쓰지 않습니다.
- 운영 keyer(로컬 비밀 key 기반 keyed hash)는 storage SHA-256을 쓰는 project 연결 작업이며, 테스트는
  비암호학적 FNV keyer를 씁니다.

## 남은 위험 / 한계

- keyer key 자체의 보관 위치(source config와 같은 소유자 전용 영역)는 project 연결 시 정합니다.
- plain mode는 계속 지원합니다. 어느 mode를 기본으로 쓸지는 설정 정책 결정이 필요합니다.

## 상태 정정(통합 리뷰, 2026-11-14)

- `[x]` → `[~]`: 엄격 선행 사슬에 `[~]` 행(XV.1.1 등)이 있어 함께 `[~]`로 둡니다. 이 행 자체의 검증은 dependencies crate 안에서 통과했습니다.

## 상태 복원(deps_wiring lane, 2026-11-14)

- `[~]` → `[x]`: 엄격 선행 XV.1.4·XV.3.1 `[x]`.
- 실제 파서 경로 검증: privacy index·local source config 검증은 파서와 무관하며 기존 테스트가 계속 통과합니다(`exported_diagnostic_and_private_index_contain_no_url`, `files_are_separate_and_source_config_is_owner_only`). owner-only 쓰기에 부모 디렉터리 fsync를 추가했습니다(리뷰 P7).
- 공통 근거: `Fix: stellar.yaml 의존성 선언을 실제 매니페스트 파서에 연결`(리뷰 P2), `Fix: stellar.lock.yaml entry 형식을 IV.1.6 모델 하나로 통합`(리뷰 P3, ADR-0021), `Fix: resolver library 매니페스트 읽기를 실제 stellar.yaml 파서로 연결`.
- 실행(`DEV-MAC`): `cargo test -p stellar-project` lib 261 passed·통합 17 passed, `cargo test -p stellar-dependencies` 57 passed·doctest 1 passed, `cargo clippy -p stellar-project -p stellar-dependencies --all-targets -- -D warnings` 경고 0, `cargo fmt -p stellar-project -p stellar-dependencies -- --check` 통과.
