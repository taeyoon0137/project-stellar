# 작업 보고서: X.5.3 constraint graph 구축과 분류

- WBS: X.5.3
- 작업명: constraint graph를 구축하고 cycle·underdetermined·overdetermined 분류
- 상태: 완료 (`[x]`)
- 선행: X.5.2(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/constraint.rs` — `ConstraintClass`, `AxisGraph`, `edge_coefficients`, rank 계산, `build_axis_graph`(분류·의존 순서·연결 요소), graph fixture 테스트, 모듈 문서.
- `docs/task-reports/X.5.3.md`(신규), 계획서(X.5.3 `[ ]`→`[x]`).

## 구현 요약

- **graph**: 축마다 형제 = node, "자식 → target 형제" = 간선(중복 간선 1개로).
- **미지수·등식**: 형제 한 축의 미지수는 시작 위치(항상)와 크기(`Fill`일 때). constraint 하나 = 등식 하나이며
  계수(2배 단위)는 시작 쪽 (2, 0), 끝 쪽 (2, 2), 가운데 (2, 1), baseline은 콘텐츠 baseline이 있으면 (2, 0),
  없으면 bottom fallback (2, 2). 크기가 정해진 형제는 시작 계수만 봅니다.
- **분류**: 순환에 속함 → `Cycle`, 등식 수 > rank(중복·모순) → `Overdetermined`, rank < 미지수 →
  `Underdetermined`, 그 외 `Determined`. 순환에 의존할 뿐 순환에 속하지 않은 형제는 자기 등식으로 분류합니다.
- **부산물**: 의존 순서(Kahn, 준비된 형제 중 앞 visual order부터, 순환·순환 의존 형제 제외)와 무방향 연결 요소
  (union-find, visual order). 순서는 solver(X.5.4), 연결 요소는 영향 범위 제한(X.5.5)이 사용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | 99 unit passed + 2 doctest passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| determined/underdetermined fixture 5형제: 고정+Left, Fill+sibling Left+Right, Fill+Left, 무제약, Fill+Left+CenterX → D/D/U/U/D, 순서·연결 요소(`determined_and_underdetermined_graph_fixture`) | 통과 |
| overdetermined fixture 5형제: 고정+양끝, Fill+평행 2개, Fill+3개, 고정+Top+콘텐츠 baseline → O×4, Fill+Top+baseline fallback → D(`overdetermined_graph_fixture`) | 통과 |
| cycle fixture: 0↔1 순환, 자기 참조 2, 순환 의존 3(D), 독립 4 → C/C/C/D/D, 순서 [4], 연결 요소(`cycle_graph_fixture`) | 통과 |
| 의존 순서가 target 먼저(1, 2, 0)(`dependency_order_puts_targets_first`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. (분류 결과의 오류·진단 처리와 solver는 X.5.4에서 엔진에 연결합니다.)

## 남은 위험 / 한계

- 순환 판정은 정렬되지 않은 형제마다 DFS를 수행합니다(형제 수 n에 대해 최악 O(n²)). 형제 수가 매우 큰 Constraint
  container에서는 SCC 알고리즘으로 바꿀 수 있습니다.
