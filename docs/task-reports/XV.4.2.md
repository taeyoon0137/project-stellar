# 작업 보고서: XV.4.2 lock entry 필드와 누락 field validator

- WBS: XV.4.2
- 작업명: 각 lock entry에 exact commit, cached content hash, library schema version, transitive dependencies 기록
- 상태: 완료 (`[x]`) — 2026-11-23 library-finish, 아래 "작업 (2026-11-23, library-finish)" 절. 이전 기록: `[~]` 유지 — 구현 판정 통과, 선행 미충족(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 완료 (`[x]`)
- 선행: XV.3.4(이 lane `[~]` — 필요한 검증 결과 타입은 존재), XV.4.1(이 lane `[~]` — 변환 코드 존재)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/lock.rs` — `LockEntry`에 `content_hash`·`schema_version`·`dependencies`
  추가, `LOCK_ENTRY_KEYS`, `LockEntry::from_node`, `LockDocument::from_node`(validator),
  `LockBuildError::MissingContentHash`, `from_graph(graph, content_hashes)`.
- `crates/dependencies/src/node.rs` — `ValueError::{InvalidInteger, UnsupportedVersion, Inconsistent}`,
  `expect_sequence`, `parse_u32_strict`.
- `docs/task-reports/XV.4.2.md`(신규), 계획서(XV.4.2 `[ ]`→`[x]`).

## 구현 요약

- **기록**: entry마다 `exact_commit`, `content_hash`(검증된 cache entry의 `sha256:` hash —
  `from_graph`가 node별 map으로 받음, 없으면 `MissingContentHash`로 lock 생성 거부), `schema_version`
  (resolver가 commit 매니페스트에서 읽은 값), `dependencies`(전이 의존 node `{library_id,
  exact_commit}` 정렬 시퀀스)를 기록합니다.
- **validator**: `LockDocument::from_node`는 10개 entry 필드를 모두 필수로 요구하고, 알 수 없는 키,
  잘못된 값(UUID·commit·content hash·엄격 정수·ref 이름), `logical_id`와 경로 마지막 이름 모순, 빈
  경로, 비정렬 전이 의존성, 지원하지 않는 lock schema version, 비 canonical entry 순서를 경로와 함께
  거부합니다(lock은 resolver 출력만 허용, IV.1.7).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 46 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **누락 field validator 실패**: entry 필드 10개를 하나씩 제거한 문서가 각각 `MissingField("resolved[0].<field>")`로 실패(`missing_field_validator_fails`) | 통과 |
| 잘못된 content hash·`01` 정수·logical_id 모순·빈 경로·축약 commit·알 수 없는 키·lock version 2·entry 순서 뒤바뀜 거부 | 통과 |
| entry가 commit·content hash·schema version·전이 의존성을 가지며 round-trip 동일, content hash 없는 node는 lock 생성 거부(`every_entry_records_commit_hash_schema_and_transitive_dependencies`) | 통과 |
| 확장된 형식의 lock golden·선언 순서 무관(`canonical_lock_golden`) | 통과 |

## 차이(작업 지시와 다른 점)

- XV.4.1의 `canonical_lock_golden`은 같은 lane의 형식 확장에 맞춰 새 필드(content_hash,
  schema_version, dependencies)를 포함하도록 golden을 갱신했습니다(형식 변경이 이 작업의 내용).
- cached content hash 계산(materialize된 entry의 SHA-256)은 XV.3.3(차단)·project 연결 범위라 이
  작업은 검증된 hash를 입력으로 받습니다.

## 남은 위험 / 한계

- project `Lockfile`(IV.1.6) 모델은 이 필드 집합(library_id·repository·requested·dependencies)을 아직
  모릅니다. 텍스트 lock 연결 시 project 모델을 이 validator와 맞춰야 합니다.

## 상태 정정(통합 리뷰, 2026-11-14)

- `[x]` → `[~]`: 엄격 선행(XV.3.4·XV.4.1)이 `[~]`입니다.

## 상태 유지(deps_wiring lane, 2026-11-14)

- `[~]` 유지: 엄격 선행 XV.3.4가 아직 `[~]`입니다(실제 ProjectProbe·storage validator 연결은 project crate의 probe/adapter 파일이 필요하며 이 lane의 허용 경로 밖). lock entry 필드·validator는 이제 실제 `stellar.lock.yaml` 파서(`Lockfile::from_yaml`)가 그대로 씁니다(ADR-0021, `lock::tests::rejects_missing_field_in_resolved`).

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 구현 판정 통과, 선행 미충족.
- 근거: lock entry 필드(exact commit, cached content hash, schema version, transitive dependencies)와 누락 field 거부는 실제 `stellar.lock.yaml` 파서가 씁니다. 선행 XV.3.4가 `[~]`라 `[~]`를 유지합니다. 보고서 상태 줄이 계획서와 달리 `[x]`였던 것을 바로잡습니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/project/src/lock.rs` `Lockfile::from_yaml`(실제 `stellar.lock.yaml` 파서)이 dependencies의 entry 필드·validator를 쓰고, `rejects_missing_field_in_resolved`가 누락 field를 거부합니다(ADR-0021).

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-dependencies` | 96 passed |

### 남은 조건과 finding

- XV.3.4가 `[x]`가 되면 코드 변경 없이 `[x]`입니다.

## 작업 (2026-11-23, library-finish): 선행 XV.3.4 충족과 실제 lock 확인

### 한 일

- 선행 XV.3.4가 이 작업에서 제품 검증기로 완료되었습니다(XV.3.4 보고서). lock entry 필드·validator 코드는 바꾸지 않았습니다.
- 실제 Git library(전이 의존성 A → C)를 제품 resolver·materialize로 해석하고 제품 lock 기록(`write_resolved_lockfile`)으로 쓴 lock을
  제품 lock parser(`library_open::read_lock`)로 다시 읽어 확인했습니다.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 101 passed, 0 failed; doctest 1 passed |
| `cargo test -p stellar-project --lib library_` | library_entry·library_git 단위 시험 통과, 0 failed |
| `cargo clippy -p stellar-dependencies -p stellar-project -p stellar-assets -p stellar-integration-tests --all-targets -- -D warnings` | 경고 0 |
| `cargo xtask validate-adr --base 0c226a06` | 종료 코드 0 |
| `cargo test -p stellar-integration-tests --test library_lock_integrity` | 1 passed |

| 검사 | 결과 |
|---|---|
| entry가 exact commit·content hash(entry bytes를 `StorageSha256`으로 다시 계산한 값과 같음)·schema version 4·전이 의존성(C node)을 기록하고 제품 parser로 round-trip(`lock_records_entry_hash_and_tampered_cache_is_corruption_without_other_commit`) | 통과 |
| **누락 field validator 실패**: `content_hash` 줄을 지운 lock은 `read_lock`이 `LockInvalid`로 거부 | 통과 |
