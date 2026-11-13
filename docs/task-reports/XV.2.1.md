# 작업 보고서: XV.2.1 DependencyNodeKey

- WBS: XV.2.1
- 작업명: `DependencyNodeKey`를 LibraryId + ResolvedCommit으로 정의해 동일 library 여러 버전을 별도 node로 허용
- 상태: 완료 (`[x]`)
- 선행: XV.1.1(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/graph.rs`(신규) — `DependencyNodeKey`(`new`, `of_pin`, `Display`).
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.2.1.md`(신규), 계획서(XV.2.1 `[ ]`→`[x]`).

## 구현 요약

- `DependencyNodeKey { library_id: LibraryId, commit: CommitId }`. 파생 `Ord`가 LibraryId → commit
  순서라 같은 library의 버전이 정렬상 인접합니다. `Hash`·`Eq`는 두 필드 모두를 사용하므로 같은
  LibraryId라도 commit이 다르면 다른 node입니다(아키텍처 30.3 버전별 격리).
- `Display`는 `<library-id>@<commit>`입니다. `CommitId`가 전체 hash만 허용하므로 key도 exact
  commit만 담습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 17 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 같은 library v1/v2 key가 서로 다르고 BTreeMap·HashSet에 동시에 2개 존재, 같은 commit은 동일 key, LibraryId 우선 정렬, Display 형식(`same_library_two_versions_are_distinct_keys`) | 통과 |
| lock pin → key(`key_from_lock_pin`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 아키텍처 30.3의 resource 참조 키는 `DependencyInstanceId + LibraryId + ResolvedCommit +
  ResourceStableId`입니다. DependencyInstanceId와 resource 단위 key는 library snapshot(XV.5.1)
  범위입니다.
