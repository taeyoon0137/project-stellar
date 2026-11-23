# 작업 보고서: XV.6.2 local override 대상 검증

- WBS: XV.6.2
- 작업명: local path가 일반 repo 또는 잘못된 library id이면 거부
- 상태: 완료 (`[x]`) — 2026-11-23 library-finish, 아래 "작업 (2026-11-23, library-finish)" 절. 이전 기록: `[~]` 유지 — 남은 범위·결함 있음(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — 위치 재검사(symlink 해석)·거부 정책은 실제 디렉터리 fixture로 완료했지만,
  "일반 repo" 판정 자체는 project crate의 `ProjectProbe`(IV.5.1) 연결이 필요합니다(현재 테스트는 표
  기반 probe fake).
- 선행: XV.6.1(이 lane에서 완료), IV.5.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 파일 시스템·symlink·시스템 Git, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/local_override.rs` — `LocalOverrideRejection`, `ValidatedLocalOverride`,
  `validate_local_override`, 실제 디렉터리 fixture 테스트.
- `docs/task-reports/XV.6.2.md`(신규), 계획서(XV.6.2 `[ ]`→`[~]`).

## 구현 요약

- 대상과 사용 프로젝트 root를 canonicalize(symlink 해석)해 대상이 프로젝트 자신·내부면
  `InsideProject`로 거부합니다(XV.6.1 lexical 검사를 우회하는 symlink 차단). 없는 경로는
  `Unavailable`입니다.
- XV.3.4의 `LibraryProjectValidator::probe`(ProjectProbe)로 일반 Git 저장소(`NotAStellarProject`)와
  손상 매니페스트를 거부하고, probe `project.id`가 선언 library id와 다르면 `LibraryIdMismatch`입니다.
- 통과 결과는 `ValidatedLocalOverride`(private field)로만 표현됩니다. 오류에는 경로 원문이 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 53 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 실제 `git init` 일반 저장소 → `NotAStellarProject`, 다른 id library → `LibraryIdMismatch`, 손상 library → `CorruptManifest`, 없는 경로 → `Unavailable(NotFound)`(`plain_repo_or_wrong_library_id_is_rejected`) | 통과(probe 결과는 표 fake) |
| lexical 검사는 통과하지만 프로젝트 안을 가리키는 symlink → `InsideProject` | 통과(실제 symlink) |
| 정상 library → 검증 결과(id·schema version·canonical root) | 통과 |

## 차이(작업 지시와 다른 점)

- 의존 방향상 `ProjectProbe`를 직접 호출할 수 없어 XV.3.4 trait으로 받았습니다. 실제 probe가 일반
  저장소 fixture를 거부하는지는 project crate 연결 후 확인해야 하므로 `[~]`입니다.

## 남은 위험 / 한계

- linked worktree 대상(gitfile) 허용은 XV.6.4 범위입니다(ProjectProbe의 gitfile 처리에 의존).
- override 대상의 current commit·dirty 상태 진단(XV.6.3)은 선행 XIV.2.2 미완료로 차단입니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 남은 범위·결함 있음.
- 근거: local override 위치 재검사(symlink 해석)·거부 정책은 실제 디렉터리 fixture로 있지만 "일반 repo" 판정은 표 기반 probe fake만 써서 `[~]`를 유지합니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/dependencies/src/local_override.rs`. project의 `manifest.rs`는 `local_overrides`만 가져오고 거부 판정에 실제 `ProjectProbe`를 넘기지 않습니다.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-dependencies` | 96 passed |

### 남은 조건과 finding

- 결함 finding(소유 lane 없음 → backlog): XV.3.4와 같은 probe 연결로 실제 일반 repo·잘못된 library id fixture를 확인합니다.

## 작업 (2026-11-23, library-finish): 실제 ProjectProbe로 local override 검증

### 한 일

- `crates/project/src/library_overrides.rs`(신규): `validate_project_local_overrides`가 선언 매핑과 대상 검증을 제품 검증기
  `ProjectLibraryValidator::checkout`(실제 `probe_project`)으로 실행하고 override마다 결과를 돌려줍니다(ADR-0119 결정 7).

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 101 passed, 0 failed; doctest 1 passed |
| `cargo test -p stellar-project --lib library_` | library_entry·library_git 단위 시험 통과, 0 failed |
| `cargo clippy -p stellar-dependencies -p stellar-project -p stellar-assets -p stellar-integration-tests --all-targets -- -D warnings` | 경고 0 |
| `cargo xtask validate-adr --base 0c226a06` | 종료 코드 0 |
| `cargo test -p stellar-integration-tests --test library_local_override` | 1 passed |

| 검사 | 결과 |
|---|---|
| **fixture 실패**: 실제 폴더로 일반 Git 저장소(`stellar.yaml` 없음)·`.git` 없는 Stellar 폴더는 `NotAStellarProject`, 다른 library id는 `LibraryIdMismatch`, 손상 매니페스트는 `CorruptManifest`, 없는 경로는 `Unavailable(NotFound)`, symlink로 사용 프로젝트 안을 가리키면 `InsideProject` | 통과 |
| 정상 Stellar 작업 폴더는 통과(library id·schema version 4), 거부 문장에 경로 원문 없음 | 통과 |
