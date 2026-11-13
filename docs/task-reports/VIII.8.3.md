# 작업 보고서: VIII.8.3 손상된 기본값의 안전값과 corruption diagnostic

- WBS: VIII.8.3
- 작업명: 기본값이 없거나 잘못된 파일은 타입별 안전값과 corruption diagnostic을 사용하되 프로젝트 파일을 자동 수정하지 않음
- 상태: 완료 (`[x]`)
- 선행: VIII.8.2(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/bindings/src/fallback.rs` — `safe_value`, `safe_literal`, `StoredDefault`·`StoredDefaults`, `DefaultCorruption`,
  `CorruptionDiagnostic`, `ResolvedDefaults`, `resolve_stored_defaults`, `BindingEvaluator::open_with_stored_defaults`,
  모듈 문서의 안전값 표와 테스트 2개.
- `docs/task-reports/VIII.8.3.md`(신규), 계획서(VIII.8.3 `[ ]`→`[x]`).

## 구현 요약

- **안전값**(모듈 문서 표에 고정): `Optional<T>` → null, Boolean false, 수치·단위 0, String `""`, Enum variant 0, Color
  sRGB 투명, Gradient stop 없음, Shadow 0·투명, Typography 빈 payload, ResourceRef NIL. 13개 타입 모두 정의합니다.
- **해석**(`resolve_stored_defaults`): live binding이 없는 orphan target마다 저장된 기본값(선언 타입 + 저장된 canonical
  AST bytes)을 ① 없음 `Missing`, ② bytes 디코드 실패 `Malformed`(VIII.2.5 decoder 오류), ③ 선언 타입 불일치·type 오류
  `InvalidType`으로 판정해 해당 타입의 안전값 literal로 대체하고 `CorruptionDiagnostic`(binding, target, 원인, 안전값
  타입·값)을 orphan ID 순으로 남깁니다. 정상 기본값은 VIII.8.2 경로 그대로 평가합니다.
- **열기**(`open_with_stored_defaults`): 손상이 있어도 열기는 실패하지 않고 `(evaluator, diagnostics)`를 돌려줍니다.
  입력(저장소·저장된 기본값)은 읽기 전용 참조로만 받아 수정할 수 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 종료 코드 0 (45 unit passed, 0 failed) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **malformed fixture open 가능·오류 표시**(`malformed_default_fixture_opens_with_safe_values_and_diagnostics`): orphan 5개(정상·없음·잘린 bytes·Color를 Length에·version 손상 Optional)로 열기 성공, 값 4px·0px·0px·0px·null, 진단 4건(`missing`, `malformed`, `invalid-type`, `malformed`) binding 순·안전값 기록, 입력 저장소·기본값 bytes 불변, 재열기 시 같은 값·진단 | 통과 |
| 13개 타입 안전값이 그 타입과 일치, Required/Optional literal 구성(`safe_values_cover_every_type`) | 통과 |

## 차이(작업 지시와 다른 점)

- "파일"은 Component Property 저장 형식(Phase IX)이 아직 없어, 저장된 기본값을 "선언 타입 + 저장된 canonical AST
  bytes"(VIII.2.5 형식)로 받는 `StoredDefaults` 제공자로 모델링했습니다. 새 저장 형식은 만들지 않았습니다.

## 남은 위험 / 한계

- 정상으로 해석된 기본값의 **실행 시** 평가 오류(예: 0으로 나누기)는 안전값으로 바꾸지 않고 node 오류 값으로 남습니다.
- 진단의 UI 표시(프로젝트 전체 진단 목록)는 로드맵(§14.4)이며, 이 작업은 진단 데이터만 만듭니다.
