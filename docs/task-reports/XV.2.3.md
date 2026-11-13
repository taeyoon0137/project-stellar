# 작업 보고서: XV.2.3 전이 순환 탐지

- WBS: XV.2.3
- 작업명: transitive cycle을 탐지하고 dependency path 전체를 오류로 반환
- 상태: 완료 (`[x]`)
- 선행: XV.2.2(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/resolver.rs` — `ResolveError::Cycle { path, node }`, 탐색 체인 검사, 테스트.
- `docs/task-reports/XV.2.3.md`(신규), 계획서(XV.2.3 `[ ]`→`[x]`).

## 구현 요약

- resolver의 명시적 stack frame은 각각 부모 node를 가지므로 stack의 부모 목록이 곧 현재 탐색
  체인입니다. 새 선언이 해석된 node key가 체인에 있으면 `Cycle { path, node }`로 실패합니다. `path`는
  root에서 순환을 닫는 선언까지 전체 logical name 경로(`a > b > a`)입니다.
- 체인 밖에서 이미 해석된 node(diamond, root의 중복 직접 선언)는 기존 dedup으로 재사용하므로 순환이
  아닙니다. 순환 판정 단위는 node key(LibraryId + commit)라 같은 library의 다른 버전 체인
  (A v2 → A v1)은 순환이 아닙니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 22 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| A→B→A fixture 실패, 경로 `a > b > a`·node A v1, 메시지에 전체 경로(`transitive_cycle_fails_with_full_path`) | 통과 |
| 자기 의존(A→A) `a > me`, 긴 순환 `x > a > b > c > a` 전체 경로 | 통과 |
| diamond·root 중복 직접 선언·같은 library 다른 버전 체인은 순환 아님(`diamond_and_repeated_root_reference_are_not_cycles`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 순환 판정은 node key 단위입니다. 서로 다른 commit을 번갈아 가리키는 비정상 체인은 commit 수만큼
  유한하게 진행한 뒤 같은 key에서 탐지됩니다. 깊이·node 수 상한 정책은 아직 없습니다.

## 상태 정정(통합 리뷰, 2026-11-14)

- `[x]` → `[~]`: 엄격 선행 사슬에 `[~]` 행(XV.1.1 등)이 있어 함께 `[~]`로 둡니다. 이 행 자체의 검증은 dependencies crate 안에서 통과했습니다.
