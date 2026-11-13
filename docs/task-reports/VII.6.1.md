# 작업 보고서: VII.6.1 command 왕복 property test macro

- WBS: VII.6.1
- 작업명: 각 command에 `apply(forward); apply(inverse)`가 원본 semantic snapshot으로 돌아오는 property test macro
- 상태: 완료 (`[x]`)
- 선행: VII.2, VII.3(section 단위 선행 — 이 lane에서 VII.2.1~VII.2.8, VII.3.1~VII.3.4 모두 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/commands/src/test_support.rs`(신규, `#[doc(hidden)]` 테스트 전용) — `FreshIds`, 종류별 command 생성기 `generate`
  (wildcard 없는 match), `random_valid_request`. PRNG는 document test_support의 SplitMix64 재사용.
- `crates/commands/src/property_tests.rs`(신규, `cfg(test)`) — `round_trip_property!` macro, 종류별 왕복 test 13개, 등록 검사.
- `crates/commands/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VII.6.1.md`(신규), 계획서(VII.6.1 `[ ]`→`[x]`).

## 구현 요약

- **macro**: `round_trip_property! { name => 대상, min N; ... }`가 대상마다 `#[test]`를 만들고 등록 목록 상수(`REGISTERED`)를 남깁니다.
- **문서 command 왕복**: seed 0..160의 random valid document(VI.10.5)에 그 종류의 후보를 만들어 `validate`를 통과하면 실행 → 전체 문서
  validator(VI.10.1) 통과 확인 → 역 command 실행 → semantic snapshot(VI.10.4)이 원본과 같은지 검사합니다. 종류별 최소 적용 횟수를 둬
  생성기가 비어 vacuous하게 통과하는 것을 막습니다.
- **선택 event 왕복**(VII.3): 무작위 선택(숨김 정책 무작위)에서 `SetSelection` 적용 후 역 적용 → view 상태 동일.
- **등록 검사**: `REGISTERED` = `CommandKind::ALL`(12종) + 선택 event, 중복 없음. 새 종류를 추가하면 생성기 match(wildcard 없음)가 컴파일
  단계에서, 등록 검사가 테스트 단계에서 누락을 잡습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-commands` | 완료 |
| `cargo test -p stellar-commands` | unit 76 passed, doctest 3 passed, 0 failed(5.12s) |
| `cargo clippy -p stellar-commands --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-commands -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| InsertNode·DeleteSubtree·MoveNode·SetProperty·InsertModifier·RemoveModifier·MoveModifier·SetModifierProperty·ConvertFragmentToFrame·ConvertShapeToPath·RestoreNodeRecords·DuplicateSubtree 각 왕복(forward 후 validator 통과, inverse 후 snapshot 동일, 종류별 최소 적용 횟수 충족) | 12건 통과 |
| SetSelection forward/inverse 왕복(`set_selection_round_trip`) | 통과 |
| 등록 목록 = `CommandKind::ALL` + 선택 event(`every_phase_vii_command_is_registered`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 `VII.2`, `VII.3`은 section 단위 선행이며, 이 lane에서 해당 행이 모두 `[x]`입니다(완화 불필요).
- 생성기는 history crate(VII.6.2)도 쓰도록 commands의 숨김 공개 모듈(`test_support`)에 두었습니다(document crate와 같은 관례).

## 남은 위험 / 한계

- ConvertFragmentToFrame은 random 문서에서 위치 보존이 가능한 Fragment가 적어 최소 적용 횟수를 10으로 낮게 두었습니다.
