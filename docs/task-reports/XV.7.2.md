# 작업 보고서: XV.7.2 library resource Semantic Diff

- WBS: XV.7.2
- 작업명: library resource Semantic Diff를 ComponentDefinition·Property·Token 단위로 계산
- 상태: 완료 (`[x]`)
- 선행: XIV.3.7(`[x]`), XV.5.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/resource_diff.rs`(신규) — `ResourceUnit`, `ChangedField`, `ChangeKind`,
  `ResourceChange`, `LibraryResourceDiff`(`change_for`·`removed_properties`), `DiffError`,
  `diff_library_snapshots`.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.7.2.md`(신규), 계획서(XV.7.2 `[ ]`→`[x]`).

## 구현 요약

- **기준**: 같은 library의 현재·후보 snapshot(XV.7.1)을 StableId로 짝짓습니다. 이름이 바뀌어도 같은
  StableId면 `Modified { Name }`이고, 전체 blob 변경으로 축약하지 않습니다(XIV.3.7과 같은 원칙).
- **단위와 필드**: component(추가·삭제, 이름·slot·외부 참조 집합·공통 property 순서), property(추가·삭제,
  이름·타입·default·공개 여부), TokenSet(추가·삭제), token(추가·삭제, 이름·타입·값·외부 참조 집합·소속 set
  이동).
- **축약 규칙**: component가 통째로 추가·삭제되면 그 property는 따로 나열하지 않습니다. token은 독립 참조
  대상이므로 set이 사라져도 token마다 삭제로 나열합니다.
- **결정적 순서**: component(그 아래 property) → TokenSet → token, 각 StableId 순입니다. export 선언 순서와
  무관하고(property 선언 순서 자체는 `PropertyOrder`로 비교), 바뀐 필드가 없는 단위는 결과에 없습니다.
- `removed_properties`는 XV.7.3(override orphan 검증)이 쓸 삭제 property 목록입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-dependencies` | 종료 코드 0 |
| `cargo test -p stellar-dependencies` | 91 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| resource 변경 fixture(component 이름·slot·property 순서, property 이름·타입+공개 여부·default·삭제·추가, component 참조 변경·삭제·추가, set 삭제·추가, token 값·참조·이름·set 이동·추가·삭제) 17건 정확, 바뀌지 않은 단위·삭제 component의 property 미나열(`resource_change_fixture_is_exact`) | 통과 |
| 같은 snapshot diff 비어 있음, 역방향은 추가↔삭제 교환·변경 필드 동일(`identical_snapshots_diff_empty_and_reverse_swaps_added_removed`) | 통과 |
| export 선언 순서가 달라도 같은 diff(`declaration_order_of_exports_does_not_change_diff`) | 통과 |
| 다른 library snapshot 비교 거부(`different_libraries_are_not_compared`) | 통과 |

## 차이(작업 지시와 다른 점)

- `stellar-semantic-diff`(XIV.3.7) crate를 쓰지 않고 dependencies crate 안에서 계산합니다. dependencies는
  core-types에만 의존할 수 있고(crate graph), 비교 대상도 문서 node tree가 아니라 형식 중립 export record입니다.
  StableId 단위 비교 원칙은 같습니다.

## 남은 위험 / 한계

- default·값 비교는 producer canonical encoding(`ExportFingerprint`)의 bytes 같음입니다. 같은 의미의 다른 encoding은
  변경으로 보입니다(ADR-0023 결과 참고). component 정의 tree 내부 변경은 export record에 없으므로 보이지 않습니다.
