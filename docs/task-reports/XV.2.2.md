# 작업 보고서: XV.2.2 dependency graph edge 경로와 node dedup

- WBS: XV.2.2
- 작업명: dependency graph edge에 parent dependency path 기록, 같은 resolved node dedup
- 상태: 완료 (`[x]`)
- 선행: XV.2.1(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/graph.rs` — `DependencyPath`, `DependencyEdge`, `DependencyNode`,
  `DependencyGraph`.
- `crates/dependencies/src/resolver.rs`(신규) — `LibraryManifest`, `LibraryManifestSource` trait,
  `ManifestSourceError`, `ResolveError`, `resolve_dependency_graph`, 테스트용 `Catalog`.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.2.2.md`(신규), 계획서(XV.2.2 `[ ]`→`[x]`).

## 구현 요약

- **edge**: 선언 한 건마다 `DependencyEdge { parent, child, path, repository, requested }`를
  기록합니다. `path`는 사용 프로젝트 root부터 logical name 경로(`a > c`)이고 `parent == None`이면
  root 선언입니다.
- **dedup**: node는 `BTreeMap<DependencyNodeKey, DependencyNode>`에 하나만 둡니다. 이미 있는 key에
  다시 도달하면 edge만 추가하고 매니페스트를 다시 읽거나 하위를 재탐색하지 않습니다. node는 직접
  의존 node 집합(전이 의존성)과 매니페스트 schema version을 가집니다.
- **resolver**: 명시적 stack 깊이 우선 탐색(깊은 체인에서도 호출 stack 미소모). 선택자 해석은
  `SelectorResolver`, 매니페스트는 `LibraryManifestSource` trait으로 받고 `(repository 비교 키,
  선택자)`를 세션 memo로 한 번만 해석합니다. 선언 library id와 commit 매니페스트 `project.id`가
  다르면 `LibraryIdMismatch`입니다. 모든 오류는 dependency path를 담고 URL은 담지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 20 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| diamond(root→a,b; a→c v1; b→c v1): node 3·edge 4, edge 목록(부모·자식·경로) 정확, c로 들어오는 경로 `a > c`·`b > c`, a·b의 전이 의존성 = {c}, c 매니페스트 1회·선택자 해석 3회(`diamond_dependency_dedups_node_and_records_both_paths`) | 통과 |
| 없는 tag·library id 불일치·읽을 수 없는 매니페스트가 각각 실패 경로(`a > c`, `a`)와 함께 오류, 메시지에 URL 없음(`failures_carry_dependency_path`) | 통과 |
| node가 commit 매니페스트 schema version 기록(`node_records_manifest_schema_version`) | 통과 |

## 차이(작업 지시와 다른 점)

- library 매니페스트 읽기(`git show <commit>:stellar.yaml` + 제한 YAML 파싱)는 project crate가
  `LibraryManifestSource`로 구현합니다. 이 작업의 fixture는 in-crate `Catalog`입니다.

## 남은 위험 / 한계

- 순환은 이 단계에서 dedup으로 종료만 보장합니다. 순환 오류 보고는 XV.2.3입니다.
- 같은 node에 서로 다른 URL(미러)로 도달하면 첫 edge의 URL로 매니페스트를 읽습니다. commit hash가
  내용을 고정하므로 결과는 같습니다.

## 상태 정정(통합 리뷰, 2026-11-14)

- `[x]` → `[~]`: 엄격 선행 사슬에 `[~]` 행(XV.1.1 등)이 있어 함께 `[~]`로 둡니다. 이 행 자체의 검증은 dependencies crate 안에서 통과했습니다.

## 상태 복원(deps_wiring lane, 2026-11-14)

- `[~]` → `[x]`: 엄격 선행 XV.2.1 `[x]`.
- 실제 파서 경로 검증: 실제 `stellar.yaml` 텍스트로 만든 diamond(a→c tag, b→c branch, 같은 commit)에서 c node 1개·edge 경로 `a > c`, `b > c`(`manifest_source::tests::diamond_and_cycle_through_real_stellar_yaml_parser`).
- 공통 근거: `Fix: stellar.yaml 의존성 선언을 실제 매니페스트 파서에 연결`(리뷰 P2), `Fix: stellar.lock.yaml entry 형식을 IV.1.6 모델 하나로 통합`(리뷰 P3, ADR-0021), `Fix: resolver library 매니페스트 읽기를 실제 stellar.yaml 파서로 연결`.
- 실행(`DEV-MAC`): `cargo test -p stellar-project` lib 261 passed·통합 17 passed, `cargo test -p stellar-dependencies` 57 passed·doctest 1 passed, `cargo clippy -p stellar-project -p stellar-dependencies --all-targets -- -D warnings` 경고 0, `cargo fmt -p stellar-project -p stellar-dependencies -- --check` 통과.
