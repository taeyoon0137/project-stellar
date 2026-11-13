# 작업 보고서: VIII.8.2 orphan binding 선언 기본값 평가

- WBS: VIII.8.2
- 작업명: orphaned binding 평가 결과는 대상 Component Property의 선언된 기본값 사용, 마지막 계산값 저장 금지
- 상태: 완료 (`[x]`)
- 선행: VIII.8.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/bindings/src/fallback.rs`(신규) — `DeclaredDefault`, `DeclaredDefaults`, `orphan_default_spec`, `store_specs`,
  `BindingEvaluator::with_orphans`·`with_orphans_and_limits`와 테스트.
- `crates/bindings/src/scheduler.rs` — 생성자를 producer 명세(`ProducerSpec`) 기반 `from_specs`로 일반화(`with_limits`는
  명세를 만들어 위임, 동작 동일), `token_specs`, 정의 없는 토큰 타입 처리(`TokenAwareSources`), `SchedulerError::Graph`.
- `crates/bindings/src/graph.rs` — `Producer::OrphanDefault(BindingId)`.
- `crates/bindings/src/orphan.rs` — `BindingStore::insert_live`(orphan과 같은 ID 거부).
- `crates/bindings/src/lib.rs` — `fallback` 모듈 등록.
- `docs/task-reports/VIII.8.2.md`(신규), 계획서(VIII.8.2 `[ ]`→`[x]`).

(프로덕션 파일 5개 — 규칙 1.2.13 한도 이내)

## 구현 요약

- **기본값 producer**: orphan target 속성 node의 값은 그 속성의 선언 기본값 표현식(`DeclaredDefault`: 현재 선언 타입 +
  표현식)이 만듭니다(`Producer::OrphanDefault`). scheduler가 다른 producer와 같이 위상 순서·증분 재평가·provenance로
  다룹니다. 기본값이 토큰을 참조하면 토큰 변경 시 함께 재평가됩니다.
- **마지막 계산값 금지**: `OrphanedBinding`에는 값 필드가 없고, 평가할 때마다 선언 기본값을 다시 평가합니다.
- **우선순위**: 같은 target에 live binding이 다시 생기면 live binding이 값을 만들고 기본값 producer는 만들지 않습니다.
- **리팩터링**: scheduler 생성 경로를 명세 기반으로 바꿨습니다. 기존 `new`·`with_limits`는 같은 명세를 만들어 위임하며
  기존 테스트 41개가 그대로 통과했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 리팩터링 직후 41 passed(동작 보존 확인) → 기능 추가 후 종료 코드 0 (43 unit passed) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **재실행 후 동일 기본값**(`orphan_target_uses_declared_default_not_last_value`): live 상태 p1 = 11px(마지막 계산값) → 토큰 삭제로 orphan → p1 = 선언 기본값 4px, dependent p2 = 8px; 새 evaluator로 재실행해도 4px·8px; producer·provenance가 `OrphanDefault` | 통과 |
| 기본값 표현식이 토큰을 참조(t2 × 2 = 6px), 토큰 변경 시 t2·p1·p2 증분 재평가, live binding 재생성 시 live 우선, orphan ID와 같은 live 추가 거부(`default_expressions_are_evaluated_and_live_binding_wins`) | 통과 |

## 차이(작업 지시와 다른 점)

- 신규 모듈이므로 테스트는 구현과 함께 작성했습니다(구현 전 상태는 컴파일 불가).
- 선언 기본값은 Component Property 모델(IX.1.2)이 없어 `DeclaredDefaults` 제공자로 받습니다.

## 남은 위험 / 한계

- 선언 기본값이 없거나 잘못된 경우의 타입별 안전값·진단은 VIII.8.3에서 다룹니다(이 단계에서는 source 값 없음 오류).
