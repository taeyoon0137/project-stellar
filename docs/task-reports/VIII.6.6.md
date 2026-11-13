# 작업 보고서: VIII.6.6 binding command

- WBS: VIII.6.6
- 작업명: binding 생성·교체·삭제 command, type mismatch는 command validate에서 거부
- 상태: 완료 (`[x]`)
- 선행: VIII.6.1(`[x]`, 이 lane), Phase VII(phase 단위 선행 — 완화 적용, 아래 "차이" 참고)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/bindings/src/binding_commands.rs`(신규) — `BindingCommand`(Create·Replace·Delete), `BindingContext`,
  `BindingCommandError`와 테스트.
- `crates/bindings/src/lib.rs` — `binding_commands` 모듈 등록.
- `docs/task-reports/VIII.6.6.md`(신규), 계획서(VIII.6.6 `[ ]`→`[x]`).

## 구현 요약

- **command**: 닫힌 enum. `validate`는 모음을 바꾸지 않고, `apply`는 검증 후 적용하며 inverse를 돌려줍니다
  (Create ↔ Delete, Replace ↔ Replace(이전 source·기대 타입)). 실패 시 모음 불변.
- **validate 규칙**: ① 구조(ID·target 중복, 없는 binding; Replace는 target·owner 유지), ② cycle(자기 참조 포함, 통합
  graph `cycle_if_dependencies`), ③ type(source 결과 → 기대 타입 대입 가능, VIII.6.1). type mismatch는 적용 전
  `validate`에서 거부합니다.
- **참조 타입**: 토큰 = registry 토큰 타입(tombstone 포함), binding 있는 속성 = 그 binding의 기대 타입, 나머지 =
  호출자 제공자(`BindingContext::others`).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 첫 실행 컴파일 오류 1(테스트 기대값 타입) → 수정; 다음 실행 1 실패(자기 참조가 type 오류로 먼저 보고됨) → 검사 순서를 cycle → type으로 바꿔 종료 코드 0 (37 unit passed) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **apply/inverse**(`create_replace_delete_round_trip`): Create(속성→속성), Replace(source·nullable 기대 타입 변경), Delete 각각 forward → inverse = 원래 모음, redo = forward 모음 | 통과 |
| **invalid type fixture**(`type_mismatch_is_rejected_in_validate`): Color 속성 ← Number source가 `validate`·`apply` 모두 `Mismatch`, Required 속성 ← null source 교체 거부, 모르는 참조 표현식 오류, 실패 시 불변 | 통과 |
| cycle(`[p1, p2, p1]`)·자기 참조·같은 target·같은 ID·없는 binding 거부, 모든 실패에서 불변(`structure_and_cycle_violations_are_rejected`) | 통과 |

## 차이(작업 지시와 다른 점)

- **Phase VII 선행 완화**: VIII.5.5와 같이 Phase VII command 기반(VII.1.x, base 전부 `[ ]`)이 없고 `stellar-commands`
  의존이 허용되지 않아 binding 전용 closed enum command로 구현했습니다. Phase VII 구현 시 그 command enum으로 옮기는
  재작업이 필요합니다.
- 신규 모듈이므로 테스트는 구현과 함께 작성했습니다(구현 전 상태는 컴파일 불가).

## 남은 위험 / 한계

- 기대 타입이 실제 속성 선언 타입과 같은지는 속성 metadata(document crate)가 없어 확인하지 않습니다.
- 검증마다 통합 graph를 새로 만듭니다(O(토큰 + binding)).
