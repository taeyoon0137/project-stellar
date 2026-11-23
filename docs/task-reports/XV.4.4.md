# 작업 보고서: XV.4.4 lock content hash와 cache entry 불일치 = 손상

- WBS: XV.4.4
- 작업명: lockfile content hash와 cache entry가 다르면 corruption으로 처리하고 자동으로 다른 commit을 선택하지 않음
- 상태: `[~]` 유지 — 구현 판정 통과, 선행 미충족(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 완료 (`[x]`)
- 선행: XV.4.2(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 파일 fixture).

## 변경 파일 목록

- `crates/dependencies/src/lock_integrity.rs`(신규) — `CachedContentSource` trait,
  `CacheReadFailure`, `LockIntegrityError`, `verify_lock_against_cache`, 디렉터리 기반 테스트 cache.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.4.4.md`(신규), 계획서(XV.4.4 `[ ]`→`[x]`).

## 구현 요약

- lock의 각 node(중복 제거)에 대해 cache entry **내용에서 content hash를 다시 계산**해 lock 값과
  비교합니다. 다르면 `Corrupted { node, path }`, entry가 없으면 `MissingEntry`, 읽기 실패는
  `ReadFailed`로 즉시 멈춥니다.
- 검사 함수는 선택자 해석기를 받지 않으므로 다른 commit을 고를 수단이 없습니다(구조적 보장). lock에
  없는 commit의 cache entry는 읽지도 않습니다. 복구(재fetch·재materialize·사용자 update)는 호출자
  결정입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 49 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 변조 없는 디렉터리 cache는 lock과 일치(`intact_cache_matches_lock`) | 통과 |
| **tampered cache fixture**: c v1 entry 파일 변조 → `Corrupted{c v1, a > c}`; 같은 library의 유효한 c v2 entry가 있어도 읽기·선택 없음(읽은 commit = lock의 a v1, c v1뿐); 파일 주입도 변조로 판정(`tampered_cache_fails_as_corruption_without_choosing_another_commit`) | 통과 |
| entry 삭제 → `MissingEntry`(`missing_entry_is_reported_separately`) | 통과 |

## 차이(작업 지시와 다른 점)

- 테스트의 content hash는 결정적 FNV 기반 32바이트(형식은 `sha256:`)입니다. 운영 hash(storage
  SHA-256)와 materialize된 entry 형식(XV.3.3, 선행 XIV.2.6 미완료로 차단)은 project 연결 시
  `CachedContentSource`로 들어옵니다. 정책(비교·손상 판정·대체 commit 금지)은 hash 알고리즘과 무관합니다.

## 남은 위험 / 한계

- entry 전체를 다시 hash하므로 큰 library에서는 비용이 큽니다. 열기 경로에서 언제 전체 검증할지(예:
  최초 사용·주기적)는 성능 정책이 필요합니다.

## 상태 정정(통합 리뷰, 2026-11-14)

- `[x]` → `[~]`: 엄격 선행 XV.4.2가 `[~]`입니다.

## 상태 유지(deps_wiring lane, 2026-11-14)

- `[~]` 유지: 엄격 선행 XV.4.2가 `[~]`입니다(위 XV.4.2 사유).

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 구현 판정 통과, 선행 미충족.
- 근거: lockfile content hash와 cache entry가 다르면 corruption으로 처리하고 다른 commit을 자동 선택하지 않는 판정이 있습니다. 선행 XV.4.2가 `[~]`라 `[~]`를 유지합니다. 보고서 상태 줄이 `[x]`였던 것을 바로잡습니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/dependencies/src/lock_integrity.rs` `verify_lock_against_cache`(85행): cache entry 내용에서 content hash를 다시 계산해 lock 값과 비교하고, 다르면 `Corrupted`, 없으면 `MissingEntry`를 반환합니다. 다른 commit을 고르는 경로가 없습니다.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-dependencies` | 96 passed |

### 남은 조건과 finding

- XV.3.4 → XV.4.2 사슬이 풀리면 다시 판정합니다.
