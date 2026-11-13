# 작업 보고서: XV.2.5 DuplicateLibraryVersionWarning

- WBS: XV.2.5
- 작업명: 같은 LibraryId의 여러 resolved commit이 그래프에 있으면 오류가 아니라 `DuplicateLibraryVersionWarning` 생성
- 상태: 완료 (`[x]`)
- 선행: XV.2.1(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/graph.rs` — `DependencyGraph::duplicate_library_versions`,
  `DuplicateLibraryVersionWarning`, `DuplicateVersion`, 테스트.
- `docs/task-reports/XV.2.5.md`(신규), 계획서(XV.2.5 `[ ]`→`[x]`).

## 구현 요약

- node key가 LibraryId → commit 순으로 정렬되므로 LibraryId별로 묶어 commit이 둘 이상인 library마다
  경고 하나를 만듭니다. 각 버전은 commit과 그 node로 들어온 dependency path(정렬·중복 제거)를
  가집니다. resolve 자체는 성공합니다(아키텍처 30.3: 버전별 격리, 자동 단일 버전 통합 금지).
- `Display`는 library id·버전 수·버전별 경로를 보여 주고 URL은 포함하지 않습니다. 예상 추가
  용량과 UI용 URL 전달은 XV.8.3 범위입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 28 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| v1/v2 fixture(root→a→c v1, root→b→c v2): resolve 성공, node 4, 경고 정확히 1건(library C, c1 ← `a > c`, c2 ← `b > c`), 메시지에 URL 없음(`two_versions_of_one_library_yield_exactly_one_warning`) | 통과 |
| 같은 버전 diamond는 경고 0건(`single_version_paths_produce_no_warning`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- Token 불일치·Component 호환성 경고(아키텍처 30.3)는 library snapshot과 semantic diff(XV.5·XV.7)가
  있어야 계산할 수 있습니다.

## 상태 정정(통합 리뷰, 2026-11-14)

- `[x]` → `[~]`: 엄격 선행 사슬에 `[~]` 행(XV.1.1 등)이 있어 함께 `[~]`로 둡니다. 이 행 자체의 검증은 dependencies crate 안에서 통과했습니다.
