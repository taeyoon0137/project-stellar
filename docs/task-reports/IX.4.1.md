# 작업 보고서: IX.4.1 경계 넘는 binding의 ComponentProperty chain 필수화

- WBS: IX.4.1
- 작업명: component boundary를 넘는 binding이 각 경계의 ComponentProperty StableId chain을 필수로 갖게 함
- 상태: 완료 (`[x]`)
- 선행: IX.1.2(`[x]`), VIII.6.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/drilling.rs`(신규) — `DrillingBinding`, `DrillingBindingId`, `DrillingSlot`,
  `DrillingSet`(canonical codec), `DrillingError`, `resolve_path`, `MAX_DRILLING_DEPTH`, 테스트 fixture(`depth`단계
  nested component chain).
- `crates/document/src/component/definition.rs` — 테스트 전용 `ComponentEntry::definition_mut`·`tree_mut`,
  `DefinitionTree::set_payload`(fixture 조작용, `#[cfg(test)]`).
- `crates/document/src/component/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.4.1.md`(신규), 계획서(IX.4.1 `[ ]`→`[x]`).

## 구현 요약

- **모델**: `DrillingBinding { owner(상위 ComponentDefinition), source(owner property), path(nested instance chain,
  바깥→안쪽), target(가장 안쪽 컴포넌트 property), chain(경계별 ComponentPropertyId) }`. `chain[k]`는 `path[k]`가
  참조하는 컴포넌트의 property이고 마지막이 target입니다.
- **chain 필수(구조 규칙, 생성자·디코더 공통)**: path가 비면 `NoBoundary`, chain이 경계 수보다 짧으면
  `MissingChain { boundaries, properties }`, 길면 `ExtraChain`, 마지막이 target이 아니면 `ChainTargetMismatch`.
  path 반복 인스턴스·상한(`MAX_DRILLING_DEPTH` = 인스턴스 중첩 상한 64) 초과도 거부하며, 디코더는 길이 프리픽스를
  할당 전에 상한과 비교합니다.
- **의미 검증(`validate`)**: owner·source 존재, `path[0]`은 owner 정의 tree, `path[k]`는 `path[k-1]` 컴포넌트 정의
  tree의 ComponentInstance(경계 건너뛰기는 `InstanceOutOfScope`), `chain[k]`는 그 컴포넌트의 **공개** property,
  source → `chain[0]` → … 타입 대입 가능, 중간 경계 `k ≥ 1`마다 `path[k-1]` 컴포넌트가 소유한 link 존재
  (`MissingIntermediateLink`), 첫 경계 인스턴스의 같은 property 정적 override와 충돌 금지.
- **모음(`DrillingSet`)**: 첫 경계 값 자리(owner, `path[0]`, `chain[0]`)마다 binding 하나. id 오름차순 canonical
  bytes, 디코딩은 구조 규칙·정렬·자리 유일성을 검사하고 library 의미 검증은 `validate`가 합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 320 passed, 통합 7+1+1, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 경계 2개 binding의 chain 0개·1개, target 불일치, 경계 0개, 긴 chain 거부 + chain 없는 저장 bytes 디코딩 거부(`binding_without_boundary_chain_is_rejected`) | 통과 |
| chain 완비 + 중간 link 존재 시 통과, link 없음·중간 property가 다른 컴포넌트 것이면 거부, round-trip(`full_chain_validates_with_intermediate_links`) | 통과 |
| 경계 건너뛰기·비인스턴스·private·타입 불일치·자리 중복·id 중복·정적 override 충돌 거부(`semantic_rules_reject_scope_visibility_type_and_duplicates`) | 통과 |
| 정렬 위반·trailing·절단·과대 길이·반복 인스턴스 디코딩 거부(`decode_rejects_unsorted_trailing_and_deep_input`) | 통과 |

## 차이(작업 지시와 다른 점)

- crate 그래프상 document는 `stellar-bindings`에 의존할 수 없어(`document → expressions`만 허용), 경계를 넘는
  binding을 document 계층의 `DrillingBinding`으로 정의했습니다. 값 원천은 owner 컴포넌트의 property이며 경계 안쪽의
  일반 표현식 binding은 `stellar-bindings` `ValueBinding`(VIII.6.1)의 몫입니다.
- `DrillingSet`은 `ComponentLibrary` bytes와 별도 resource입니다. library 저장 형식(semantic-diff·storage가 사용)을
  바꾸지 않기 위해서입니다. chunk 저장 연결은 후속입니다.

## 남은 위험 / 한계

- `stellar-bindings` `ValueBinding`의 target이 nested instance 경계를 넘는지 판정하는 연결은 document를 보지 못하는
  bindings crate 구조상 상위 계층(runtime 등)이 합니다.
