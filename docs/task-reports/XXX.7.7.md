# 작업 보고서: XXX.7.7 Context provenance

- WBS: XXX.7.7
- 작업명: evaluated value provenance chain에 ContextKey와 provision node StableId, default 해석·slot 경유 표시.
- 상태: 완료 (`[x]` 후보)
- 결정: ADR-0099 결정 7
- 선행: XXX.7.4, VIII.6.5

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `crates/bindings/src/context.rs`: `context_provenance`, `ContextHop`, `ContextProvenance`(XXX.7.3 commit).
- `crates/bindings/src/provenance.rs`: 읽은 node를 producer 경로표로 해석(XXX.7.3 commit).
- `tests/integration/tests/context_provenance.rs`(신규).

## 구현 요약

- VIII.6.5 chain의 각 단계에서 실제로 읽은 `Context(K)` 참조마다 key, 해석 node, 출처(provision StableId·provider node·인스턴스
  chain 또는 default), Slot 공개 목록 경유를 기록합니다. `providers()`는 최종값 → provision → … 순서의 provider node입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-integration-tests --test context_provenance` | 통과 |
| `cargo test -p stellar-bindings --lib context` | 통과 |

시험:

- `context_provenance::provenance_chain_lists_provisions_from_final_value_to_key`: Card root provision → View root provision chain,
  default 해석 표시, slot 경유 표시.
- `context::tests::provenance_records_key_and_provision_chain`(bindings).

## 차이(작업 지시와 다른 점)

- `ProvenanceStep`을 바꾸지 않고 chain 옆에 Context 단계를 붙였습니다(기존 provenance 소비자 영향 없음).

## 남은 위험 / 한계

- 인스펙터 표시(XXX.7.12)는 UI 재설계 때입니다.
