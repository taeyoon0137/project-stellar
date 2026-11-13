# 작업 보고서: XVI.3.2 가시 node 2만·컴포넌트 인스턴스 5만·중첩 64 fixture

- WBS: XVI.3.2
- 작업명: 2만 visible node, 5만 component instance, 중첩 64 fixture를 각각 생성
- 상태: 완료 (`[x]`)
- 선행: IX.2.2 (`[x]`), VI.10.1 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `benchmarks/fixtures/src/limits.rs`(신규) — `visible_nodes`, `count_visible_in_viewport`,
  `component_instances`, `nesting_chain`, `validate_component_fixture`, `deep_document`.
- `benchmarks/fixtures/src/error.rs`(수정) — `Component`, `Expansion` 오류 추가.
- `benchmarks/fixtures/src/cli.rs`(수정) — `visible-20k`, `components-50k`, `nesting-64`, `deep-64` 추가.
- `benchmarks/fixtures/src/lib.rs`(수정) — `limits` 모듈 등록.
- `benchmarks/fixtures/tests/limit_fixtures.rs`(신규) — 세 fixture 검증과 음성 대조.
- `docs/task-reports/XVI.3.2.md`(신규), 계획서(XVI.3.2 `[ ]`→`[x]`).

## 구현 요약

- **가시 node 2만**(`visible_nodes(seed, 20_000)`): 문서 fixture입니다. 1920×1080 viewport 크기
  View root 아래 20 px 격자에 card Frame(배경 Rectangle·Ellipse·Image·Path leaf)을 놓아 root 포함
  정확히 20,000 node가 유효 가시(자신·모든 조상 visible, world bounds가 viewport와 겹침)입니다.
  culling 대상으로 숨김 section(visible=false, 1,001 node)과 viewport 밖 section(2,001 node)을 함께
  둡니다. 가시 판정이 텍스트 레이아웃에 좌우되지 않게 Text·Fragment는 쓰지 않습니다. 문서
  validator(VI.10.1)를 통과하고 미해결 참조가 없습니다. `count_visible_in_viewport`는 영속
  레코드로 부모→자식 index를 만들어 world 변환(Frame transform 누적)과 bounds를 독립 계산하는
  검산기입니다.
- **컴포넌트 인스턴스 5만**(`component_instances(seed, 50_000)`): 영속 문서 레코드에 아직
  ComponentInstance payload variant가 없어(VI.10), IX.2 scene 표현(`NodeColumns` + `ScenePayloads` +
  `ComponentLibrary`)으로 만듭니다. 디자인 시스템형 library(Icon, Icon을 중첩한 Button, Button을
  중첩한 Card, Icon을 중첩한 ListRow; 정의 node 17개)와 root ← section(500 인스턴스씩) ←
  인스턴스 scene입니다. 인스턴스 절반가량이 선언된 Property override(String·Boolean)를 가집니다.
  정의 node payload는 실제 canonical 인코딩(`encode_payload`)입니다.
- **validator**(`validate_component_fixture`): library canonical round-trip, 모든 인스턴스의
  `InstanceExpansion::new`(payload 해석·정의 해결)와 `walk`(미해결·재귀·중첩 상한), override가 선언된
  Property에만 있는지, 가상 node가 정의 node를 참조만 하는지(서로 다른 저장소 주소 수 = 정의 node
  수)를 검사합니다. library 등록 시 정의 검증(IX.1.1)도 거칩니다.
- **중첩 64**: `nesting_chain(seed, 64)`은 `C0 ⊃ … ⊃ C63` 정의 chain의 scene 인스턴스 하나로, scene
  인스턴스를 포함한 인스턴스 chain 길이가 정확히 64(`MAX_INSTANCE_NESTING`)입니다.
  `deep_document(seed, 64)`는 root 아래 Frame 64단(각 단에 Shape·Text 형제)으로 가장 깊은 node의
  depth가 정확히 64(`MAX_NODE_DEPTH`)인 문서입니다. 둘 다 음성 대조용으로 64 초과를 만들 수 있고,
  각 validator가 거부합니다.
- 모든 fixture는 seed로 재현되며 content hash(문서는 chunk model, 컴포넌트는 library와 scene의
  canonical bytes)를 함께 냅니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bench-fixtures` | 완료 |
| `cargo test -p stellar-bench-fixtures` | unit 10 passed, `limit_fixtures` 3 passed, `synthetic_scale` 2 passed·2 ignored, 0 failed |
| `cargo clippy -p stellar-bench-fixtures --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-bench-fixtures -- --check` | 통과 |
| `git diff --check` | 통과 |
| `cargo run -q -p stellar-bench-fixtures --release -- visible-20k` | 23,002 node, visible 20,000, hidden 1,001, offscreen 2,001, 120 ms |
| `cargo run -q -p stellar-bench-fixtures --release -- components-50k` | scene 50,101 node, 인스턴스 50,000, 정의 node 17, 가상 node 389,870, 최대 chain 3, 108 ms |
| `cargo run -q -p stellar-bench-fixtures --release -- nesting-64` | 정의 node 191, 최대 chain 64 |
| `cargo run -q -p stellar-bench-fixtures --release -- deep-64` | 193 node |

| 검사 | 결과 |
|---|---|
| 가시 2만: validator 통과·미해결 0, 독립 검산 가시 수 20,000, 전체 = 가시 + 숨김 + viewport 밖, seed 재현·다른 seed도 가시 20,000, 1,234 같은 나머지 목표 정확, 0·용량 초과 거부(`visible_20k_fixture_has_exactly_20k_visible_nodes`) | 통과 |
| 인스턴스 5만: 모든 인스턴스 expansion 통과, 정의 node 복제 없음(서로 다른 주소 = 정의 node 수), override 존재, scene node 수 정확, seed 재현(`component_50k_fixture_expands_every_instance_without_copies`) | 통과 |
| 중첩 64: 인스턴스 chain 정확히 64 통과, 65는 `NestingTooDeep { depth: 65 }`, 문서 depth 정확히 64 validator 통과, 65는 `DepthExceeded { depth: 65 }`(`nesting_64_fixtures_sit_exactly_at_the_limit`) | 통과 |

## 차이(작업 지시와 다른 점)

- 5만 인스턴스·중첩 64 컴포넌트 fixture는 문서 chunk model이 아니라 IX.2 scene 표현으로 만듭니다.
  영속 문서 레코드(VI.10)에 ComponentInstance payload variant가 아직 없기 때문입니다. 따라서
  "validator"는 문서 validator 대신 library 등록 검증과 expansion 검증(IX.1.1·IX.2.2)입니다.
- 중첩 64는 컴포넌트 인스턴스 chain과 문서 node depth chain 두 가지로 만들었습니다(아키텍처 §5.3
  "컴포넌트 중첩 깊이 64"와 문서 depth ≤ 64 정책이 같은 값).

## 남은 위험 / 한계

- 가시 판정은 이 fixture의 geometry(Frame·Image·Rectangle·Ellipse·Polygon·Line·Path bounds,
  Frame transform 누적)로 계산합니다. Text bounds, modifier(Transform·Shadow 등)의 visual bounds
  확장, clip은 반영하지 않으므로 이 fixture에는 그런 요소를 넣지 않았습니다. 렌더러의 실제 culling
  결과와 대조하는 일은 XI 단계 benchmark 몫입니다.
- 문서 레코드가 ComponentInstance를 표현하게 되면 인스턴스 fixture를 문서 chunk model로도 만들어야
  합니다.
