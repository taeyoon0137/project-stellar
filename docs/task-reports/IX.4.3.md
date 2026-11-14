# 작업 보고서: IX.4.3 DrillingPlan 계산

- WBS: IX.4.3
- 작업명: 내부 property를 상위로 노출할 때 필요한 중간 property와 binding chain을 계산하는 `DrillingPlan`(적용 안 함)
- 상태: 완료 (`[x]`)
- 선행: IX.4.1(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/drilling_plan.rs`(신규) — `DrillingRequest`, `DrillingPlan`(`compute`), `NewProperty`,
  `PlannedLink`, `PlannedProperty`, `ReusedLink`, `DrillingPlanError`.
- `crates/document/src/component/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.4.3.md`(신규), 계획서(IX.4.3 `[ ]`→`[x]`).

## 구현 요약

- **입력**: 최상위 컴포넌트 `top`, `top` 정의 tree부터의 nested instance path(IX.4.2 `RequiresDrilling` chain에서 scene
  인스턴스를 뺀 부분), 가장 안쪽 컴포넌트의 공개 property. path는 IX.4.1 `resolve_path` 규칙(경계별 범위·인스턴스
  종류·컴포넌트 해석)으로 해석합니다.
- **계산**(안쪽 → 바깥): 경계마다 owner가 소유한 live link가 그 자리에 있으면 재사용하고(`reused`) link source로
  노출이 이어집니다. 없으면 owner에 새 공개 property(이름·타입·default = 현재 노출 중인 property의 것)와 새 property
  → 자리 link를 계획합니다. 정적 override가 그 자리를 채우면 `StaticOverride`로 거부합니다.
- **결과**: 경계 오름차순(바깥 → 안쪽) `new_properties`·`links`·`reused`, 노출 후 `top` property `exposed`
  (`New { boundary: 0 }` 또는 기존 property). 모든 경계가 연결돼 있으면 `is_noop`.
- library·drilling 모음은 읽기만 합니다(적용은 IX.4.5).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 330 passed, 통합 7+1+1, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 1·2·10단계 nesting: 경계마다 새 property(컴포넌트·이름·타입·default)와 link(owner·인스턴스·source·target) 정확, exposed = top 새 property, 계산 전후 library·모음 bytes 동일, 재계산 결과 동일(`plans_for_one_two_and_ten_level_nesting_are_exact`) | 통과 |
| 기존 link 재사용 시 source 이름·Optional 타입·default로 상위 property 계획, 전부 연결 시 no-op·exposed 기존 property(`existing_links_are_reused_and_exposure_follows_their_source`) | 통과 |
| private·경계 건너뛰기·빈 path·없는 property·정적 override 자리 거부(`invalid_requests_are_rejected`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 이름 충돌 suffix와 preview(컴포넌트 경로 포함)는 IX.4.4 범위라 여기서는 기준 이름만 계산합니다.

## 남은 위험 / 한계

- 정적 override가 있는 자리는 자동으로 reset하지 않고 거부합니다. UI가 reset 후 다시 계획해야 합니다.

## 후속 수정 (리뷰 반영)

`Fix: private source link를 재사용하는 drilling 계획 거부` 커밋에서 고쳤습니다.

- **문제**: 경계 자리에 이미 있는 live link를 재사용할 때 그 link의 source가 공개인지 확인하지 않았습니다. link
  source는 owner 내부 property라 private이어도 저장 규칙(IX.4.1)상 허용됩니다. 그래서 C1의 private property가
  I1.label을 채우는 상태에서 C0으로 노출을 요청하면 `compute`가 private property를 target으로 삼는 바깥 link를
  계획해 `Ok`를 돌려주고, 적용(IX.4.5)은 항상 `PrivateProperty`로 실패해 롤백됐습니다. "private property 거부"는
  가장 안쪽 property에만 적용되고 있었습니다.
- **변경**: 재사용할 link의 source가 owner 선언 목록에 있고 공개인지 확인하고, private이면
  `DrillingError::PrivateProperty { component: owner, property: source }`로 거부합니다. top 경계(k = 0)에서도
  거부합니다. 노출 결과는 top 인스턴스가 override할 property여야 하는데 private property는 인스턴스 override 대상이
  아니기 때문입니다. 그 자리는 이미 link가 채우고 있어 새 공개 property를 따로 연결하는 계획(값 원천 두 개)도 만들 수
  없으므로 거부가 맞습니다.
- **검증**: `reused_link_with_private_source_is_rejected`를 먼저 추가해 수정 전 `Ok` 계획이 나오는 것(실패)을
  확인했습니다. 수정 후 바깥 경계와 top 경계 둘 다 `PrivateProperty`가 나오고, 계산 전후 library·모음 bytes가
  같습니다.

| 명령(후속 수정) | 결과 |
|---|---|
| `cargo test -p stellar-document --lib reused_link_with_private_source`(수정 전) | 1 failed(`unwrap_err()`가 `Ok` 계획을 받음) |
| `cargo test -p stellar-document` | unit 352 passed, 통합 7+1+1, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |
