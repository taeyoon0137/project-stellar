# 작업 보고서: VIII.5.2 scope 토큰 registry

- WBS: VIII.5.2
- 작업명: 프로젝트·라이브러리·문서 scope token registry, 동일 이름 허용 여부와 ID 우선 resolution 명시
- 상태: 완료 (`[x]`)
- 선행: VIII.5.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/bindings/src/registry.rs`(신규) — `TokenScope`, `TokenRegistry`, `ResolvedToken`, `NameResolution`,
  `ReferenceIssue`, `RegistryError`와 테스트.
- `crates/bindings/src/lib.rs` — `registry` 모듈 등록.
- `docs/task-reports/VIII.5.2.md`(신규), 계획서(VIII.5.2 `[ ]`→`[x]`).

## 구현 요약

규칙은 `registry.rs` 모듈 문서에 고정했습니다.

- **scope**: `Project`, `Library(StableId)`, `Document(StableId)`. 문서는 같은 문서·프로젝트·모든 라이브러리를,
  프로젝트는 프로젝트·라이브러리를, 라이브러리는 자기 자신만 봅니다.
- **ID 우선**: `TokenRef`는 registry 전체에서 한 scope에만 존재하며(`add_scope`·`insert`가 다른 scope의 같은 ID
  거부), `resolve(TokenRef)`는 이름·shadowing과 무관하게 정확히 한 정의를 돌려줍니다.
- **같은 이름 허용 여부**: scope 안에서는 유일(VIII.5.1 TokenSet), scope 사이에서는 허용합니다.
- **이름 resolution(shadowing)**: 계층 우선순위 문서 → 프로젝트 → 라이브러리. 이름이 있는 첫 계층의 토큰을
  고르고, 같은 계층(라이브러리 여러 개)에 둘 이상이면 `Ambiguous`입니다. 이름은 GUI 검색·작성 보조용입니다.
- **참조 가시성**: `reference_issues()`가 없는 토큰 참조(`Dangling`)와 scope에서 보이지 않는 참조(`NotVisible`:
  라이브러리→프로젝트, 프로젝트→문서, 다른 문서)를 정렬된 집합으로 보고합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 종료 코드 0 (6 unit passed, 0 failed) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **scope shadowing fixture**(`scope_shadowing_resolves_names_by_tier`): 같은 이름 `color.primary`가 라이브러리 L·프로젝트·문서 D에 있을 때 D→문서 토큰, 다른 문서 E→프로젝트 토큰, 프로젝트→프로젝트 토큰, L→L 토큰, M→없음; 두 라이브러리의 `color.accent`는 `Ambiguous`(scope 순) | 통과 |
| ID 해석은 이름이 가려진 라이브러리 토큰도 정확히 해석, 정상 가시성 참조는 issue 0(`id_resolution_ignores_names_and_shadowing`) | 통과 |
| 다른 scope의 같은 ID 거부, 중복·미등록 scope 거부, 보이지 않는 참조 3종·dangling 1종 정확, 제거 시 색인 정리(`ids_are_unique_across_scopes_and_visibility_is_checked`) | 통과 |

## 차이(작업 지시와 다른 점)

- 신규 모듈이므로 테스트는 구현과 함께 작성했습니다(구현 전 상태는 컴파일 불가).
- 라이브러리 ID는 `stellar-dependencies`의 `LibraryId`를 쓸 수 없어(허용 edge 밖) 같은 값의 `StableId`로 둡니다.

## 남은 위험 / 한계

- 라이브러리 여러 버전 동시 로드(XV.5.1: LibraryId+commit+StableId)는 반영하지 않았습니다. 현재 scope key는
  LibraryId만 씁니다.
- 라이브러리 토큰의 읽기 전용 정책은 command(VIII.5.5)에서 적용합니다.
