# 작업 보고서: XV.5.2 ExternalComponentRef·ExternalTokenRef

- WBS: XV.5.2
- 작업명: 사용 project document가 library resource를 참조하는 `ExternalComponentRef`, `ExternalTokenRef`
- 상태: 완료 (`[x]`)
- 선행: XV.5.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/external_ref.rs`(신규) — `ExternalComponentRef`, `ExternalTokenRef`,
  `ExternalResourceRef`, `ExternalRefParseError`, 접두사·길이 상수, 테스트 fixture.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/adr/ADR-0027-library-snapshot-resource-key.md` — 결정 7(외부 참조 형식)과 대안·호환성 추가.
- `docs/task-reports/XV.5.2.md`(신규), 계획서(XV.5.2 `[ ]`→`[x]`).

## 구현 요약

- **저장 값**: 참조하는 쪽 `stellar.yaml`의 의존성 logical name(DependencyInstanceId), LibraryId, resource
  StableId. commit은 저장하지 않고 scope의 lock이 정합니다(update 후 문서 재기록 없음, ADR-0027 결정 7).
- **타입 분리**: component·token 참조는 서로 다른 타입이고(core-types `ComponentRef`·`TokenRef`를 감쌈),
  `ExternalResourceRef`가 둘을 묶습니다. 해석 실패 시 `UnresolvedRef`에 남길 `ExpectedRefType`을 줍니다.
- **canonical 텍스트**: `component:<name>@<library id>/<stable id>`, `token:<name>@<library id>/<stable id>`.
  `Display`가 쓰고 `FromStr`가 읽습니다. 길이 상한(`MAX_EXTERNAL_REF_LEN`)을 먼저 확인하고, 종류 접두사·
  구분자·이름 규칙(XV.1.1)·UUID·canonical 여부(소문자 UUID)·종류 일치를 검사합니다. 오류에는 입력 원문을
  담지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-dependencies` | 종료 코드 0 |
| `cargo test -p stellar-dependencies` | 73 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| component·token 참조 텍스트 golden, 타입별·통합 parser 왕복, 두 번째 왕복 bytes 동일, 최대 길이 이름 왕복(`references_round_trip_through_canonical_text`) | 통과 |
| 빈 값·알 수 없는 종류·구분자 누락·잘못된 이름·비 UUID·추가 component·대문자 UUID·상한 초과·종류 불일치 거부, 오류에 원문 없음(`malformed_and_non_canonical_references_are_rejected`) | 통과 |
| 같은 이름·library·StableId라도 component·token 참조는 다름(`component_and_token_refs_are_distinct_types`) | 통과 |

## 차이(작업 지시와 다른 점)

- document crate의 instance payload·binding 표현식에 외부 참조를 넣는 통합은 하지 않았습니다. document
  crate는 이 lane 범위 밖이고 dependencies에 의존할 수 없습니다(crate graph). round-trip은 참조 타입의
  canonical 텍스트로 검증했습니다.

## 남은 위험 / 한계

- 문서 chunk에 이 텍스트를 저장하는 통합 작업이 형식을 바꾸면 ADR-0027 결정 7을 함께 갱신해야 합니다.

## 상태 검토(9A 리뷰 P2, 2026-11-14)

- `[x]` 유지: 검증 열 "round-trip"은 이 작업이 구현하는 `ExternalComponentRef`·`ExternalTokenRef`의
  canonical 텍스트 왕복으로 검증했습니다.
- 사실 기록: 문서는 아직 이 참조를 저장하지 않습니다(차이 절). document crate는 dependencies에 의존할 수
  없어(crate graph) 문서 chunk에 canonical 텍스트로 넣는 통합 작업이 필요하고, 그 작업이 문서 저장·읽기
  왕복을 검증해야 합니다. 해석할 수 없는 참조는 참조 전체를 보존하는 `UnresolvedExternalRef`(XV.5.5 리뷰
  수정 P1)로 남깁니다.

## 작업 (2026-11-24, library-materialize): 사용 프로젝트가 외부 component 참조를 저장

- 결정: [ADR-0137](../adr/ADR-0137-library-stored-refs-and-materialize-commands.md) 결정 1. "차이"와 "상태 검토"의 남은 일(문서가
  참조를 저장하지 않음)을 처리했습니다.
- 저장 위치: 사용 프로젝트 `stellar.yaml` 의존성 선언의 `components`(component StableId 시퀀스). 한 항목이 `ExternalComponentRef`의 세
  값(선언 이름 = 매핑 키, LibraryId = `library_id`, StableId = 항목)을 이룹니다. document crate는 dependencies에 의존할 수 없어 instance
  payload 대신 의존성 선언 옆에 둡니다(같은 저장 값, ADR-0027 결정 7).
- 판정: `classify_component`가 저장된 참조를 먼저 봅니다(하나면 외부, 둘 이상이면 모호). 저장되지 않은 instance는 이전 프로젝트 호환
  읽기(`ComponentOrigin::Undeclared`)이고 진단 `library-ref/undeclared`가 저장을 알립니다. 저장된 참조는 snapshot이 없어도 외부라 offline
  override 설정이 `LibraryUnavailable`로 거부됩니다.
- 저장 명령: `stellar library add … --component <id>`, `stellar library import <경로> <이름> <id>... [--remove]`.

### 변경 파일

- `crates/dependencies/src/declaration.rs`(`components` 필드·parser·writer), `crates/dependencies/src/node.rs`(`ValueError` 3개),
  `crates/dependencies/src/component_imports.rs`(신규), `crates/dependencies/src/ref_resolution.rs`(`with_imports`·`imports`),
  `crates/dependencies/src/lib.rs`
- `crates/project/src/library_open.rs`(매니페스트 읽기·`declared_only`·`ManifestInvalid`), `crates/project/src/library_refs.rs`
  (판정 순서·`Undeclared`·`ExternalUsages::undeclared`), `crates/project/src/library_guard.rs`
- `crates/ops/src/library.rs`(`has_external`, 매니페스트 문제 진단, lock 없을 때도 저장 참조 유지), `crates/runtime/src/library.rs`
- 구조체 literal 컴파일 보정: `crates/project/src/manifest.rs`(fixture), `crates/project/tests/restricted_yaml_fuzz.rs`
- `docs/spec/stellar-format.md`(형식 표), `docs/adr/ADR-0137-library-stored-refs-and-materialize-commands.md`(신규)

### 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies --lib declaration` | 0 — 저장 참조 왕복·정규화·거부 시험 포함 통과 |
| `cargo test -p stellar-dependencies --lib component_imports` | 0 — 1 passed |
| `cargo test -p stellar-ops --lib library_materialize::tests::stored_refs_decide_external_components_and_document_diagnostics` | 0 — 1 passed(제품 명령 커밋 이후) |

| 검사 | 결과 |
|---|---|
| **문서 저장·읽기 왕복**: `components`는 StableId 오름차순으로 쓰고 입력 순서를 정규화, 비면 키 없음, 대문자 UUID·중복·4096개 초과·시퀀스 아님 거부(경로 포함) | 통과 |
| 저장된 참조가 없으면 `Undeclared`, import 뒤 `External(ds)`, 저장된 참조는 offline에서도 외부라 override 설정 거부 | 통과 |

### 남은 위험 / 한계

- 같은 component를 instance마다 다른 의존성(같은 library 두 버전)에서 쓰는 경우는 표현하지 않습니다(모호 진단).
- 외부 token 참조는 아직 문서에 저장 위치가 없습니다(binding 표현식 쪽, 이 작업 범위 밖).
