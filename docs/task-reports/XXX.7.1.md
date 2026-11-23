# 작업 보고서: XXX.7.1 `ContextKey` resource

- WBS: XXX.7.1
- 작업명: `ContextKey` resource(StableId, 이름, TypeSpec, 필수 닫힌 default, 프로젝트·라이브러리·문서 scope)와 저장 형식.
- 상태: 완료 (`[x]` 후보) — 행 범위 구현·검증 기준을 채웠습니다. marker는 통합 때 정합니다.
- 결정: ADR-0099 결정 1·2, ADR-0025 결정 3.1, ADR-0075 결정 8(IX.12.1 예약 번호)
- 선행: IX.12.1, IX.1.2, VIII.5.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/context/{mod,key}.rs`(신규): `ContextScope`, `ContextKey`, canonical bytes, TypeSpec bytes 함수.
- `crates/document/src/lib.rs`: `context` module 등록, `stellar_expressions` 재수출(`stellar_document::expressions`).
- `docs/adr/ADR-0099-component-context-core.md`(신규).

## 구현 요약

- default는 Component Property default와 같은 `CheckedExpression::closed` 검사를 거칩니다. 참조(다른 key·token)가 있으면
  열린 식으로 거부하고, root 타입이 선언 타입에 맞지 않으면 거부합니다. 생성자가 default를 요구하고, 디코더는 default 자리가
  빈 bytes면 `MissingDefault`로 거부합니다.
- 참조는 StableId(`ReferenceTarget::Context(key)`)라 rename은 소비·provision 참조를 바꾸지 않습니다.
- record bytes는 key 하나 단위이고, 문서 단위 모음은 XXX.7.2의 `ContextSet`이 IX.12.1 record kind `0x01`로 담습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document --lib context::` | 통과(14개, 이 행은 `context::key::tests` 2개) |
| `cargo test -p stellar-project --lib source_v4::context` | 통과(텍스트 원본 key round-trip 포함) |

시험:

- `context::key::tests::key_requires_closed_default_of_declared_type`: default 누락·타입 불일치·열린 식(token, 다른 key)·NIL·이름 공백 거부.
- `context::key::tests::key_round_trips_and_rename_keeps_identity`: 세 scope round-trip, 꼬리 bytes 거부, rename 후 StableId 참조 유지.

## 차이(작업 지시와 다른 점)

- 없음. 저장 형식 세부는 ADR-0099 결정 2입니다.

## 남은 위험 / 한계

- 프로젝트·라이브러리 scope key의 저장 위치(프로젝트 공통 set, 라이브러리 패키지)는 정하지 않았습니다. 해석·진단은
  `ContextKeys`로 여러 set을 겹쳐 받습니다(XXX.7.13 fixture의 외부 라이브러리 key).
