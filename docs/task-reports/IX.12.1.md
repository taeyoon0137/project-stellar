# 작업 보고서: IX.12.1 Component Context 저장 번호 예약

- WBS: IX.12.1
- 작업명: Component Context 저장에 쓸 numeric 번호와 확장 위치를 ADR로 예약하고 registry 예약 상수로 고정
- 상태: 완료 (`[x]`)
- 결정: ADR-0075 결정 8

## 필수 실행 환경

- `DEV-MAC`(Rust 단위 테스트).

## 변경 파일 목록

- `crates/core-types/src/reserved.rs`(신규) — `reserved::context` 예약 상수: `ReferenceTarget` Context tag `0x05`,
  chunk kind `0x8000_0007`(`context-set`, required), record kind `ContextKey` `0x01`·`ContextProvision` `0x02`,
  ComponentDefinition 확장 section `ContextRequirement` 목록 tag `0x01`, Slot 확장 section 공개 Context key tag
  `0x01`, semantic snapshot key `0xE705`·`0xE716`과 대역 `0xECxx`. 겹침 golden test. `crates/core-types/src/lib.rs` 등록.
- `crates/document/src/css/mod.rs` — 실제 `ReferenceTarget` 모든 variant의 tag와 예약 tag 비교 test(document가
  expressions에 의존하므로 여기서 확인).
- `crates/semantic-diff/src/keys.rs` — 예약 snapshot key·대역이 지금 key와 겹치지 않는 test.
- `docs/adr/ADR-0075-css-property-registry.md` — 결정 8 예약 표와 chunk kind 선택 근거.

## 구현 요약

- 번호만 고정했고 encoder·decoder·chunk kind registry 동작은 바꾸지 않았습니다(`crates/expressions`,
  `crates/storage`, `crates/document/src/component` 미변경). Context lane은 이 상수를 읽어 같은 번호로 구현합니다.
- `crates/core-types/`는 보호 경로이며 같은 diff 범위에 ADR-0075가 있습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-core-types --lib reserved` | 통과 |
| `cargo test -p stellar-document --lib reserved` | 1 passed |
| `cargo test -p stellar-semantic-diff --lib reserved` | 통과 |

| 검사 | 결과 |
|---|---|
| ADR 존재(ADR-0075 결정 8 표) | 확인 |
| 예약 번호가 기존 번호와 겹치지 않는 golden(ReferenceTarget 실제 tag, chunk kind 1\~6, snapshot key) | 통과 |
| 기존 golden bytes 불변 | encoder·decoder 변경 없음. storage·expressions golden test는 lane 최종 check-core에서 실행 |
| `cargo xtask validate-adr` | lane 최종 검증에서 실행(아래 lane 결과) |

## 남은 위험 / 한계

- chunk kind 1\~6 비교는 storage golden 값을 core-types test에 복사한 목록입니다. core-types가 storage에 의존할 수
  없어서입니다. storage `kinds.rs`에 kind를 더할 때 이 예약(7)을 피해야 합니다(예약 상수 문서 주석에 기록).
