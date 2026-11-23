# 작업 보고서: XXV.2.1 무결성 진단 공통 DiagnosticIndex 집계

- WBS: XXV.2.1
- 작업명: OrphanedBinding, missing token·font·asset·library, invalid accessibility, target unsupported, migration warning을 공통 DiagnosticIndex로 집계
- 상태: 구현 완료. 선행 XV.5.5가 `[~]`라 통합 판정 때 계획 1.2 규칙 4를 적용해야 합니다.
- 결정: ADR-0095 결정 1·2·7

## 필수 실행 환경

- `DEV-MAC`(Rust 단위 test).

## 변경 파일 목록

- `crates/integrity/`(신규 crate `stellar-integrity`)
  - `Cargo.toml`, `src/lib.rs`.
  - `src/diagnostic.rs`: 진단 종류 10개, 심각도, 정체성 key, 안정 ID.
  - `src/sources.rs`: 생산자 결과 변환과 내장 checker.
  - `src/index.rs`: 집계, 개수 요약, scope 교체.
  - `src/incremental.rs`: scope footprint 타입.
  - `src/remediation.rs`: action ID와 종류별 제공 규칙.
  - `src/tests.rs`.
- `Cargo.toml`(workspace member 한 줄), `Cargo.lock`(새 package 등록).
- `tools/xtask/src/crate_graph.rs`, `docs/architecture/crate-dependency-graph.md`(소유 밖, 새 crate 등록): `stellar-integrity` 허용
  의존과 `stellar-ops → stellar-integrity`.
- `docs/adr/ADR-0095-integrity-diagnostic-index.md`(신규).

## 구현 요약

- 생산자 판정 함수를 그대로 쓰고 결과만 옮깁니다.
  - 문서 미해결 참조: `validate_document`의 `unresolved` → missing node·token·component(내장 `ReferenceChecker`). 위치
    root가 modifier면 소유 node를 source로 씁니다.
  - Image asset: `ImagePayload::resolve_asset` → missing asset(내장 `AssetChecker`, asset store 조회가 있을 때만).
  - `BindingStore::orphans` → orphaned binding(원인별 code 4개).
  - 호출자가 모은 `UnresolvedRef`(token registry 등) → missing token.
  - `MissingFontDiagnostic` → 영향 Text마다 missing font.
  - `AccessibilityIssue` → invalid accessibility(심각도 그대로).
  - `LibraryUnavailableDiagnostic`(프로젝트 단위)·`UnresolvedExternalRef`(문서 참조) → missing library.
  - Target Profile `MultiProfileReport` → Unsupported·NotRegistered(오류), Deprecated(경고), pin registry 없음(경고).
    Native·Approximated는 옮기지 않습니다.
  - 메모리 migration(from→to) → migration warning(프로젝트 단위).
- index는 key 순서 map 하나에 진단을 둡니다. 종류×심각도·문서별 개수는 삽입·삭제 때 갱신합니다.
- (문서, 생산자) scope 단위로 교체합니다. 같은 내용으로 교체하면 revision을 올리지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-integrity` | 통과(17 passed) |
| `cargo clippy -p stellar-integrity --all-targets --locked -- -D warnings` | 경고 0 |
| `cargo xtask validate-crate-graph` | exit 0 |

| 검사 | 결과 |
|---|---|
| fixture count 정확: 무작위 문서에 깨진 clip path·mask source·Image asset을 더하고 생산자 8종 결과를 넣으면 종류별 개수가 기대값과 같음(missing node +2, asset +1, orphan 3, font 2, 접근성 1, library 1, target 2, migration 1, token +1), 오류 2 | 통과(`tests::fixture_counts_are_exact_per_kind_and_severity`) |
| 필터·개수 표가 전체 탐색 결과와 같음 | 통과(`index::tests::filtered_pages_equal_filtered_scan_in_key_order`) |
| scope 교체가 다른 scope를 건드리지 않음 | 통과(`index::tests::replace_scope_touches_only_that_scope_and_stale_cursor_is_rejected`) |

## 차이(작업 지시와 다른 점)

- missing token은 문서 레코드에 token 참조 저장 위치가 아직 없습니다. 그래서 binding orphan과 호출자가 모은 미해결 token
  참조에서만 나옵니다.
- missing component는 XXV.2.1 목록에 없지만 같은 미해결 참조 종류라 함께 옮깁니다. 저장 문서의 컴포넌트 정의 누락 판정은
  `stellar validate`(`missing-component-definition`)에 그대로 두고 다시 판정하지 않습니다.

## 남은 위험 / 한계

- headless snapshot에서는 미해결 참조와 migration만 수집합니다. asset store·font registry·접근성 tree·Target Profile·library
  cache·binding 저장소 입력을 snapshot에서 만드는 경로가 생기면 `stellar_ops::integrity::diagnose_snapshot`에 연결해야
  합니다(ADR-0095 결정 7).
- 앱 runtime(문서 thread)에는 아직 연결하지 않았습니다(UI 재설계 때).
