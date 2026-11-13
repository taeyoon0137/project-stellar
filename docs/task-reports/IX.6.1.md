# 작업 보고서: IX.6.1 SemanticContract·InteractionContract

- WBS: IX.6.1
- 작업명: Control을 node kind가 아닌 Frame에 부착하는 `SemanticContract`와 `InteractionContract`로 정의
- 상태: 완료 (`[x]`)
- 선행: VI.3.1(`[x]`), IX.5.2(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/control/mod.rs`(신규) — `ControlContract`, `SemanticContract`, `InteractionContract`,
  `ContractKind`/`ContractField`/`ContractValue`/`ValueKind`, `ControlContractTable`(부착·분리·codec),
  `ContractScope`(+ `ComponentEntry` 구현), 검사 결과 타입 `ContractIssue`/`ContractProblem`/`Severity`.
- `crates/document/src/lib.rs` — `pub mod control;` 등록.
- `crates/document/src/component/mod.rs` — canonical codec 도우미 모듈을 crate 내부 공용(`pub(crate)`)으로 노출.
- `docs/task-reports/IX.6.1.md`(신규), 계획서(IX.6.1 `[ ]`→`[x]`).

## 구현 요약

- **Frame 부착**: `ControlContractTable::attach`는 대상이 Frame일 때만 받습니다(node 종류에 Control을 추가하지
  않음). 한 Frame에 계약 하나입니다.
- **분리 저장**: 계약은 Frame payload와 별도 표(frame StableId → 계약)에 있습니다. `detach`는 표 항목만 지우므로
  Frame의 종류·payload는 그대로이고 일반 Frame으로 남습니다.
- **SemanticContract**: numeric 종류(`ContractKind`) + numeric field(`ContractField`) → 값(`ContractValue`) 희소 map.
  값은 상수, 컴포넌트 Property, 상태 변수, semantic action, node, 이벤트 종류입니다. 기술 아키텍처 12.5대로
  계약 종류는 파일 스키마의 닫힌 enum이 아니므로, 모르는 종류·field도 디코딩 후 그대로 보존합니다.
- **InteractionContract**: focus 가능 여부와 처리하는 이벤트 종류 집합(IX.5.2 `EventKind`).
- **codec**: frame id·field id·이벤트 tag 오름차순 canonical bytes. 정렬 위반·중복·모르는 값/이벤트 tag·trailing
  bytes는 거부합니다.
- **검사 기반**: 종류별 검사(IX.6.2~IX.6.6)가 쓸 `ContractScope`(Property·상태 변수 타입, action 선언, node 종류·
  포함 관계)와 결과 타입(`ContractIssue`, 심각도 Error/Diagnostic)을 정의했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 286 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| Text·없는 node 부착 거부, Frame 부착, 중복 부착 거부, 분리 후 표 비움·Frame 종류·payload bytes 불변(`contract_attaches_only_to_frames_and_removal_keeps_plain_frame`) | 통과 |
| 모르는 종류·field 보존 round-trip byte-identical, 절단·trailing·모르는 값 tag 거부(`table_round_trips_unknown_kinds_and_rejects_malformed_bytes`) | 통과 |

## 차이(작업 지시와 다른 점)

- 계약 종류를 닫힌 Rust enum이 아니라 numeric 종류 + field map으로 정의했습니다(12.5의 "폐쇄형 enum 아님").
  알려진 종류의 의미 규칙은 종류 모듈과 registry(IX.6.7)가 정합니다.

## 남은 위험 / 한계

- 표를 문서 저장 chunk(VI.10)에 싣는 연결은 component library chunk와 함께 project 계층에서 합니다.
