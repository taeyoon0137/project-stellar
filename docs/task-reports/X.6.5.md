# 작업 보고서: X.6.5 HugContent cycle 탐지

- WBS: X.6.5
- 작업명: HugContent cycle을 탐지하고 무한 상호 의존을 diagnostic으로 반환
- 상태: 완료 (`[x]`)
- 선행: X.6.4(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/layout/src/output.rs` — `LayoutDiagnostic::HugContentCycle { ancestor, node, axis }`, 표시 문구, 결과 canonical
  bytes 진단 tag 2.
- `crates/layout/src/engine.rs` — 배치(top-down) 중 축별 콘텐츠 크기 조상 전파(`content_source`, `child_link`)와 순환
  검사, 테스트.
- `crates/layout/src/grid.rs` — item 영역 track 의존성(`AreaDependence`, 누적 개수로 O(1)) 기록.
- `crates/layout/src/grid_tracks.rs` — `Breadth::is_content`, `Track::is_flexible` crate 공개.
- `crates/layout/src/free.rs` — `PreparedAnchors::contributes_to_extent`(측정·순환 검사 공통 규칙).
- `crates/layout/src/sizing.rs` — Hug 부모의 Percentage 자식 정책 문서에 순환 진단 추가.
- `crates/layout/tests/fixtures/layout_corpus_v1.golden` — 진단 수·결과 digest 갱신(아래).
- `docs/task-reports/X.6.5.md`(신규), 계획서(X.6.5 `[ ]`→`[x]`).

## 구현 요약

- **순환 정의**: node N의 한 축 sizing이 `Percentage`이고, N의 containing block 크기가 N 자신의 측정 기여로 정해지는
  콘텐츠 크기 조상 A에 의존하면 순환입니다(N = p% × A, A = N 기여 + … → p ≠ 1이면 수렴하지 않는 상호 의존). 엔진은
  기존 규칙(X.6.1)대로 N이 콘텐츠 크기로 기여하게 해 계산을 끝내고 `HugContentCycle { ancestor: A, node: N, axis }`를
  남깁니다. 레이아웃 결과(rect)는 바뀌지 않습니다.
- **전파 규칙**(배치 단계 top-down, 측정 cache와 무관):
  - node 자신의 콘텐츠 크기 조상: `HugContent` 축 → 자신(비율로 다른 `Fixed` 축에서 정해지면 제외), `Fill`·
    `Percentage` 축 → containing block의 조상을 그대로(사슬 연장), `Fixed` 축 → 없음(사슬 끊김).
  - 자식에게: Free는 부모 extent에 기여하는 자식(끝 anchor·백분율 시작 anchor 없음)만, Stack·Overlay·Constraint는
    모든 자식에게 전달합니다. Grid item은 걸친 track이 콘텐츠 기반(MinContent·MaxContent 경계)이면 Grid 자신, fr이면
    Grid 축의 콘텐츠 크기 조상(Hug grid면 Grid)을 받습니다. 고정 track은 끊습니다.
- **순환이 아닌 경우**: `Fill`(부모 크기를 그대로 채우는 고정점), definite 부모의 Percentage, 부모 extent에 기여하지
  않는 자식, `Fixed` 중간 node 아래.
- 진단은 배치 pre-order로 node마다 축당 최대 하나입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-layout` | 종료 코드 0 |
| `cargo test -p stellar-layout` | unit 135 + `layout_corpus` 2 + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-layout --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-layout -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| parent(Hug)↔child(50%) fixture → `HugContentCycle(parent, child, 가로)` 한 건, parent 40·child 20, 진단 bytes 왕복(`parent_child_hug_cycle_fixture_is_reported`) | 통과 |
| definite 부모·Hug 부모의 Fill·끝 anchor 자식·Fixed 중간 node는 진단 없음, Fill 중간 node는 Hug root까지 사슬, Hug 중간 node는 가장 가까운 조상 보고(`non_cycles_are_not_reported`) | 통과 |
| Hug Stack cross 50% → 세로 순환, 고정 크기 Grid의 MaxContent 열 50% item → Grid 순환, 고정 열·definite fr 열은 없음, Hug grid의 fr 열 → 순환(`stack_and_grid_hug_cycles`) | 통과 |
| X.11.1 corpus: 변경 전후 250개 fixture의 box bytes를 임시 scratch 테스트(커밋하지 않음)로 비교 → box 변경 0, 진단이 바뀐 fixture 55개(모두 `HugContentCycle` 추가) | rect 불변 확인 |

- **golden 갱신 원인**: corpus 생성기가 Hug root 아래 `Percentage` 자식을 만들기 때문에 새 진단이 결과 bytes에
  들어갑니다. 입력 digest(`inputs=`)와 box 수, constraint 행은 그대로이고, free·stack·grid·overlay 행의 진단 수
  (0 → 9·39·60·4)와 결과 digest만 바뀌었습니다. 갱신은 실패한 golden 비교가 출력한 실제 값으로 했습니다.

## 차이(작업 지시와 다른 점)

- "parent↔child cycle fixture 실패"를 "순환 fixture가 진단으로 보고된다"로 해석했습니다. 지시가 "diagnostic으로
  반환"이고, X.6.1이 Hug 부모 아래 Percentage를 거부하지 않는 정책(콘텐츠 기여로 계산 완료)을 고정했기 때문에 오류로
  바꾸지 않았습니다. 엄격 검사가 필요하면 호출자가 `HugContentCycle` 진단을 실패로 취급하면 됩니다.
- 순환 판단은 선언된 sizing 기준입니다. 부모가 강제한 크기(Stack Stretch·flex 분배)는 고려하지 않습니다.

## 남은 위험 / 한계

- Stack Stretch로 cross 크기가 강제된 Hug 자식 아래 사슬처럼 forced 크기가 개입하면 보수적으로 보고하거나 놓칠 수
  있습니다(rect 계산에는 영향 없음).
- min-content 측정(X.4.5) 등 측정 경로가 늘어나도 진단은 배치 단계 규칙으로만 정해집니다.
