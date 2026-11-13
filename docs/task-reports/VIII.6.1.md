# 작업 보고서: VIII.6.1 ValueBinding 정의

- WBS: VIII.6.1
- 작업명: `ValueBinding`에 target PropertyPath, source expression root, expected type, scope owner 정의
- 상태: 완료 (`[x]`)
- 선행: III.2.2(`[x]`), VIII.2.5(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/bindings/src/binding.rs`(신규) — `BindingId`, `ScopeOwner`, `ValueBinding`, `assignable`, `check_source`,
  `BindingTypeError`, `BindingCodecError`, canonical binary 형식(`BINDING_FORMAT_VERSION = 1`)과 테스트.
- `crates/bindings/src/lib.rs` — `binding` 모듈 등록.
- `docs/task-reports/VIII.6.1.md`(신규), 계획서(VIII.6.1 `[ ]`→`[x]`).

## 구현 요약

- **필드**: `id`(BindingId = StableId), `target`(PropertyPath), `source`(ExpressionAst; `source_root()`가 root node),
  `expected_type`(TypeSpec), `scope_owner`(`Node(NodeRef)` | `Component(ComponentRef)`).
- **type validation**: source를 VIII.2.4 type checker로 검사하고 결과 타입이 target 타입에 대입 가능해야 합니다
  (`Required(T)` ← `Required(T)`; `Optional(T)` ← `Required(T)`·`Optional(T)`). 표현식 오류는 오류 node StableId를 포함합니다.
- **round-trip 형식**(모듈 문서에 고정): `version:u16 ++ id[16] ++ blob(PropertyPath canonical) ++ type_spec ++ owner ++
  blob(canonical AST)`, V.1.1 규칙(LE, `u32` LE 길이 프리픽스). 디코더는 잘림·잉여·미지 tag를 거부하고, 길이
  프리픽스가 남은 입력보다 길면 할당 전에 거부하며, 다시 인코딩한 bytes가 같을 때만 받아들입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 종료 코드 0 (22 unit passed, 0 failed) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **round-trip**(`bindings_round_trip_through_canonical_bytes`): owner 2종, PropertyPath key·index·한글 key segment tail, Required·Optional·Enum(`u32::MAX`) 타입, match/공유 node source → bytes → 동일 binding, 재직렬화 bytes 동일 | 통과 |
| 잘림(모든 길이)·잉여·버전·nullable flag·type tag·owner tag·PropertyPath 손상·`u32::MAX` 길이 프리픽스·source 손상 거부(`malformed_bytes_are_rejected`) | 통과 |
| **type validation**(`source_type_must_be_assignable_to_target`): 일치 통과, Optional 속성 ← Required 결과 통과, 다른 타입·nullable 결과 → Required 속성 거부, 모르는 참조 표현식 오류 | 통과 |

## 차이(작업 지시와 다른 점)

- 신규 모듈이므로 테스트는 구현과 함께 작성했습니다(구현 전 상태는 컴파일 불가).
- 행의 "round-trip"을 위해 binding binary 형식(버전 1)을 새로 정의했습니다. 이는 저장 형식 성격의 결정이며
  `crates/bindings`는 ADR 보호 경로가 아니지만, 형식 결정 기록(ADR)은 이 lane의 허용 경로(`docs/adr/` 제외) 밖이라
  작성하지 않았습니다. 후속으로 ADR 기록이 필요합니다.

## 남은 위험 / 한계

- `expected_type`이 실제 target 속성의 선언 타입과 같은지는 속성 metadata registry(document crate, 허용 edge 밖)가
  필요해 binding 단독으로는 확인하지 않습니다. 호출자가 선언 타입을 넣어야 합니다.
- PropertyPath decoder는 expressions crate의 비공개 decoder와 같은 규칙을 이 crate에 다시 구현했습니다(공용화는 후속).
