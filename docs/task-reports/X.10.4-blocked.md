# 차단 보고서: X.10.4 Text intrinsic 크기·baseline의 layout intrinsic hook 연결

## WBS

- WBS: X.10.4
- 작업명: Text intrinsic width/height와 baseline을 layout intrinsic hook에 연결
- 상태: `[!]`
- 선행: X.10.2(이 lane `[x]`), X.6.4(`[ ]`, 미착수)

## 실패 선행조건

- **X.6.4 미완료**: "content intrinsic measurement trait를 Shape, Text, Image, ComponentInstance별로 정의"가
  기준 commit(1c461c2)과 이 lane 어디에서도 `[x]`가 아닙니다. 연결 대상인 layout intrinsic hook(trait)이 아직
  없으므로 연결할 수 없습니다. 명시 행 ID 선행은 엄격 적용 대상이라 완화하지 않았습니다.
- **crate 경계**: `tools/xtask/src/crate_graph.rs` 정책상 `stellar-layout`과 `stellar-text`는 모두
  `stellar-core-types`에만 의존할 수 있어 두 crate가 서로를 참조할 수 없습니다. 연결 위치(상위 crate 또는 허용
  간선 추가)는 X.6.4 trait 설계와 함께 정해야 합니다. 이 lane은 crate graph를 수정하지 않습니다.

## 재현 명령

- `grep -n "^| X\.6\.4 " docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` → 상태 `[ ]`.
- `cargo run -q -p xtask -- validate-crate-graph`(현재 정책: layout·text → core-types만).

## 필요한 결정과 환경

- X.6.4에서 intrinsic measurement trait의 위치(layout crate)와 Text 구현이 놓일 crate(예: text를 layout에 연결하는
  상위 crate, 또는 `stellar-layout → stellar-text` 간선 허용)를 정하는 결정이 필요합니다.
- 실행 환경은 `DEV-MAC`이면 충분합니다.

## 변경되지 않은 파일

- 코드 변경 없음. text 쪽 재료는 준비되어 있습니다: `TextLayout::width`·`height`·`content_height`,
  `first_baseline`·`last_baseline`(X.10.2), 레이아웃 `LayoutUnit`과 같은 1/64 px `TextUnit`(X.10.1).

## 재개 조건

- X.6.4가 `[x]`이고 연결 crate 경계가 crate graph 정책에 반영되면, Text intrinsic 측정(너비 제약 없음 = max-content,
  너비 제약 = 줄바꿈 높이)과 첫 줄 baseline을 hook에 연결하고 Stack baseline fixture(X.3.6과 공유)를 통과시킵니다.
