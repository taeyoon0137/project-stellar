# 작업 보고서: XV.5.5 missing cache·offline library 부분 열기

- WBS: XV.5.5
- 작업명: missing cache/offline entry는 library resource unavailable diagnostic을 만들고 project 기본 데이터는 읽기 가능
- 상태: 완료 (`[x]`) — 2026-11-23 library-finish, 아래 "작업 (2026-11-23, library-finish)" 절. 이전 기록: `[~]` 유지 — 남은 범위·결함 있음(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — library 부분 로드와 진단은 완료했지만 프로젝트 열기 경로에 연결하지 않았습니다(9A 리뷰 P2).
- 선행: XV.3.3(`[x]`), III.2.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 실제 Git fixture 테스트는 시스템 `git`을 씁니다.

## 변경 파일 목록

- `crates/dependencies/src/library_load.rs`(신규) — `load_locked_libraries`, `LoadedLibraries`,
  `LibraryUnavailableDiagnostic`, `UnavailableReason`, `ResourceUnavailable`, `ResourceResolution`,
  `PartialClosure`, `unresolved_external`.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.5.5.md`(신규), 계획서(XV.5.5 `[ ]`→`[x]`).

## 구현 요약

- **lock 기준 로드**: lock의 node마다 entry를 순서대로 시도해 `load_cached_snapshot`(XV.5.1)으로 읽습니다.
  `open_entry`만 쓰므로 Git·network를 실행하지 않습니다. 기대 content hash는 lock 값입니다.
- **부분 열기**: 실패한 node는 오류가 아니라 `LibraryUnavailableDiagnostic`(node, 도달 경로, 이유)으로
  남깁니다. 이유는 `MissingCacheEntry`(offline·미 다운로드·정리), `ContentHashMismatch`(손상, 다른 commit
  자동 선택 없음), `CacheFailure`, `ExportUnreadable`, `InvalidExports`입니다. 함수가 실패하는 경우는
  lock 구조(부모 경로) 오류뿐이라 나머지 library와 프로젝트 기본 데이터는 그대로 열립니다.
- **library resource unavailable**: 사용할 수 없는 node를 가리키는 참조는
  `ResourceResolution::Unavailable(ResourceUnavailable)`입니다. 미선언 이름 같은 해석 실패
  (`Unresolved`)와 구분합니다. `unresolved_at`·`unresolved_external`은 원래 StableId·기대 타입·발견
  위치를 보존한 `UnresolvedRef`(III.2.4)를 만듭니다.
- **부분 전이 해석**: `resolve_transitive_partial`은 사용할 수 없는 library를 만나도 멈추지 않고, 해석된
  resource·unavailable 참조·해석 실패를 나눠 돌려줍니다.
- 진단 문자열에는 repository URL을 넣지 않습니다(XV.1.4).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-dependencies` | 종료 코드 0 |
| `cargo test -p stellar-dependencies` | 84 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| offline missing fixture(실제 Git, a·b만 cache, a→c의 c 없음): 로드 성공·Git 호출 0, c 진단 1건(경로 `a > c`, URL 원문 없음), b component·token 해석, a card 해석 + c chip·ink 참조 unavailable 2건, `UnresolvedRef` 보존, c 직접 참조는 미선언 오류(`offline_missing_library_opens_partially_with_unavailable_diagnostic`) | 통과 |
| lock content hash 변조 → b `ContentHashMismatch`(export 미호출), export 읽기 실패 → b만 unavailable, a 계속 로드(`corrupted_or_unreadable_entries_become_diagnostics_not_errors`) | 통과 |
| `unresolved_external`이 StableId·기대 타입·위치 보존(`unresolved_external_preserves_reference_identity`) | 통과 |

## 차이(작업 지시와 다른 점)

- "project 기본 데이터 읽기"는 library 로드가 프로젝트 열기를 실패시키지 않는 계약(부분 결과 반환)으로
  검증했습니다. 실제 프로젝트 열기 경로(project·runtime crate)에 이 로드를 연결하는 일은 이 lane 범위 밖입니다.

## 남은 위험 / 한계

- 진단 집계(XXV.2.1 DiagnosticIndex)와 UI 표시는 후속 작업입니다.
- cache entry 손상 판정은 cache index의 content hash 비교입니다. entry 파일 내용 재계산 검사는
  XV.4.4 `verify_lock_against_cache`를 따로 호출해야 합니다.

## 리뷰 수정(9A 리뷰 P1, 2026-11-14)

- 문제: `unresolved_external`·`ResourceUnavailable::unresolved_at`이 외부 참조를 core-types `UnresolvedRef`로
  바꿔 의존성 선언 이름과 LibraryId를 잃었습니다. 모듈 문서는 이 값을 문서에 남겨도 된다고 안내했으므로,
  offline으로 열고 저장하면 cache가 돌아온 뒤 어느 의존성의 참조였는지 알 수 없어 다시 연결할 수 없었습니다.
- 재현: `unresolved_external_keeps_dependency_name_and_library` — 선언 이름 또는 LibraryId만 다른 두 참조의
  미해결 형태가 같아져 실패했습니다(수정 전 `assertion left != right failed`).
- 수정: 참조 전체(선언 이름·LibraryId·종류·StableId)와 위치를 보존하는 `UnresolvedExternalRef`를 추가하고
  `unresolved_external`·`unresolved_at`이 이 타입을 돌려줍니다. core-types 변환은 `diagnostic_ref()`(진단
  집계용 손실 변환)로만 남겼습니다. 모듈 문서와 ADR-0027 결정 7을 함께 고쳤습니다.
- 검증: 위 재현 테스트 통과, canonical 참조 텍스트 + 위치로 다시 만든 값이 같음
  (`unresolved_external_preserves_reference_identity`), offline fixture의 미해결 참조가 선언 이름 `c`·
  library 3·StableId를 보존(`offline_missing_library_opens_partially_with_unavailable_diagnostic`).

## 리뷰 수정(9A 리뷰 P5, 2026-11-14)

- 문제: `load_locked_libraries`는 node별 lock entry 중 cache에서 찾은 첫 entry를 쓰고, 같은 node의 다른
  entry가 content hash·schema version을 다르게 기록해도 확인하지 않았습니다. 변조되었거나 서로 맞지 않는
  entry가 entry 순서에 따라 조용히 무시되었습니다.
- 재현: `inconsistent_entries_for_one_node_make_it_unavailable` — node a에 두 번째 경로(`b > a`) entry를
  추가하고 content hash·schema version·전이 의존성 중 하나만 바꾼 lock에서 수정 전에는 a가 로드되어
  실패했습니다.
- 수정: 같은 node의 entry는 content hash·schema version·전이 의존성이 모두 같아야 합니다. 다르면 어느
  entry도 시도하지 않고 `UnavailableReason::InconsistentLockEntries` 진단(두 경로 포함)으로 남깁니다.
  repository 표기와 선택자는 선언한 부모마다 다를 수 있어 비교하지 않습니다.
- 검증: 위 테스트 통과(세 변조 모두 a unavailable·b 정상, 일치하는 두 번째 경로는 정상 로드).
- 한계: 같은 검사를 lock validator(XV.4.2 `LockDocument::from_node`)에 넣는 일은 이 수정 범위 밖입니다.

## 상태 정정(9A 리뷰 P2, 2026-11-14)

- `[x]` → `[~]`: 작업 행의 "project 기본 데이터는 읽을 수 있게"와 검증 열 "offline missing fixture partial
  open"은 library 집합 수준에서만 검증했습니다. `load_locked_libraries`가 실패 node를 진단으로 남기고 나머지를
  돌려주는 것까지입니다. 실제 프로젝트 열기 경로(project crate의 `stellar.lock.yaml` 읽기)는 이 함수를
  호출하지 않으므로, offline에서 **프로젝트**가 부분적으로 열리고 기본 데이터를 읽는 것은 확인하지 않았습니다.
  또 운영 export 공급자가 없어 snapshot은 테스트 공급자로만 채워집니다(XV.5.1 차이).
- 남은 일: 프로젝트 열기 경로에 이 로드를 연결하고, offline missing fixture로 프로젝트 기본 데이터가 열리고
  누락 library가 진단으로 남는 것을 확인하면 `[x]`로 올립니다.
- unavailable 진단 생성, 미해결 외부 참조 보존(P1), 같은 node entry 불일치 진단(P5)은 이 crate에서
  완료했습니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 남은 범위·결함 있음.
- 근거: library 부분 로드와 unavailable 진단은 있지만 프로젝트 열기 경로가 `load_locked_libraries`를 부르지 않아 `[~]`를 유지합니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/dependencies/src/library_load.rs`: 실패 node를 진단으로 남기고 나머지를 반환. project·runtime·ops에서 부르는 곳이 없습니다.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-dependencies` | 96 passed |

### 남은 조건과 finding

- 결함 finding(소유 lane 없음 → backlog): 프로젝트 열기 경로에 연결하고 offline missing fixture로 프로젝트 기본 데이터가 열리고 누락 library가 진단으로 남는지 확인합니다.

## 작업 (2026-11-23, library-finish): 프로젝트 library 열기 함수와 offline fixture

### 한 일

- `crates/project/src/library_open.rs`(신규): `open_project_libraries`가 `stellar.lock.yaml`을 loader 상한까지 읽어 제품 lock parser로
  검증하고, 제품 검증기·export 공급자로 `load_locked_libraries`를 부릅니다. 실패하지 않습니다: lock 없음은 의존성 없는 프로젝트,
  lock 문제는 `LibraryOpenProblem`, node 문제는 unavailable 진단입니다(ADR-0119 결정 6).

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 101 passed, 0 failed; doctest 1 passed |
| `cargo test -p stellar-project --lib library_` | library_entry·library_git 단위 시험 통과, 0 failed |
| `cargo clippy -p stellar-dependencies -p stellar-project -p stellar-assets -p stellar-integration-tests --all-targets -- -D warnings` | 경고 0 |
| `cargo xtask validate-adr --base 0c226a06` | 종료 코드 0 |
| `cargo test -p stellar-integration-tests --test library_partial_open` | 1 passed |

| 검사 | 결과 |
|---|---|
| **offline missing fixture partial open**: 사용 프로젝트가 a·b를 lock, offline cache에 a만 있음, library 저장소를 지운 뒤(Git·network 불가) 열기 → b만 `MissingCacheEntry` 진단(URL 원문 없음), a resource는 해석, b resource는 `ResourceResolution::Unavailable` | 통과 |
| 프로젝트 기본 데이터(텍스트 원본 문서)는 같은 상태에서 제품 원본 reader로 열리고 semantic snapshot이 같음 | 통과 |
| lock 손상 → `LockInvalid`만 남고 문서는 열림, lock 없음 → 빈 library | 통과 |

### 남은 위험

- 창 session·CLI·MCP 열기는 아직 이 함수를 부르지 않습니다. 문서가 외부 참조를 저장하지 않아 그 결과를 쓸 소비자가 없기 때문입니다.
  외부 참조 저장과 함께 그 열기 경로에서 부르게 해야 합니다.
