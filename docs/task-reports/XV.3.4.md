# 작업 보고서: XV.3.4 library cache entry 검증

- WBS: XV.3.4
- 작업명: cache entry를 ProjectProbe와 storage validator로 검증하고 일반 Git repo 또는 손상 library 거부
- 상태: `[~]` 유지 — 남은 범위·결함 있음(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — 검증 순서·거부 정책·검증 결과 타입은 완료했지만, 실제 `ProjectProbe`(IV.5.1)·
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
