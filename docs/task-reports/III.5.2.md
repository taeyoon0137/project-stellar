# 작업 보고서: III.5.2 OrderKey 알고리즘 구현(green)

- WBS: III.5.2
- 작업명: 결정론적 fractional indexing `OrderKey` 구현(난수 미사용), 100만 회 property test
- 상태: 완료 (`[x]`)
- 선행: III.5.1(요구사항·fixture), ADR 승인(ADR-0003)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `docs/adr/ADR-0003-order-key.md` (신규: OrderKey 알고리즘 결정)
- `crates/core-types/src/order_key.rs` (`between`/`from_bytes` 구현, `between_bytes` helper, 구현 검증 테스트 + 100만 회 property test)
- `docs/task-reports/III.5.2.md`(신규), 계획서(III.5.2 `[ ]`→`[x]`)

## 구현 요약

- ADR-0003에 결정론적 fractional indexing을 확정(사용자 승인). 키는 base-256 소수 byte string,
  사전식 비교 = 시각 순서, 정규형은 비어 있지 않고 `0x00` 미종료.
- `between(left, right)`: 좌/우 이웃 소수 사이의 **결정론적 중간값**을 생성(난수 미사용).
  `None`은 하한(0.0)/상한(1.0). 자릿수를 앞에서부터 비교해 간격(≥2)이 있으면 중간 자릿수로
  종료, 간격 1이면 좌측 자릿수를 따라 내려가며 상한을 1.0으로 풀어 항상 0이 아닌 자릿수로
  종료(정규형 유지).
- `from_bytes`: 빈 키(`Empty`)·후행 0(`TrailingZero`)을 거부.
- 동시 브랜치가 만든 동일 키는 상위 정렬에서 `StableId`로 tie-break(III.5.1 fixture).
- III.5.1의 7개 red fixture가 모두 green으로 전환.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-core-types` (104 unit + 8 doctest) | 0 |
| `cargo fmt -p stellar-core-types -- --check` | 0 |
| `cargo clippy -p stellar-core-types --all-targets -- -D warnings` | 0 |
| `cargo run -p xtask -- validate-adr crates/core-types/src/order_key.rs docs/adr/ADR-0003-order-key.md` | 0(확인) |

| 검사 | 결과 |
|---|---|
| III.5.1 요구사항 fixture(R1~R5) 7개 green | 통과 |
| 생성 키 정규형(비어 있지 않음, 후행 0 없음) | 통과 |
| 인접 byte 차이 1 이웃·동일 prefix 사이 삽입(자릿수 확장) | 통과 |
| 같은 끝 1000회 반복 삽입 단조 증가 | 통과 |
| **100만 회 임의 삽입: 중복·순서 역전 없음**(BTreeSet 1,000,000개) | 통과(~4.5s) |

## 차이(작업 지시와 다른 점)

- "승인된 알고리즘"의 승인은 사용자 결정(추천안 채택)으로 확보하고 ADR-0003에 기록했습니다.
  이 ADR이 같은 PR 범위의 core-types 보호 경로 변경(III.3.5~III.5.x 포함)을 ADR 정책상
  커버합니다.
- property test의 "임의" 삽입 위치는 테스트 전용 결정론적 xorshift64로 선택합니다(키 생성
  자체는 난수를 쓰지 않음 — 위치 선택과 키 생성을 분리). 시드 고정이라 재현 가능합니다.

## 남은 위험 / 한계

- `between`은 `left < right`(존재 시)를 가정합니다. 호출자가 실제 이웃을 넘기는 한 성립하며,
  property test가 이를 검증합니다.
- 키 길이는 반복 삽입으로 자랄 수 있습니다. 길이 회복(재균형)은 III.5.3에서 구현합니다
  (`needs_rebalance` 신호는 이미 제공).
