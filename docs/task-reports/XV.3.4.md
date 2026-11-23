# 작업 보고서: XV.3.4 library cache entry 검증

- WBS: XV.3.4
- 작업명: cache entry를 ProjectProbe와 storage validator로 검증하고 일반 Git repo 또는 손상 library 거부
- 상태: 완료 (`[x]`) — 2026-11-23 library-finish, 아래 "작업 (2026-11-23, library-finish)" 절. 이전 기록: `[~]` 유지 — 남은 범위·결함 있음(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — 검증 순서·거부 정책·검증 결과 타입은 완료했지만, 실제 `ProjectProbe`(IV.5.1)·
  generation loader(V.4.4) 연결과 실제 디렉터리 fixture 검증은 project crate 연결 작업이 필요합니다.
- 선행: IV.5.1(`[x]`), V.4.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/entry_validation.rs`(신규) — `LibraryProjectValidator` trait,
  `ProbedLibrary`, `LibraryProbeFailure`, `StorageValidationFailure`, `LibraryEntryRejection`,
  `ValidatedLibraryEntry`, `validate_library_entry`, 테스트용 `TableValidator`.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.3.4.md`(신규), 계획서(XV.3.4 `[ ]`→`[~]`).

## 구현 요약

- 검증 순서: ProjectProbe → probe `project.id`와 선언 `LibraryId` 비교 → storage validator(current
  pointer → manifest → chunk). 비싼 storage 검증은 앞 단계 통과 후에만 실행합니다.
- 거부 사유: 일반 Git 저장소(`NotAStellarProject`), 매니페스트 손상, library id 불일치, 저장 데이터
  손상(단계 포함), I/O.
- `ValidatedLibraryEntry`는 private field라 `validate_library_entry` 밖에서 만들 수 없어 검증되지 않은
  entry가 lock·로드 경로로 들어가지 못합니다. 검증된 schema version을 lock entry(XV.4.2)가 씁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 41 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| invalid library fixture(일반 Git repo, 손상 YAML, 다른 library id, chunk 누락) 각각 정확한 사유로 거부, probe·id 거부 시 storage 검증 미실행(`invalid_library_fixtures_are_rejected`) | 통과(검증기는 in-crate 표 fake) |
| 정상 entry → root·library id·schema version 보존(`valid_entry_carries_schema_version`) | 통과 |

## 차이(작업 지시와 다른 점)

- 의존 방향상 project(`ProjectProbe`)·storage(generation loader)를 직접 호출할 수 없어 trait으로
  받았습니다. 실제 fixture(일반 Git 저장소 디렉터리, 손상된 library 디렉터리)로 거부를 확인하는 것은
  project crate가 trait을 구현한 뒤 가능합니다 — 그래서 `[~]`입니다.

## 남은 위험 / 한계

- `ProjectProbe`는 root에 `.git`을 요구하지만 materialize된 cache entry(XIV.2.6 방식)는 `.git`이
  없습니다. project 연결 시 cache entry용 probe 모드(root `.git` 요구 생략)가 필요합니다.
- cache entry materialize(XV.3.3)는 선행 XIV.2.6 미완료로 차단 상태입니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 남은 범위·결함 있음.
- 근거: 검증 순서·거부 정책·결과 타입은 있지만 실제 `ProjectProbe`(IV.5.1)·generation loader(V.4.4) 연결과 실제 디렉터리 fixture가 없어 `[~]`를 유지합니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/dependencies/src/entry_validation.rs`: 검증 결과 타입과 순서. probe·loader는 fake로만 호출됩니다.
- project·runtime·ops에서 `entry_validation`을 쓰는 곳이 없습니다.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-dependencies` | 96 passed, 0 failed |

### 남은 조건과 finding

- 결함 finding(dependencies·project 연결, 소유 lane 없음 → backlog): cache entry용 probe 모드(root `.git` 요구 생략)를 project crate에 두고 실제 일반 Git repo·손상 library 디렉터리 fixture로 거부를 확인합니다. XV.4.2·XV.4.4가 이 행을 선행으로 둡니다.

## 작업 (2026-11-23, library-finish): 제품 ProjectProbe·storage validator 연결

### 한 일

- `crates/project/src/library_entry.rs`(신규): `ProjectLibraryValidator`가 `LibraryProjectValidator`의 제품 구현입니다.
  cache entry probe는 `stellar.yaml` 정규 파일(symlink 거부)을 loader 상한까지 읽어 제한 loader·`Manifest::from_yaml`로
  해석합니다(materialize한 entry에는 `.git`이 없어 IV.5.1의 `.git` 요구만 뺌). Git 작업 폴더 probe는 `probe_project` 그대로입니다.
  storage validator는 `design/current`가 있으면 current pointer → manifest → **모든 chunk를 읽어 hash 확인**하고 component
  library chunk를 해석하며, 텍스트 원본(ADR-0088)을 해석합니다. generation과 텍스트 원본이 둘 다 컴포넌트 정의를 담으면 손상입니다.
- `crates/dependencies/src/library_snapshot.rs`: `load_cached_snapshot`이 entry read lock 아래에서 bytes 재해시(XV.4.4) →
  `validate_library_entry`(XV.3.4)를 통과한 entry에서만 export를 읽습니다(`SnapshotLoadError::{Corrupted, Rejected}`). lock 기준
  열기와 update 후보가 이 경로 하나를 씁니다(`CacheUpdateSource`의 별도 검증 호출은 지움).
- `crates/project/src/library_exports.rs`(신규): 제품 `LibraryExportSource`(`SourceExports`). `crates/project/src/library_git.rs`(신규):
  시스템 Git 실행기(`SystemGitCommands`)와 저장 SHA-256(`StorageSha256`).
- `StorageValidationFailure::TextSource`, `ExportReadFailure::InvalidLibrary`를 더했습니다. 결정: ADR-0119 결정 1·2.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 101 passed, 0 failed; doctest 1 passed |
| `cargo test -p stellar-project --lib library_` | library_entry·library_git 단위 시험 통과, 0 failed |
| `cargo clippy -p stellar-dependencies -p stellar-project -p stellar-assets -p stellar-integration-tests --all-targets -- -D warnings` | 경고 0 |
| `cargo xtask validate-adr --base 0c226a06` | 종료 코드 0 |
| `cargo test -p stellar-integration-tests --test library_cache_entries` | 1 passed |

| 검사 | 결과 |
|---|---|
| **invalid library fixture 실패**(실제 Git 저장소): 일반 Git 저장소는 materialize와 두 probe 모두 거부, 다른 library id는 `LibraryIdMismatch`, 텍스트 원본 손상은 `CorruptStorage(TextSource)`, 제품 cache 읽기 경로도 손상 entry를 `Rejected`로 거부(`cache_entries_are_validated_by_probe_and_storage_before_exports_are_read`) | 통과 |
| 정상 entry: `.git` 없는 entry를 cache entry probe가 받고, 제품 export 공급자가 generation component library chunk에서 Card·property 타입 표기(`String`, `Optional<Number>`)·공개 여부와 token을 읽음 | 통과 |
| bytes 변조·검증 거부 entry는 검증기·export를 부르지 않음(`library_snapshot::tests::tampered_or_rejected_entry_is_not_read`, `library_load::tests::tampered_bytes_and_rejected_entries_are_diagnostics_without_other_commit`) | 통과 |
| cache entry probe 단위 시험(`library_entry::tests` 2건) | 통과 |

### 남은 위험

- 열 때마다 entry 전체를 다시 hash합니다(ADR-0119 Performance). 큰 library에서 문제가 되면 검증 기록 최적화를 따로 정합니다.

## 검토 (2026-11-24, library-finish 비판 검토)

- 판정: `[x]` 유지.
- 고친 결함(이중 판정): "generation component library와 텍스트 원본이 함께 정의를 담으면 손상" 판정이 storage validator
  (`validate_library_storage`)와 export 공급자(`read_library_resources`)에 따로 있었습니다. `library_entry::read_library_root` 하나로
  모으고 두 경로는 오류 분류만 옮깁니다(커밋 `1fe5cb21`).
- 더한 시험: `tests/integration/tests/library_cache_entries.rs`의 `generation_and_text_component_definitions_together_are_corrupt`가 텍스트
  원본만 있으면 통과, 같은 root에 generation component library를 더하면 storage validator `CorruptStorage(TextSource)`와 export 공급자
  `Corrupt`를 확인합니다(커밋 `550b4565`).
