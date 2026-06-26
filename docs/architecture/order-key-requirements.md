# OrderKey 요구사항 (III.5.1)

자식 순서 키(`OrderKey`)의 요구사항을 정의합니다. 본 문서는 구현 이전의 **명세**이며,
III.5.1 시점에는 대응 fixture가 미구현 상태로 실패합니다. 알고리즘 확정과 구현은 III.5.2,
재균형은 III.5.3에서 다룹니다(기술 기준: 기술 아키텍처 12.7 자식 순서).

## 목적

자식 순서는 배열 인덱스 자체를 정체성으로 사용하지 않습니다. 동시 브랜치 삽입과 안정적
Merge가 가능하도록, 각 자식은 부모 안에서 **결정론적 `OrderKey`**를 가집니다. 순서는
`OrderKey`의 비교로 결정합니다.

## 요구사항

### R1. 비교 가능한 byte sequence

- `OrderKey`는 비교 가능한 byte sequence입니다. 정렬 순서는 byte **사전식(lexicographic)**
  비교와 일치합니다.
- 키는 0~1 사이의 base-256 소수로 해석됩니다(각 byte가 소수 자릿수). 정규형은 **`0x00`으로
  끝나지 않으며 비어 있지 않습니다**(후행 0은 같은 값의 다른 표현이므로 금지).

### R2. 앞·뒤·사이 삽입

- 기존 첫 자식 앞에 삽입: 결과 키 < 첫 자식 키.
- 기존 마지막 자식 뒤에 삽입: 결과 키 > 마지막 자식 키.
- 인접한 두 자식 `a < b` 사이 삽입: `a < 결과 < b`.
- 빈 부모에 첫 삽입: 유효한 단일 키를 생성.

### R3. 서로 다른 브랜치 동시 삽입

- 좌/우 이웃이 같은 두 브랜치가 독립적으로 키를 생성해도 각자 이웃 사이의 유효한 키를
  만듭니다(중앙 조정 불필요).
- 두 브랜치가 **동일한** `OrderKey`를 만들 수 있습니다. 이때 최종 순서는 자식의 안정
  식별자(`StableId`)로 tie-break 합니다(키 동일 → `StableId` 오름차순). 따라서 전체 순서는
  항상 전순서이며 결정론적입니다.

### R4. 결정론(난수 미사용)

- 키 생성에 **난수를 사용하지 않습니다**. 같은 (좌, 우) 이웃 입력은 항상 같은 키를 만듭니다.
- 같은 자식 집합은 항상 같은 정렬 결과를 만듭니다(양 플랫폼 동일).

### R5. 키 길이 상한과 재균형

- 반복 삽입으로 키가 길어질 수 있습니다. 키 길이가 임계값을 넘으면 **부모 단위 재균형**이
  필요함을 신호합니다(`needs_rebalance`).
- 재균형(III.5.3)은 자식의 시각 순서(따라서 `StableId` 순서)를 보존하며, 같은 입력에 대해
  두 번 실행해도 동일한 결과를 만듭니다.

## 검증 fixture (III.5.1 시점: 미구현으로 실패)

`crates/core-types/src/order_key.rs`의 테스트가 위 요구사항을 인코딩합니다. III.5.1에서는
`OrderKey` 생성 API가 미구현(`unimplemented!`)이라 호출 시 실패합니다.

- R2: `insert_front_back_between`
- R3: `concurrent_branch_inserts_independent`, `equal_key_tie_break_by_stable_id`
- R4: `between_is_deterministic`
- R1: `order_matches_byte_lexicographic`, `from_bytes_rejects_non_canonical`
- R5: `key_length_cap_signals_rebalance`

## 비고

- 구체 알고리즘(결정론적 fractional indexing)과 근거는 III.5.2의 ADR에서 확정합니다.
- 본 문서는 III.5.2 구현 이후에도 요구사항 기준으로 유지합니다.
