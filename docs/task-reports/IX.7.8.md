# 작업 보고서: IX.7.8 View 변수·Variant command

- WBS: IX.7.8
- 작업명: View variable/variant 생성·변경·삭제 command, 삭제 시 참조 variant는 orphan diagnostic으로 남김
- 상태: 완료 (`[x]`)
- 선행: IX.7.1~IX.7.7(IX.7.3은 기존 `[x]`, 나머지는 이 lane에서 `[x]`), Phase VII(phase 단위 선행 — 완화 적용)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/view_variant/commands.rs`(신규) — `ViewVariantModel`(변수 모음 + Variant, codec, 진단),
  `ViewVariantCommand`(Create/Replace/Delete Variable·Variant, inverse 전용 RestoreVariant), `VariantDiagnostic`,
  `VariantCommandError`.
- `crates/document/src/view_variant/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.7.8.md`(신규), 계획서(IX.7.8 `[ ]`→`[x]`).

## 구현 요약

- **command**(closed enum, 검증 후 적용·실패 시 model 불변, inverse 반환):
  - 변수: `CreateVariable` ↔ `DeleteVariable`, `ReplaceVariable` ↔ 이전 값 `ReplaceVariable`.
  - Variant: `CreateVariant` ↔ `DeleteVariant`, `ReplaceVariant`·`DeleteVariant` → `RestoreVariant(이전)`.
  - 생성·교체는 조건을 현재 변수에 대해 엄격히 검사합니다(없는 변수·허용 밖 값 거부). `RestoreVariant`는 inverse
    전용이라 검사 없이 그대로 되살려 orphan 조건 Variant의 undo도 정확합니다.
- **orphan**: 변수 삭제는 참조 Variant를 지우지 않습니다. `diagnostics()`가 `OrphanCondition { variant, variable }`을,
  변수 타입·허용 값 변경으로 조건 값이 무효가 되면 `InvalidConditionValue`를 냅니다. orphan term은 만족되지 않으므로
  그 Variant는 적용되지 않습니다(IX.7.2).
- **저장**: model은 변수 모음 bytes + Variant(id 오름차순: 조건, IX.7.3 override bytes) canonical bytes이며 orphan
  조건도 보존합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 316 passed, 통합 7+1+1+2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| command 8개(변수 생성·교체·삭제, Variant 생성·교체·삭제) 적용 → inverse 역순 적용 시 각 단계 bytes 정확 복원, redo 재현, 최초 bytes 복귀, model round-trip(`commands_apply_and_inverse_exactly`) | 통과 |
| 참조 변수 삭제 → Variant 2개 유지·`OrphanCondition` 2건, orphan model round-trip, orphan Variant 삭제 undo 복원, 변수 삭제 undo 후 진단 0건(`deleting_a_referenced_variable_leaves_orphan_diagnostics`) | 통과 |
| 없는 변수 조건·허용 밖 값·중복·없는 대상 command 거부와 model 불변, 변수 타입 변경 → `InvalidConditionValue`(`invalid_commands_leave_model_unchanged`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 `Phase VII`은 phase 단위 선행이라 완화했습니다. `crates/commands`는 다른 lane이 구현 중이라 document 계층
  closed enum + apply(inverse 반환)로 구현했습니다. command 계층 등록·history 연동은 후속입니다.
- inverse 전용 `RestoreVariant` variant를 추가했습니다(orphan 조건 Variant의 정확한 undo).

## 남은 위험 / 한계

- Variant override의 scene 참조·캡슐화 검사는 base scene이 필요해 command가 아니라 IX.7.3·IX.7.7 API가 합니다.
  command 계층 연결 시 두 검사를 validate 단계에 넣어야 합니다.
