# 작업 보고서: XV.5.5 missing cache·offline library 부분 열기

- WBS: XV.5.5
- 작업명: missing cache/offline entry는 library resource unavailable diagnostic을 만들고 project 기본 데이터는 읽기 가능
- 상태: 완료 (`[x]`)
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
  집계용 손실 변환)로만 남겼습니다. 모듈 문서와 ADR-0023 결정 7을 함께 고쳤습니다.
- 검증: 위 재현 테스트 통과, canonical 참조 텍스트 + 위치로 다시 만든 값이 같음
  (`unresolved_external_preserves_reference_identity`), offline fixture의 미해결 참조가 선언 이름 `c`·
  library 3·StableId를 보존(`offline_missing_library_opens_partially_with_unavailable_diagnostic`).
