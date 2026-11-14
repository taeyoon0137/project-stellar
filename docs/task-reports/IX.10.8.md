# 작업 보고서: IX.10.8 한 View의 다중 Target Profile capability 검사

- WBS: IX.10.8
- 작업명: 한 View를 여러 TargetProfile에 대해 동시에 validate하고 Native/Approximated/Unsupported/Deprecated diagnostic 생성
- 상태: 완료 (`[x]`)
- 선행: IX.10.6(이 lane `[x]`), IX.10.7(이 lane `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/target-profile/src/validate.rs`(신규) — `ViewNode`, `ViewModel`, `PropertyCapabilityMap`(trait),
  `CapabilityDiagnosticCode`, `Severity`, `CapabilityFinding`, `ProfileOutcome`, `ProfileReport`, `MultiProfileReport`,
  `validate_view`, `validate_profiles`, 테스트 공용 multi-profile fixture.
- `crates/target-profile/src/lib.rs` — 모듈 등록.
- `docs/task-reports/IX.10.8.md`(신규), 계획서(IX.10.8 `[ ]`→`[x]`).

## 구현 요약

- **입력**: 문서 계층에 의존하지 않는 `ViewModel`(node 문서 순서·parent·element 종류·contract, 공통값, profile override)과
  PropertyPath → capability 속성 대응 trait `PropertyCapabilityMap`(문서 계층이 구현).
- **profile마다**: pin한 release로 해석(IX.10.4) → 공통값 + profile override + variant 층 적용(IX.10.7) → 제외 node와 그 하위
  node를 건너뛰고 element·contract(중복 제거)·속성 사용(같은 node의 같은 capability는 path 오름차순 첫 path 하나)을 진단합니다.
- **진단**: Native(1, Note — 코드 생성 입력용 기록), Approximated(2, Info), Unsupported(3, Error), Deprecated(4, Warning, 대체
  subject 포함), 미등록(5, Error). `problems()`는 Note를 뺀 목록, `count(code)`는 code별 개수입니다.
- **profile 수준 결과**: pin한 release를 앱이 모르면 검사하지 않고 `RegistryUnavailable`로 보고합니다(오류로 집계). 프로젝트에 없는
  profile의 override는 `unknown_override_profiles`로 보고합니다.
- **결정론**: 결과 순서는 입력 profile 순서 → node 문서 순서 → element, contract 번호순, 속성 path순입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-target-profile` | 35 passed, 0 failed |
| `cargo clippy -p stellar-target-profile --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-target-profile -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| multi-profile fixture expected diagnostics: node 8개 View를 Web 2022·iOS 26·Android 30·Android 35에 동시에 검사, 네 profile의 진단 목록(node·subject·path·code)이 기대값과 정확히 일치 — Web의 backdrop 근사·subgrid 불가·clip rect Deprecated, iOS 전용 SF Symbol override, Android 35의 grid 하위 트리 제외·status bar Deprecated·미등록 capability; Web 상태별 개수 [14,1,1,1,0], 대체 subject, 심각도, 미등록 profile override 보고, 재실행 결과 동일 (`multi_profile_fixture_produces_expected_diagnostics`) | 통과 |
| 앱이 모르는 release(pin 7)의 profile은 검사하지 않고 `RegistryUnavailable`, 문제 없는 View는 오류 없음·contract 중복 제거 (`profile_pinned_to_unknown_release_is_reported_not_validated`) | 통과 |

## 차이(작업 지시와 다른 점)

- crate graph상 document·diagnostics crate를 쓸 수 없어 View 입력과 진단 타입을 지역 모델로 정의했습니다. 문서 tree →
  `ViewModel` 변환, `PropertyCapabilityMap` 구현, `stellar-diagnostics` 형식으로의 변환은 상위 crate 작업입니다.
- Native 사용도 Note 심각도의 진단으로 기록합니다(코드 생성 IR, XXII.1.1 입력). 이 결정은 허용 경로 밖인 `docs/adr/` 대신 모듈
  문서와 이 보고서에 기록했습니다.

## 남은 위험 / 한계

- 검사는 주어진 variant 층 하나에 대해서만 합니다. 모든 variant 조합 검사는 호출자가 조합별로 반복해야 합니다.
- node 수가 큰 View에서 profile 수 × node 수만큼 진단이 생깁니다(Native 포함). 대량 View의 성능 예산은 아직 측정하지 않았습니다.

## 후속 수정(리뷰 반영): 제외 판정 비용

리뷰(TP-5)에서 `collect_usages`가 node마다 `is_excluded`로 root까지 parent를 거슬러 올라가 깊은 선형 트리에서 O(N^2)라는 점이
지적되어 `Fix` commit으로 고쳤습니다. 이 함수는 upgrade plan(IX.10.9)도 모든 View에 대해 씁니다.

- `ResolvedView::excluded_among`(IX.10.7 `overrides.rs`)을 추가했습니다. 판정한 node의 결과를 재사용해 `parent_of`를 node마다 최대
  한 번만 부르고(O(N log N)), 순환·목록에 없는 parent에서도 node마다 `is_excluded`(`max_depth` = node 수)를 부른 것과 같은
  결과입니다. `collect_usages`는 이 함수로 제외 집합을 한 번에 구합니다.

| 명령 | 결과 |
|---|---|
| 수정 전(같은 판정을 node마다 `is_excluded`로 하는 구현) `cargo test -p stellar-target-profile overrides::` | 1 failed(`excluded_among_matches_is_excluded_with_linear_parent_lookups`: 2,000단계 선형 트리에서 parent 조회 1,000,000회) |
| 수정 후 `cargo test -p stellar-target-profile` | 42 passed, 0 failed |

| 검사 | 결과 |
|---|---|
| 2,000단계 선형 트리에서 중간 node 제외 시 그 node와 하위만 제외, parent 조회 ≤ node 수; 제외 없는 순환·제외 node가 있는 순환·목록에 없는 제외 조상·목록에 없는 조상에서 node별 `is_excluded`와 같은 결과 (`excluded_among_matches_is_excluded_with_linear_parent_lookups`) | 통과 |
| 기존 multi-profile 진단 fixture(제외 node 하위 미검사 포함) | 통과(변경 없음) |

## 상태 확인(리뷰 반영, 2026-11-14)

- 리뷰(TP-2)는 이 행의 입력도 지역 모델이라는 점을 지적했습니다. 이 행의 작업명과 검증 열은 한 View의 다중 profile 검사 API와 기대 진단 fixture이고, 실제 문서·프로젝트
  저장이나 command를 요구하지 않습니다. 검증 열은 fixture로 충족되므로 `[x]`를 유지합니다.
- 사실: 입력 View는 지역 `ViewModel`이고, 문서 tree → `ViewModel` 변환과 `PropertyCapabilityMap` 구현이 아직 없습니다. 이 연결은 IX.10.3(`stellar.yaml` 연결)과 IX.10.10(override 저장 형식·command·실제 host)의 남은 범위로 두었고,
  두 행은 `[~]`입니다.
