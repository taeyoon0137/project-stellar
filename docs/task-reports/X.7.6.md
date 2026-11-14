# 작업 보고서: X.7.6 전체 layout fallback 제한과 debug assertion·metric

- WBS: X.7.6
- 작업명: 전체 layout fallback 경로를 테스트·진단용으로만 제공하고, 일반 property 변경에서 호출되면 debug assertion과 metric
- 상태: 완료 (`[x]`)
- 선행: X.7.3(`[x]`, 이 lane `51dc6b1`) — 명시 행 선행 충족, 완화 없음

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증; debug·release 두 build에서 테스트).

## 변경 파일 목록

- `crates/layout/src/incremental.rs` — `FullLayoutReason`(Initial, MeasurerChanged, Recovery, Diagnostic, UnexpectedFallback),
  `LayoutMetrics`(updates, 이유별 전체 레이아웃 수), `UpdateStats::full_layout_reason`, `IncrementalLayout::metrics`,
  `IncrementalLayout::full_layout_for_diagnostics`, 일반 edit 중 대체 경로의 metric + debug assertion, 테스트 전용
  `forget_layout`(`#[cfg(test)]`), 테스트.
- `docs/task-reports/X.7.6.md`(신규), 계획서(X.7.6 `[ ]`→`[x]`).

## 구현 요약

- **전체 레이아웃 경로 제한**: 재사용 없이 root부터 배치하는 `relayout_all`은 이유를 받아서만 실행하며, 이유는 세션 시작,
  측정기 revision 변경, 계산 실패 뒤 복구, 테스트·진단용 명시 호출(`full_layout_for_diagnostics`) 네 가지입니다. 일반 edit
  처리(`apply` → 증분 경로)는 이 네 가지 외에는 전체 레이아웃을 쓰지 않습니다.
- **불변식 위반 대체**: 증분 경로가 표시 node의 이전 배치를 찾지 못하면(정상 경로에서는 생기지 않음) `UnexpectedFallback`으로
  metric을 **먼저** 남기고 전체 레이아웃으로 결과를 맞춘 뒤, debug build(`cfg!(debug_assertions)`)에서는 panic으로 실패합니다.
  release는 결과 정확성을 위해 계속합니다.
- **metric**: `LayoutMetrics`는 세션 누적 edit 수와 이유별 전체 레이아웃 수를 셉니다(포화 증가). edit 결과
  `UpdateStats::full_layout_reason`에도 이유가 남습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | unit 171 + `layout_corpus` 2 + doctest 2 passed, 0 failed |
| `cargo test --release -p stellar-layout incremental::` | 14 passed(release 전용 `unexpected_fallback_is_counted_in_release_builds` 포함) |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy --release -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **단일 edit fixture에서 full layout count 0**: 모드별 문서의 모든 node × 7종 단일 edit(너비, padding, 표시 여부, 위치, transform, max height, flex; 검증 거부 제외 100회 초과)마다 `full_layouts` 0·결과 = 전체 계산, 세션 metric 전체 레이아웃 1(시작)·unexpected 0, 무작위 edit corpus 20개 × 16 step도 unexpected 0·전체 레이아웃 = 1 + 복구(`single_edit_fixtures_never_use_full_layout`) | 통과 |
| 진단용 전체 레이아웃 = 증분 결과, 측정기 변경·계산 실패(수치 범위 초과) 뒤 복구까지 이유별 metric `(initial, diagnostic, measurer_changed, recovery, unexpected) = (1, 1, 1, 1, 0)`(`diagnostic_and_recovery_full_layouts_are_counted_by_reason`) | 통과 |
| 이전 배치를 잊게 한 세션: debug build는 panic(`unexpected_fallback_asserts_in_debug_builds`), release는 metric 1·결과 = 전체 계산(`unexpected_fallback_is_counted_in_release_builds`), 두 build 모두 panic 전에 metric 기록(`unexpected_fallback_metric_is_recorded_before_the_assertion`) | 통과 |

## 차이(작업 지시와 다른 점)

- `debug_assert!(false, ..)` 대신 `if cfg!(debug_assertions) { panic!(..) }`로 썼습니다(같은 의미, clippy
  `assertions_on_constants` 회피). metric을 assertion 전에 남겨 debug에서도 위반 횟수가 보존됩니다.

## 남은 위험 / 한계

- metric은 세션 안 누적값입니다. 앱 텔레메트리·진단 화면으로 내보내는 연결은 상위 crate(runtime) 몫입니다.
- 측정기 revision 변경은 정상 사유의 전체 레이아웃이라 20만 node 문서에서 1초 이상 걸릴 수 있습니다(X.7.4 측정). 폰트 로드
  범위만 다시 측정하는 세분화는 후속 과제입니다.
