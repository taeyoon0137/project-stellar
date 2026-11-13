# 작업 보고서: XV.1.1 의존성 선언 entry 모델

- WBS: XV.1.1
- 작업명: `stellar.yaml` dependency entry(logical name, library id, source kind Git, repository URL, version selector, optional local override) 정의
- 상태: 완료 (`[x]`)
- 선행: IV.1.1(`[x]`), Phase XIV(단계 선행 — 완화 적용)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/node.rs`(신규) — 형식 중립 트리 `DeclNode`, `SchemaError`, `ValueError`, 스키마 헬퍼.
- `crates/dependencies/src/declaration.rs`(신규) — `LibraryId`, `LogicalName`, `SourceKind`,
  `RepositoryUrl`, `CommitId`, `RefName`, `SelectorKind`, `VersionSelector`, `LocalOverridePath`,
  `DependencyDeclaration`, `dependencies_to_node`/`dependencies_from_node`.
- `crates/dependencies/src/lib.rs` — 모듈 등록, crate 문서.
- `docs/task-reports/XV.1.1.md`(신규), 계획서(XV.1.1 `[ ]`→`[x]`).

## 구현 요약

- **형식 중립 트리**: 이 crate는 core-types에만 의존하므로 제한 YAML을 직접 읽지 않습니다.
  `DeclNode`(스칼라/시퀀스/매핑, 키 순서 보존)는 project의 `YamlValue`와 1:1로 대응하며, project가
  변환을 연결합니다. 스키마 오류는 경로와 위반 규칙만 담고 입력 값(URL·경로)을 복사하지 않습니다.
- **entry 필드**: `source`(현재 `git`만), `library_id`(라이브러리 프로젝트 `project.id` UUID →
  `LibraryId(StableId)`), `repository`, `version`, 선택 `local_override`. 알 수 없는 키는 거부합니다.
- **version 선택자**: tag는 아키텍처 20.3 예시와 같은 스칼라(`version: v2.4.0`), branch·commit은 한
  키 매핑(`{branch: main}`, `{commit: <hex>}`)이 canonical 형식입니다. 입력의 `{tag: ...}` 매핑은
  스칼라로 정규화합니다. 키가 0개·2개 이상이면 `InvalidSelector`입니다.
- **값 규칙**: commit은 40자(SHA-1)·64자(SHA-256) hex만 받고 소문자로 정규화합니다(축약 금지).
  tag·branch는 `git check-ref-format`의 보수적 부분집합, logical name은 ASCII 영숫자 시작 +
  `-_.`만 허용합니다. repository URL은 제어 문자·공백, `-` 시작(Git option 주입), 명령을 실행하는
  remote helper transport(`ext::`, `fd::`)를 거부합니다.
- 같은 logical name 중복은 파싱에서 보존하고 판정은 XV.1.3 validation에 맡깁니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 6 passed, 0 failed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| tag·branch(+local override)·commit 3개 entry: 모델→노드→모델 동일, 노드→모델→노드 동일, canonical 키 순서·형식 고정(`schema_round_trip_preserves_every_field_and_order`) | 통과 |
| `{tag: v1}` 매핑 → 스칼라 정규화, 대문자 commit 소문자화, SHA-256 64자 commit(`tag_mapping_form_and_uppercase_commit_normalize`) | 통과 |
| 누락 필드·알 수 없는 키·타입 불일치·알 수 없는 source·잘못된 UUID·`-` 시작 URL·`ext::` transport·축약 commit·2키/빈 version·알 수 없는 선택자 키·잘못된 ref/logical name을 경로와 함께 거부(`schema_violations_are_rejected_with_paths`) | 통과 |
| 같은 이름 중복 entry를 validation용으로 보존(`duplicate_names_are_preserved_for_validation`) | 통과 |

## 차이(작업 지시와 다른 점)

- 단계 선행 "Phase XIV"는 사용자 지시(2026-11-14)에 따라 완화했습니다. 이 작업은 Git 실행 코드가
  필요하지 않습니다.
- schema round-trip은 `DeclNode` 수준에서 검증했습니다. `stellar.yaml` 텍스트 ↔ `YamlValue` ↔
  `DeclNode` 연결은 project crate가 담당하며(의존 방향상 이 crate는 project를 쓸 수 없음), 기존
  `project::manifest::Dependency`(name·source·repository·version 문자열)를 이 모델로 바꾸는 연결도
  project 작업 범위입니다.
- 아키텍처 20.3 예시에는 `library_id`가 없지만 계획 행이 entry에 library id를 요구하므로 필수
  필드로 정의했습니다.

## 남은 위험 / 한계

- tag의 스칼라 단축 형식 때문에 branch 이름을 스칼라로 적으면 tag로 해석됩니다. resolve 단계에서
  해당 tag가 없으면 명시적 오류가 나지만, 같은 이름의 tag와 branch가 둘 다 있으면 tag가 선택됩니다.
- URL 형식 검증은 안전 규칙(주입·transport)만 다룹니다. 실제 접근 가능 여부는 시스템 Git이
  판단합니다.

## 상태 정정(통합 리뷰, 2026-11-14)

- `[x]` → `[~]`: round-trip은 dependencies crate의 중립 tree에서만 증명했고, 실제 stellar.yaml 파서(project crate)는 새 스키마를 아직 받지 않습니다.

## 상태 복원(deps_wiring lane, 2026-11-14)

- `[~]` → `[x]`: 엄격 선행 IV.1.1 `[x]`(단계 선행 Phase XIV는 lane 규칙으로 완화, Git 실행 코드 불필요).
- 실제 파서 경로 검증: 실제 `stellar.yaml` 텍스트(tag 스칼라, branch 매핑 + local override, commit 매핑)가 제한 loader → `Manifest::from_yaml` → canonical writer로 byte 동일하게 round-trip(`manifest::tests::dependency_entry_schema_round_trips_through_stellar_yaml_parser`), 스키마 위반은 실제 파서가 경로와 함께 거부(`dependency_schema_violations_are_rejected_by_stellar_yaml_parser`).
- 공통 근거: `Fix: stellar.yaml 의존성 선언을 실제 매니페스트 파서에 연결`(리뷰 P2), `Fix: stellar.lock.yaml entry 형식을 IV.1.6 모델 하나로 통합`(리뷰 P3, ADR-0016), `Fix: resolver library 매니페스트 읽기를 실제 stellar.yaml 파서로 연결`.
- 실행(`DEV-MAC`): `cargo test -p stellar-project` lib 261 passed·통합 17 passed, `cargo test -p stellar-dependencies` 57 passed·doctest 1 passed, `cargo clippy -p stellar-project -p stellar-dependencies --all-targets -- -D warnings` 경고 0, `cargo fmt -p stellar-project -p stellar-dependencies -- --check` 통과.
